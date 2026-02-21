use core::arch::asm;

use uefi::boot::{self, AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat};

use canicula_common::entry::{
    BootInfo, FrameBuffer, FrameBufferInfo, MemoryRegion, MemoryRegionKind, MemoryRegions,
    PixelFormat,
};

use crate::page_table;

pub const PAGE_SIZE: usize = 4096;

static mut BOOT_INFO: BootInfo = BootInfo {
    memory_regions: MemoryRegions::new(),
    framebuffer: None,
    physical_memory_offset: None,
    rsdp_addr: None,
};

fn convert_memory_type(ty: MemoryType) -> MemoryRegionKind {
    match ty {
        MemoryType::CONVENTIONAL => MemoryRegionKind::Usable,
        MemoryType::LOADER_CODE
        | MemoryType::LOADER_DATA
        | MemoryType::BOOT_SERVICES_CODE
        | MemoryType::BOOT_SERVICES_DATA => MemoryRegionKind::Bootloader,
        _ => MemoryRegionKind::UnknownUefi(ty.0),
    }
}

fn convert_pixel_format(format: UefiPixelFormat) -> PixelFormat {
    match format {
        UefiPixelFormat::Rgb => PixelFormat::Rgb,
        UefiPixelFormat::Bgr => PixelFormat::Bgr,
        _ => PixelFormat::Unknown {
            red_position: 0,
            green_position: 8,
            blue_position: 16,
        },
    }
}

