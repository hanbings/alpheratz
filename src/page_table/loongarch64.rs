use uefi::boot::{AllocateType, MemoryType};

use crate::PAGE_SIZE;
use crate::serial::serial_str;

// LoongArch PTE flags

const PTE_V: u64 = 1 << 0;  // Valid
const PTE_D: u64 = 1 << 1;  // Dirty
const PTE_PLV0: u64 = 0 << 2; // Privilege Level 0 (kernel)
const PTE_MAT_CC: u64 = 1 << 4; // Memory Access Type: Coherent Cached
const PTE_G: u64 = 1 << 6;  // Global

/// Combined leaf-PTE flags for kernel read-write cached memory.
const LEAF_ATTRS: u64 = PTE_V | PTE_D | PTE_PLV0 | PTE_MAT_CC | PTE_G;

// Direct Mapping Window values

/// DMW0: Uncached identity mapping.
/// VSEG = 0x8 → VA 0x8000_xxxx_xxxx_xxxx maps to PA 0x0000_xxxx_xxxx_xxxx.
/// MAT = 0 (Strongly-ordered UnCached), PLV0 enabled.
pub const DMW0_VALUE: u64 = (0x8 << 60) | (1 << 0);

/// DMW1: Cached identity mapping.
/// VSEG = 0x9 → VA 0x9000_xxxx_xxxx_xxxx maps to PA 0x0000_xxxx_xxxx_xxxx.
/// MAT = 1 (Coherent Cached), PLV0 enabled.
pub const DMW1_VALUE: u64 = (0x9 << 60) | (1 << 4) | (1 << 0);

// Page-table layout constants

/// PGD index for the kernel virtual-address mapping.
/// For VA with bits [47:39] = 510 (if kernel is outside DMW range).
pub const KERNEL_PGD_INDEX: usize = 510;

/// Virtual address where physical memory is linearly mapped (via DMW1).
pub const PHYSICAL_MEMORY_OFFSET: u64 = 0x9000_0000_0000_0000;

/// CSR.PWCL value for the 4-level page walk configuration.
///
/// | Field | Base | Width | Meaning |
/// |-------|------|-------|---------|
/// | PT | 12 | 9 | PTE: VA\[20:12\], 512 entries |
/// | Dir1 | 21 | 9 | PMD: VA\[29:21\], 512 entries |
///
/// Encoding: `PTBase | (PTWidth << 5) | (Dir1Base << 10) | (Dir1Width << 15)`
pub const PWCL_VALUE: u64 = 12 | (9 << 5) | (21 << 10) | (9 << 15);

/// CSR.PWCH value for the upper page-walk levels.
///
/// | Field | Base | Width | Meaning |
/// |-------|------|-------|---------|
/// | Dir2 | 30 | 9 | PUD: VA\[38:30\], 512 entries |
/// | Dir3 | 39 | 9 | PGD: VA\[47:39\], 512 entries |
///
/// Encoding: `Dir2Base | (Dir2Width << 5) | (Dir3Base << 10) | (Dir3Width << 15)`
pub const PWCH_VALUE: u64 = 30 | (9 << 5) | (39 << 10) | (9 << 15);

/// Holds allocated page-table pages and DMW configuration.
pub struct PageTableConfig {
    pgd: u64,
    pud_kernel: u64,
    pmd_kernel: u64,
    pte_base: u64,
    kernel_phys: u64,
    kernel_4k_pages: usize,
    pte_count: usize,
}

impl PageTableConfig {
    /// Physical address of the Page Global Directory.
    pub fn pgd(&self) -> u64 {
        self.pgd
    }

    /// DMW0 register value (uncached mapping, VSEG = 0x8).
    pub fn dmw0(&self) -> u64 {
        DMW0_VALUE
    }

    /// DMW1 register value (cached mapping, VSEG = 0x9).
    pub fn dmw1(&self) -> u64 {
        DMW1_VALUE
    }
}

