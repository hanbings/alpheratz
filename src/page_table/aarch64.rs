use uefi::boot::{AllocateType, MemoryType};

use crate::PAGE_SIZE;
use crate::serial::serial_str;

// Descriptor types

/// L0/L1/L2 table descriptor (points to next-level table).
const TABLE_DESC: u64 = 0b11;
/// L1/L2 block descriptor (1 GiB at L1, 2 MiB at L2).
const BLOCK_DESC: u64 = 0b01;
/// L3 page descriptor (4 KiB page).
const PAGE_DESC: u64 = 0b11;

// Block / page attribute bits

/// Access Flag – must be set or the first access faults.
const AF: u64 = 1 << 10;
/// Inner-Shareable.
const SH_INNER: u64 = 0b11 << 8;
/// AttrIndx = 0 → MAIR Attr0 (normal write-back memory).
const ATTR_NORMAL: u64 = 0 << 2;

/// Combined attribute bits for a normal-memory block / page.
const NORMAL_MEM_ATTRS: u64 = AF | SH_INNER | ATTR_NORMAL;

// Page-table layout constants

/// L0 index in TTBR1 table for the kernel mapping.
/// VA = 0xFFFF_0000_0000_0000 → bits [47:39] = 0.
pub const KERNEL_L0_INDEX: usize = 0;

/// L0 index in TTBR1 table for the physical-memory direct mapping.
/// VA = 0xFFFF_8000_0000_0000 → bits [47:39] = 256.
pub const PHYS_MAP_L0_INDEX: usize = 256;

/// Virtual address offset where all physical memory is linearly mapped.
pub const PHYSICAL_MEMORY_OFFSET: u64 = 0xFFFF_8000_0000_0000;

/// Recommended MAIR_EL1 value matching the AttrIndx encodings above.
///
/// | Index | Encoding | Meaning |
/// |-------|----------|---------|
/// | 0 | 0xFF | Normal, Inner/Outer Write-Back Non-transient |
/// | 1 | 0x00 | Device-nGnRnE |
/// | 2 | 0x44 | Normal, Inner/Outer Non-Cacheable |
pub const MAIR_VALUE: u64 = 0x0000_0000_0044_00FF;

/// Recommended TCR_EL1 value for 48-bit VA, 4 KiB granule, both halves.
///
/// T0SZ = 16, T1SZ = 16, TG0 = 4 KiB, TG1 = 4 KiB,
/// SH0/SH1 = Inner-Shareable, ORGN/IRGN = WB-WA, IPS = 48-bit PA.
pub const TCR_VALUE: u64 = {
    let t0sz: u64 = 16;
    let t1sz: u64 = 16 << 16;
    let tg0_4k: u64 = 0b00 << 14;
    let tg1_4k: u64 = 0b10 << 30;
    let sh0: u64 = 0b11 << 12;
    let sh1: u64 = 0b11 << 28;
    let orgn0: u64 = 0b01 << 10;
    let irgn0: u64 = 0b01 << 8;
    let orgn1: u64 = 0b01 << 26;
    let irgn1: u64 = 0b01 << 24;
    let ips_48: u64 = 0b101 << 32;
    t0sz | t1sz | tg0_4k | tg1_4k | sh0 | sh1 | orgn0 | irgn0 | orgn1 | irgn1 | ips_48
};

/// Holds allocated page-table pages for deferred initialization.
pub struct PageTableConfig {
    ttbr0_l0: u64,
    l1_low: u64,
    ttbr1_l0: u64,
    l1_kernel: u64,
    l2_kernel: u64,
    l1_phys_map: u64,
    l3_base: u64,
    kernel_phys: u64,
    kernel_4k_pages: usize,
    l3_count: usize,
}

impl PageTableConfig {
    pub fn ttbr0(&self) -> u64 {
        self.ttbr0_l0
    }

    pub fn ttbr1(&self) -> u64 {
        self.ttbr1_l0
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
    let l3_count = (kernel_4k_pages + 511) / 512;

    // TTBR0: L0 + L1_LOW
    // TTBR1: L0 + L1_KERNEL + L2_KERNEL + L1_PHYS_MAP + L3[n]
    let total_pages = 2 + 3 + 1 + l3_count;
    let pages_ptr = uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        total_pages,
    )
    .expect("Failed to allocate page tables");

    let base = pages_ptr.as_ptr() as u64;
    let mut off = 0u64;

    let ttbr0_l0 = base + off;
    off += PAGE_SIZE as u64;

    let l1_low = base + off;
    off += PAGE_SIZE as u64;

    let ttbr1_l0 = base + off;
    off += PAGE_SIZE as u64;

    let l1_kernel = base + off;
    off += PAGE_SIZE as u64;