/// Boot a Canicula kernel ELF on x86_64.
///
/// 1. Parses the ELF and loads PT_LOAD segments into physical memory
/// 2. Sets up 4-level page tables (identity + kernel + physical memory map)
/// 3. Collects framebuffer, memory map and RSDP into a [`BootInfo`]
/// 4. Exits UEFI boot services
/// 5. Switches to new page tables and jumps to the kernel entry point
///    with a pointer to `BootInfo` in `rdi`
pub fn boot_canicula_elf(kernel: &[u8], _cmdline: Option<&str>) -> Status {
    use log::info;
    use xmas_elf::ElfFile;
    use xmas_elf::program::Type;

    info!("Canicula ELF Boot (x86_64)");
    info!("  Kernel ELF size: {} bytes", kernel.len());

    let elf = ElfFile::new(kernel).expect("Failed to parse ELF");
    let entry_point = elf.header.pt2.entry_point();
    info!("ELF entry point: {:#x}", entry_point);

    let mut min_virt: u64 = u64::MAX;
    let mut max_virt: u64 = 0;

    for ph in elf.program_iter() {
        if ph.get_type().unwrap() == Type::Load {
            let start = ph.virtual_addr();
            let end = start + ph.mem_size();
            if start < min_virt {
                min_virt = start;
            }
            if end > max_virt {
                max_virt = end;
            }
        }
    }

    let total_size = (max_virt - min_virt) as usize;
    let num_pages = (total_size + PAGE_SIZE - 1) / PAGE_SIZE;

    info!("Kernel virtual range: {:#x} - {:#x}", min_virt, max_virt);
    info!("Kernel size: {} pages", num_pages);

    let num_pages_aligned = ((total_size + 0x20_0000 - 1) / 0x20_0000) * 512;
    let kernel_phys_ptr = boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        num_pages_aligned,
    )
    .expect("Failed to allocate memory for kernel");

    let kernel_phys_base = kernel_phys_ptr.as_ptr() as u64;
    info!("Kernel physical base: {:#x}", kernel_phys_base);

    for ph in elf.program_iter() {
        if ph.get_type().unwrap() == Type::Load {
            let virt_addr = ph.virtual_addr();
            let offset_from_base = virt_addr - min_virt;
            let phys_addr = kernel_phys_base + offset_from_base;

            let src_offset = ph.offset() as usize;
            let file_size = ph.file_size() as usize;
            let mem_size = ph.mem_size() as usize;

            unsafe {
                let dest = phys_addr as *mut u8;
                let src = kernel.as_ptr().add(src_offset);
                core::ptr::copy_nonoverlapping(src, dest, file_size);

                if mem_size > file_size {
                    core::ptr::write_bytes(dest.add(file_size), 0, mem_size - file_size);
                }
            }

            info!(
                "  Loaded: virt {:#x} -> phys {:#x} ({} bytes)",
                virt_addr, phys_addr, mem_size
            );
        }
    }

    let kernel_pml4_index = ((min_virt >> 39) & 0x1FF) as usize;

    info!("Allocating page tables...");
    let pt_config =
        unsafe { page_table::allocate_page_tables(kernel_phys_base, total_size, kernel_pml4_index) };
    info!("Page table memory allocated at: {:#x}", pt_config.root());

    const KERNEL_STACK_SIZE: usize = 1024 * 1024;
    let stack_pages = (KERNEL_STACK_SIZE + PAGE_SIZE - 1) / PAGE_SIZE;
    let stack_ptr = boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        stack_pages,
    )
    .expect("Failed to allocate kernel stack");
    let stack_top = (stack_ptr.as_ptr() as u64 + KERNEL_STACK_SIZE as u64) & !0xF;
    info!(
        "Kernel stack allocated: base={:#x}, top={:#x}",
        stack_ptr.as_ptr() as u64,
        stack_top
    );

    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>().unwrap();
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle).unwrap();

    let mode_info = gop.current_mode_info();
    let (width, height) = mode_info.resolution();
    let stride = mode_info.stride();
    let fb_addr = gop.frame_buffer().as_mut_ptr() as u64;
    let fb_size = gop.frame_buffer().size();
    let pixel_format = convert_pixel_format(mode_info.pixel_format());

    info!(
        "Screen resolution: {}x{}, stride: {}",
        width, height, stride
    );
    info!("Framebuffer address: {:#x}, size: {}", fb_addr, fb_size);

    let rsdp_addr = uefi::system::with_config_table(|entries| {
        for entry in entries {
            if entry.guid == uefi::table::cfg::ConfigTableEntry::ACPI2_GUID {
                return Some(entry.address as u64);
            }
            if entry.guid == uefi::table::cfg::ConfigTableEntry::ACPI_GUID {
                return Some(entry.address as u64);
            }
        }
        None
    });
    info!("RSDP address: {:?}", rsdp_addr);

    info!("Exiting boot services...");
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };

    unsafe {
        let boot_info_ptr = core::ptr::addr_of_mut!(BOOT_INFO);

        for desc in memory_map.entries() {
            let start = desc.phys_start;
            let end = start + desc.page_count * PAGE_SIZE as u64;
            let kind = convert_memory_type(desc.ty);

            (*boot_info_ptr)
                .memory_regions
                .push(MemoryRegion { start, end, kind });
        }

        (*boot_info_ptr).framebuffer = Some(FrameBuffer::new(
            fb_addr,
            fb_size,
            FrameBufferInfo {
                width,
                height,
                stride,
                bytes_per_pixel: 4,
                pixel_format,
            },
        ));

        (*boot_info_ptr).physical_memory_offset = Some(page_table::PHYSICAL_MEMORY_OFFSET);
        (*boot_info_ptr).rsdp_addr = rsdp_addr;
    }

    let pml4_phys = unsafe { page_table::init_page_tables(&pt_config) };

    crate::serial::serial_str("[LOADER] Jumping to kernel at ");
    crate::serial::serial_hex(entry_point);
    crate::serial::serial_str("\r\n");

    unsafe {
        let boot_info_ptr = core::ptr::addr_of_mut!(BOOT_INFO);

        asm!(
            "mov rsp, {stack}",
            "mov cr3, {cr3}",
            "jmp {entry}",
            stack = in(reg) stack_top,
            cr3 = in(reg) pml4_phys,
            entry = in(reg) entry_point,
            in("rdi") boot_info_ptr,
            options(noreturn)
        );
    }
}
