use uefi::boot::{AllocateType, MemoryType};

use crate::PAGE_SIZE;
use crate::serial::serial_str;

// Sv39 PTE flags

const PTE_V: u64 = 1 << 0; // Valid
const PTE_R: u64 = 1 << 1; // Read
const PTE_W: u64 = 1 << 2; // Write
const PTE_X: u64 = 1 << 3; // Execute
const PTE_A: u64 = 1 << 6; // Accessed
const PTE_D: u64 = 1 << 7; // Dirty

/// Leaf PTE flags for kernel read-write-execute memory.
const LEAF_RWX: u64 = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D;

// Page-table layout constants

/// Root-table index for kernel mapping.
/// VPN\[2\] = 256 → VA 0xFFFF_FFC0_0000_0000 (Sv39 sign-extended).
pub const KERNEL_ROOT_INDEX: usize = 256;

/// Root-table index range start for the physical-memory direct mapping.
/// VPN\[2\] = 384 → VA 0xFFFF_FFE0_0000_0000.
pub const PHYS_MAP_ROOT_INDEX: usize = 384;

/// Virtual address offset where all physical memory is linearly mapped.
pub const PHYSICAL_MEMORY_OFFSET: u64 = 0xFFFF_FFE0_0000_0000;

/// SATP mode field value for Sv39 (placed in bits [63:60]).
pub const SATP_MODE_SV39: u64 = 8;

/// Holds allocated page-table pages for deferred initialization.
pub struct PageTableConfig {
    root: u64,
    l1_kernel: u64,
    l0_base: u64,
    kernel_phys: u64,
    kernel_4k_pages: usize,
    l0_count: usize,
}

impl PageTableConfig {
    pub fn root(&self) -> u64 {
        self.root
    }

    /// Construct the full SATP register value (Sv39, ASID = 0).
    pub fn satp_value(&self) -> u64 {
        let ppn = self.root >> 12;
        (SATP_MODE_SV39 << 60) | ppn
    }
}

/// Build a non-leaf (pointer) PTE: next-level table address encoded as PPN
/// with only the Valid bit set.
fn table_pte(table_phys: u64) -> u64 {
    ((table_phys >> 12) << 10) | PTE_V
}

/// Build a leaf PTE for a gigapage / megapage / 4 KiB page.
fn leaf_pte(phys: u64) -> u64 {
    ((phys >> 12) << 10) | LEAF_RWX
}

/// Allocate all page-table memory via UEFI boot services.
///
/// Must be called **before** `exit_boot_services`.
///
/// # Safety
/// Caller must ensure UEFI boot services are still available.
pub unsafe fn allocate_page_tables(kernel_phys: u64, kernel_size: usize) -> PageTableConfig {
    let kernel_4k_pages = (kernel_size + PAGE_SIZE - 1) / PAGE_SIZE;
    let l0_count = (kernel_4k_pages + 511) / 512;

    // root + L1_KERNEL + L0[n]
    let total_pages = 2 + l0_count;
    let pages_ptr = uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        total_pages,
    )
    .expect("Failed to allocate page tables");

    let base = pages_ptr.as_ptr() as u64;
    let mut off = 0u64;

    let root = base + off;
    off += PAGE_SIZE as u64;

    let l1_kernel = base + off;
    off += PAGE_SIZE as u64;

    let l0_base = base + off;

    PageTableConfig {
        root,
        l1_kernel,
        l0_base,
        kernel_phys,
        kernel_4k_pages,
        l0_count,
    }
}

/// Fill in all page-table entries (Sv39).
///
/// Must be called **after** `exit_boot_services`.
///
/// Returns the physical address of the root page table.  Use
/// [`PageTableConfig::satp_value`] for the full SATP register value.
///
/// # Memory map produced
///
/// | Virtual range | Physical | Level |
/// |---|---|---|
/// | 0 – 4 GiB identity | 0 – 4 GiB | 1 GiB gigapages (root) |
/// | `PHYSICAL_MEMORY_OFFSET` + 0 – 4 GiB | 0 – 4 GiB | 1 GiB gigapages (root) |
/// | Kernel at root\[256\] | `kernel_phys` … | 4 KiB pages (L1 → L0) |
///
/// # Safety
/// Caller must ensure boot services have been exited and the addresses in
/// `cfg` are still valid.
pub unsafe fn init_page_tables(cfg: &PageTableConfig) -> u64 {
    let total_pages = 2 + cfg.l0_count;

    serial_str("[PT] Initializing RISC-V Sv39 page tables...\r\n");

    unsafe {
        core::ptr::write_bytes(cfg.root as *mut u8, 0, PAGE_SIZE * total_pages);

        let root = cfg.root as *mut u64;

        // Identity mapping: first 4 GiB via 1 GiB gigapages

        for i in 0..4u64 {
            *root.add(i as usize) = leaf_pte(i << 30);
        }

        // Physical-memory direct mapping: 4 × 1 GiB gigapages

        for i in 0..4u64 {
            *root.add(PHYS_MAP_ROOT_INDEX + i as usize) = leaf_pte(i << 30);
        }

        // Kernel mapping: root[KERNEL] → L1 → L0 (4 KiB pages)

        *root.add(KERNEL_ROOT_INDEX) = table_pte(cfg.l1_kernel);

        let l1 = cfg.l1_kernel as *mut u64;
        for i in 0..cfg.l0_count {
            let l0_addr = cfg.l0_base + i as u64 * PAGE_SIZE as u64;
            *l1.add(i) = table_pte(l0_addr);
        }

        for i in 0..cfg.kernel_4k_pages {
            let l0_idx = i / 512;
            let pte_idx = i % 512;
            let l0 = (cfg.l0_base + l0_idx as u64 * PAGE_SIZE as u64) as *mut u64;
            let phys = cfg.kernel_phys + i as u64 * PAGE_SIZE as u64;
            *l0.add(pte_idx) = leaf_pte(phys);
        }
    }

    serial_str("[PT] RISC-V Sv39 page tables initialized\r\n");

    cfg.root
}
