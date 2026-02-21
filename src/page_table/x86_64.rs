use uefi::boot::{AllocateType, MemoryType};

use crate::PAGE_SIZE;
use crate::serial_str;

const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_HUGE: u64 = 1 << 7;

/// Default PML4 entry index for kernel virtual address mapping.
///
/// Note: the actual kernel PML4 index can be chosen at runtime (e.g. derived
/// from an ELF kernel's virtual base), and is stored in [`PageTableConfig`].
pub const DEFAULT_KERNEL_PML4_INDEX: usize = 510;

/// PML4 entry index for the physical memory direct mapping.
/// Index 256 → virtual base 0xFFFF_8000_0000_0000.
pub const PHYS_MAP_PML4_INDEX: usize = 256;

/// Virtual address offset where all physical memory is linearly mapped.
pub const PHYSICAL_MEMORY_OFFSET: u64 = 0xFFFF_8000_0000_0000;

/// Holds physical addresses of all allocated page-table pages and kernel
/// geometry so that [`init_page_tables`] can fill them in after UEFI boot
/// services have been exited.
pub struct PageTableConfig {
    pml4: u64,
    pdpt_low: u64,
    pdpt_kernel: u64,
    pdpt_phys_map: u64,
    pd_low_base: u64,
    pd_kernel: u64,
    pd_phys_map_base: u64,
    pt_base: u64,
    kernel_phys: u64,
    kernel_4k_pages: usize,
    pt_count: usize,
    kernel_pml4_index: usize,
}

impl PageTableConfig {
    pub fn root(&self) -> u64 {
        self.pml4
    }
}

/// Allocate all page-table memory via UEFI boot services.
///
/// Must be called **before** `exit_boot_services`.  The returned config is
/// later passed to [`init_page_tables`].
///
/// # Safety
/// Caller must ensure UEFI boot services are still available.
pub unsafe fn allocate_page_tables(
    kernel_phys: u64,
    kernel_size: usize,
    kernel_pml4_index: usize,
) -> PageTableConfig {
    let kernel_4k_pages = (kernel_size + PAGE_SIZE - 1) / PAGE_SIZE;
    let pt_count = (kernel_4k_pages + 511) / 512;

    // PML4 + PDPT_LOW + PDPT_KERNEL + PDPT_PHYS_MAP
    // + PD_LOW[4] + PD_KERNEL + PD_PHYS_MAP[4] + PT[n]
    let total_pages = 1 + 3 + 4 + 1 + 4 + pt_count;
    let pages_ptr = uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        total_pages,
    )
    .expect("Failed to allocate page tables");

    let base = pages_ptr.as_ptr() as u64;
    let mut off = 0u64;

    let pml4 = base + off;
    off += PAGE_SIZE as u64;

    let pdpt_low = base + off;
    off += PAGE_SIZE as u64;

    let pdpt_kernel = base + off;
    off += PAGE_SIZE as u64;

    let pdpt_phys_map = base + off;
    off += PAGE_SIZE as u64;

    let pd_low_base = base + off;
    off += 4 * PAGE_SIZE as u64;

    let pd_kernel = base + off;
    off += PAGE_SIZE as u64;

    let pd_phys_map_base = base + off;
    off += 4 * PAGE_SIZE as u64;

    let pt_base = base + off;

    PageTableConfig {
        pml4,
        pdpt_low,
        pdpt_kernel,
        pdpt_phys_map,
        pd_low_base,
        pd_kernel,
        pd_phys_map_base,
        pt_base,
        kernel_phys,
        kernel_4k_pages,
        pt_count,
        kernel_pml4_index,
    }
}