/// Allocate all page-table memory via UEFI boot services.
///
/// Must be called **before** `exit_boot_services`.
///
/// # Safety
/// Caller must ensure UEFI boot services are still available.
pub unsafe fn allocate_page_tables(kernel_phys: u64, kernel_size: usize) -> PageTableConfig {
    let kernel_4k_pages = (kernel_size + PAGE_SIZE - 1) / PAGE_SIZE;
    let pte_count = (kernel_4k_pages + 511) / 512;

    // PGD + PUD_KERNEL + PMD_KERNEL + PTE[n]
    let total_pages = 3 + pte_count;
    let pages_ptr = uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        total_pages,
    )
    .expect("Failed to allocate page tables");

    let base = pages_ptr.as_ptr() as u64;
    let mut off = 0u64;

    let pgd = base + off;
    off += PAGE_SIZE as u64;

    let pud_kernel = base + off;
    off += PAGE_SIZE as u64;

    let pmd_kernel = base + off;
    off += PAGE_SIZE as u64;

    let pte_base = base + off;

    PageTableConfig {
        pgd,
        pud_kernel,
        pmd_kernel,
        pte_base,
        kernel_phys,
        kernel_4k_pages,
        pte_count,
    }
}

/// Fill in all page-table entries.
///
/// Must be called **after** `exit_boot_services`.
///
/// Returns the physical address of the PGD, suitable for writing to
/// `CSR.PGDL`.
///
/// ## Caller responsibilities
///
/// Before activating the page tables the boot code must also:
///
/// 1. Write [`DMW0_VALUE`] / [`DMW1_VALUE`] to `CSR.DMW0` / `CSR.DMW1`.
/// 2. Write [`PWCL_VALUE`] / [`PWCH_VALUE`] to `CSR.PWCL` / `CSR.PWCH`.
/// 3. Write the returned PGD address to `CSR.PGDL`.
/// 4. Install a TLB refill handler and enable paging (`CSR.CRMD.PG = 1`).
///
/// # Memory map produced
///
/// Identity and physical-memory mappings are handled via DMW (no page-table
/// entries required).  The page table only covers the kernel:
///
/// | Virtual range | Physical | Granularity |
/// |---|---|---|
/// | PGD\[510\] base + kernel | `kernel_phys` … | 4 KiB pages |
///
/// # Safety
/// Caller must ensure boot services have been exited and the addresses in
/// `cfg` are still valid.
pub unsafe fn init_page_tables(cfg: &PageTableConfig) -> u64 {
    let total_pages = 3 + cfg.pte_count;

    serial_str("[PT] Initializing LoongArch64 page tables...\r\n");

    unsafe {
        core::ptr::write_bytes(cfg.pgd as *mut u8, 0, PAGE_SIZE * total_pages);

        let pgd = cfg.pgd as *mut u64;
        let pud = cfg.pud_kernel as *mut u64;
        let pmd = cfg.pmd_kernel as *mut u64;
        let pte_base = cfg.pte_base;

        // PGD[KERNEL_PGD_INDEX] → PUD_KERNEL
        // LoongArch directory entries are simply the physical address of the
        // next-level table (page-aligned, no flag bits).
        *pgd.add(KERNEL_PGD_INDEX) = cfg.pud_kernel;

        // PUD_KERNEL[0] → PMD_KERNEL
        *pud.add(0) = cfg.pmd_kernel;

        // PMD_KERNEL[0..n] → PTE pages
        for i in 0..cfg.pte_count {
            let pte_addr = pte_base + i as u64 * PAGE_SIZE as u64;
            *pmd.add(i) = pte_addr;
        }

        // PTE: map each 4 KiB kernel page
        for i in 0..cfg.kernel_4k_pages {
            let tbl_idx = i / 512;
            let ent_idx = i % 512;
            let pte = (pte_base + tbl_idx as u64 * PAGE_SIZE as u64) as *mut u64;
            let phys = cfg.kernel_phys + i as u64 * PAGE_SIZE as u64;
            *pte.add(ent_idx) = phys | LEAF_ATTRS;
        }
    }

    serial_str("[PT] LoongArch64 page tables initialized\r\n");

    cfg.pgd
}