    let l2_kernel = base + off;
    off += PAGE_SIZE as u64;

    let l1_phys_map = base + off;
    off += PAGE_SIZE as u64;

    let l3_base = base + off;

    PageTableConfig {
        ttbr0_l0,
        l1_low,
        ttbr1_l0,
        l1_kernel,
        l2_kernel,
        l1_phys_map,
        l3_base,
        kernel_phys,
        kernel_4k_pages,
        l3_count,
    }
}

/// Fill in all page-table entries.
///
/// Must be called **after** `exit_boot_services`.
///
/// Returns the physical address of the TTBR0 L0 table.  Use
/// [`PageTableConfig::ttbr1`] to obtain the TTBR1 L0 address.
///
/// Before switching, the caller must also programme `MAIR_EL1` and
/// `TCR_EL1` with [`MAIR_VALUE`] and [`TCR_VALUE`].
///
/// # Memory map produced
///
/// | Virtual range (TTBR0) | Physical | Granularity |
/// |---|---|---|
/// | 0 – 4 GiB identity | 0 – 4 GiB | 1 GiB L1 blocks |
///
/// | Virtual range (TTBR1) | Physical | Granularity |
/// |---|---|---|
/// | 0xFFFF_0000_0000_0000 + kernel | `kernel_phys` … | 4 KiB L3 pages |
/// | 0xFFFF_8000_0000_0000 + 0 – 4 GiB | 0 – 4 GiB | 1 GiB L1 blocks |
///
/// # Safety
/// Caller must ensure boot services have been exited and the addresses in
/// `cfg` are still valid.
pub unsafe fn init_page_tables(cfg: &PageTableConfig) -> u64 {
    let total_pages = 6 + cfg.l3_count;

    serial_str("[PT] Initializing AArch64 page tables...\r\n");

    unsafe {
        core::ptr::write_bytes(cfg.ttbr0_l0 as *mut u8, 0, PAGE_SIZE * total_pages);

        let ttbr0_l0 = cfg.ttbr0_l0 as *mut u64;
        let l1_low = cfg.l1_low as *mut u64;
        let ttbr1_l0 = cfg.ttbr1_l0 as *mut u64;
        let l1_kernel = cfg.l1_kernel as *mut u64;
        let l2_kernel = cfg.l2_kernel as *mut u64;
        let l1_phys_map = cfg.l1_phys_map as *mut u64;
        let l3_base = cfg.l3_base;

        // TTBR0: identity mapping for the first 4 GiB

        // L0[0] → L1_LOW (table descriptor)
        *ttbr0_l0.add(0) = cfg.l1_low | TABLE_DESC;

        // L1_LOW[0..4]: 4 × 1 GiB block descriptors
        for i in 0..4u64 {
            *l1_low.add(i as usize) = (i << 30) | NORMAL_MEM_ATTRS | BLOCK_DESC;
        }

        // TTBR1: kernel mapping

        // L0[KERNEL_L0_INDEX] → L1_KERNEL
        *ttbr1_l0.add(KERNEL_L0_INDEX) = cfg.l1_kernel | TABLE_DESC;

        // L1_KERNEL[0] → L2_KERNEL
        *l1_kernel.add(0) = cfg.l2_kernel | TABLE_DESC;

        // L2_KERNEL[0..n] → L3 tables
        for i in 0..cfg.l3_count {
            let l3_addr = l3_base + i as u64 * PAGE_SIZE as u64;
            *l2_kernel.add(i) = l3_addr | TABLE_DESC;
        }

        // L3: each entry maps a 4 KiB kernel page
        for i in 0..cfg.kernel_4k_pages {
            let l3_idx = i / 512;
            let pte_idx = i % 512;
            let l3 = (l3_base + l3_idx as u64 * PAGE_SIZE as u64) as *mut u64;
            let phys = cfg.kernel_phys + i as u64 * PAGE_SIZE as u64;
            *l3.add(pte_idx) = phys | NORMAL_MEM_ATTRS | PAGE_DESC;
        }

        // TTBR1: physical-memory direct mapping

        // L0[PHYS_MAP_L0_INDEX] → L1_PHYS_MAP
        *ttbr1_l0.add(PHYS_MAP_L0_INDEX) = cfg.l1_phys_map | TABLE_DESC;

        // L1_PHYS_MAP[0..4]: 4 × 1 GiB block descriptors
        for i in 0..4u64 {
            *l1_phys_map.add(i as usize) = (i << 30) | NORMAL_MEM_ATTRS | BLOCK_DESC;
        }
    }

    serial_str("[PT] AArch64 page tables initialized\r\n");

    cfg.ttbr0_l0
}