/// Fill in all page-table entries.
///
/// Must be called **after** `exit_boot_services` (no UEFI calls inside).
///
/// Returns the physical address of the PML4 table, suitable for loading
/// into CR3.
///
/// # Memory map produced
///
/// | Virtual range | Physical range | Granularity |
/// |---|---|---|
/// | 0 – 4 GiB (identity) | 0 – 4 GiB | 2 MiB huge pages |
/// | `PHYSICAL_MEMORY_OFFSET` + 0 – 4 GiB | 0 – 4 GiB | 2 MiB huge pages |
/// | Kernel at PML4\[510\] | `kernel_phys` … | 4 KiB pages |
///
/// # Safety
/// Caller must ensure boot services have been exited and the addresses in
/// `cfg` are still valid.
pub unsafe fn init_page_tables(cfg: &PageTableConfig) -> u64 {
    let pml4 = cfg.pml4 as *mut u64;
    let pdpt_low = cfg.pdpt_low as *mut u64;
    let pdpt_kernel = cfg.pdpt_kernel as *mut u64;
    let pdpt_phys_map = cfg.pdpt_phys_map as *mut u64;
    let pd_low_base = cfg.pd_low_base;
    let pd_kernel = cfg.pd_kernel as *mut u64;
    let pd_phys_map_base = cfg.pd_phys_map_base;
    let pt_base = cfg.pt_base;

    let total_pages = 1 + 3 + 4 + 1 + 4 + cfg.pt_count;

    serial_str("[PT] Initializing page tables...\r\n");

    unsafe {
        core::ptr::write_bytes(pml4 as *mut u8, 0, PAGE_SIZE * total_pages);

        // PML4[0] → PDPT_LOW  (identity mapping for first 4 GiB)
        *pml4.add(0) = cfg.pdpt_low | PAGE_PRESENT | PAGE_WRITABLE;

        // PML4[KERNEL] → PDPT_KERNEL
        *pml4.add(cfg.kernel_pml4_index) = cfg.pdpt_kernel | PAGE_PRESENT | PAGE_WRITABLE;

        // PML4[PHYS_MAP] → PDPT_PHYS_MAP
        *pml4.add(PHYS_MAP_PML4_INDEX) = cfg.pdpt_phys_map | PAGE_PRESENT | PAGE_WRITABLE;

        // PDPT_LOW[0..4] → PD_LOW[0..4]
        for i in 0..4usize {
            let pd_addr = pd_low_base + i as u64 * PAGE_SIZE as u64;
            *pdpt_low.add(i) = pd_addr | PAGE_PRESENT | PAGE_WRITABLE;
        }

        // PD_LOW: identity-map first 4 GiB with 2 MiB huge pages
        for gb in 0..4u64 {
            let pd = (pd_low_base + gb * PAGE_SIZE as u64) as *mut u64;
            for i in 0..512u64 {
                let phys = (gb * 512 + i) * 0x20_0000;
                *pd.add(i as usize) = phys | PAGE_PRESENT | PAGE_WRITABLE | PAGE_HUGE;
            }
        }

        // PDPT_PHYS_MAP[0..4] → PD_PHYS_MAP[0..4]
        for i in 0..4usize {
            let pd_addr = pd_phys_map_base + i as u64 * PAGE_SIZE as u64;
            *pdpt_phys_map.add(i) = pd_addr | PAGE_PRESENT | PAGE_WRITABLE;
        }

        // PD_PHYS_MAP: map first 4 GiB with 2 MiB huge pages
        for gb in 0..4u64 {
            let pd = (pd_phys_map_base + gb * PAGE_SIZE as u64) as *mut u64;
            for i in 0..512u64 {
                let phys = (gb * 512 + i) * 0x20_0000;
                *pd.add(i as usize) = phys | PAGE_PRESENT | PAGE_WRITABLE | PAGE_HUGE;
            }
        }

        // PDPT_KERNEL[0] → PD_KERNEL
        *pdpt_kernel.add(0) = cfg.pd_kernel | PAGE_PRESENT | PAGE_WRITABLE;

        // PD_KERNEL[0..n] → PT pages
        for i in 0..cfg.pt_count {
            let pt_addr = pt_base + i as u64 * PAGE_SIZE as u64;
            *pd_kernel.add(i) = pt_addr | PAGE_PRESENT | PAGE_WRITABLE;
        }

        // PT: map each 4 KiB kernel page
        for i in 0..cfg.kernel_4k_pages {
            let pt_idx = i / 512;
            let pte_idx = i % 512;
            let pt = (pt_base + pt_idx as u64 * PAGE_SIZE as u64) as *mut u64;
            let phys = cfg.kernel_phys + i as u64 * PAGE_SIZE as u64;
            *pt.add(pte_idx) = phys | PAGE_PRESENT | PAGE_WRITABLE;
        }
    }

    serial_str("[PT] Page tables initialized\r\n");

    cfg.pml4
}
