extern crate alloc;

use core::ffi::c_void;
use core::fmt::Write;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use uefi::boot::{self, AllocateType, LoadImageSource, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;

#[cfg(target_arch = "x86_64")]
use core::arch::asm;

#[cfg(target_arch = "x86_64")]
use crate::page_table;

#[cfg(target_arch = "x86_64")]
use canicula_common::entry::{
    BootInfo, FrameBuffer, FrameBufferInfo, MemoryRegion, MemoryRegionKind, MemoryRegions,
    PixelFormat,
};

#[cfg(target_arch = "x86_64")]
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat};

static INITRD_DATA_PTR: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static INITRD_DATA_LEN: AtomicUsize = AtomicUsize::new(0);

/// Vendor Media Device Path node identifying the Linux initrd, followed by
/// an End-of-Device-Path node.  The Linux EFI stub (5.8+) searches for a
/// handle carrying this device path together with the LoadFile2 protocol.
#[repr(C, packed)]
struct InitrdDevicePath {
    vendor_type: u8,
    vendor_subtype: u8,
    vendor_length: [u8; 2],
    vendor_guid: [u8; 16],
    end_type: u8,
    end_subtype: u8,
    end_length: [u8; 2],
}

unsafe impl Sync for InitrdDevicePath {}

/// LINUX_EFI_INITRD_MEDIA_GUID  {5568e427-68fc-4f3d-ac74-ca555231cc68}
static INITRD_DEVICE_PATH: InitrdDevicePath = InitrdDevicePath {
    vendor_type: 0x04,
    vendor_subtype: 0x03,
    vendor_length: [20, 0],
    vendor_guid: [
        0x27, 0xe4, 0x68, 0x55, 0xfc, 0x68, 0x3d, 0x4f, 0xac, 0x74, 0xca, 0x55, 0x52, 0x31,
        0xcc, 0x68,
    ],
    end_type: 0x7f,
    end_subtype: 0xff,
    end_length: [4, 0],
};

#[repr(C)]
struct RawLoadFile2Protocol {
    load_file: unsafe extern "efiapi" fn(
        this: *mut RawLoadFile2Protocol,
        file_path: *const c_void,
        boot_policy: bool,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> Status,
}

unsafe impl Sync for RawLoadFile2Protocol {}

unsafe extern "efiapi" fn initrd_load_file(
    _this: *mut RawLoadFile2Protocol,
    _file_path: *const c_void,
    _boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut c_void,
) -> Status {
    let ptr = INITRD_DATA_PTR.load(Ordering::Relaxed);
    let len = INITRD_DATA_LEN.load(Ordering::Relaxed);

    if ptr.is_null() || len == 0 {
        return Status::NOT_FOUND;
    }

    unsafe {
        if buffer.is_null() || *buffer_size < len {
            *buffer_size = len;
            return Status::BUFFER_TOO_SMALL;
        }
        core::ptr::copy_nonoverlapping(ptr, buffer as *mut u8, len);
        *buffer_size = len;
    }

    Status::SUCCESS
}

static INITRD_LOAD_FILE2: RawLoadFile2Protocol = RawLoadFile2Protocol {
    load_file: initrd_load_file,
};

const DEVICE_PATH_PROTOCOL_GUID: uefi::Guid =
    uefi::guid!("09576e91-6d3f-11d2-8e39-00a0c969723b");
const LOAD_FILE2_PROTOCOL_GUID: uefi::Guid =
    uefi::guid!("4006c0c1-fcb3-403e-996d-4a6c8724e06d");

fn install_initrd_load_file2(initrd_data: &[u8]) {
    INITRD_DATA_PTR.store(initrd_data.as_ptr() as *mut u8, Ordering::Relaxed);
    INITRD_DATA_LEN.store(initrd_data.len(), Ordering::Relaxed);

    let handle = unsafe {
        boot::install_protocol_interface(
            None,
            &DEVICE_PATH_PROTOCOL_GUID,
            &INITRD_DEVICE_PATH as *const InitrdDevicePath as *const c_void,
        )
    }
    .expect("install initrd device path");

    unsafe {
        boot::install_protocol_interface(
            Some(handle),
            &LOAD_FILE2_PROTOCOL_GUID,
            &INITRD_LOAD_FILE2 as *const RawLoadFile2Protocol as *const c_void,
        )
    }
    .expect("install initrd LoadFile2");
}

fn print_status(prefix: &str, s: Status) {
    uefi::system::with_stdout(|out| {
        let _ = write!(out, "{}{:?}\r\n", prefix, s);
    });
}

/// Boot a Linux kernel via the EFI stub mechanism.
///
/// `kernel`  – raw vmlinuz / bzImage PE/COFF bytes
/// `initrd`  – optional concatenated initrd(s)
/// `cmdline` – optional kernel command line
pub fn boot_linux(kernel: &[u8], initrd: Option<&[u8]>, cmdline: Option<&str>) -> Status {
    uefi::system::with_stdout(|out| {
        let _ = write!(out, "Linux EFI Stub Boot\r\n");
        let _ = write!(out, "  Kernel: {} bytes\r\n", kernel.len());
    });

    if let Some(rd) = initrd {
        uefi::system::with_stdout(|out| {
            let _ = write!(out, "  Initrd: {} bytes\r\n", rd.len());
        });
        install_initrd_load_file2(rd);
    }

    uefi::system::with_stdout(|out| {
        let _ = write!(out, "Loading EFI kernel image...\r\n");
    });

    let image_handle = match boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: kernel,
            file_path: None,
        },
    ) {
        Ok(h) => h,
        Err(e) => {
            print_status("LoadImage failed: ", e.status());
            uefi::system::with_stdout(|out| {
                let _ = write!(
                    out,
                    "Hint: kernel must be a PE/COFF EFI stub image (not ELF).\r\n"
                );
            });
            return e.status();
        }
    };

    // cmdline_buf must outlive set_load_options → start_image
    let mut cmdline_buf = [0u16; 1024];

    if let Some(cl) = cmdline {
        uefi::system::with_stdout(|out| {
            let _ = write!(out, "  Cmdline: {}\r\n", cl);
        });

        let cl16 = match uefi::CStr16::from_str_with_buf(cl, &mut cmdline_buf) {
            Ok(v) => v,
            Err(_) => {
                uefi::system::with_stdout(|out| {
                    let _ = write!(out, "Cmdline too long (max 1024 UTF-16 code units)\r\n");
                });
                return Status::INVALID_PARAMETER;
            }
        };
        let size =
            (cl16.to_u16_slice_with_nul().len() * core::mem::size_of::<u16>()) as u32;

        let mut loaded_image = match boot::open_protocol_exclusive::<LoadedImage>(image_handle) {
            Ok(v) => v,
            Err(e) => {
                print_status("OpenProtocol(LoadedImage) failed: ", e.status());
                return e.status();
            }
        };
        unsafe {
            loaded_image.set_load_options(cmdline_buf.as_ptr() as *const u8, size);
        }
    }

    uefi::system::with_stdout(|out| {
        let _ = write!(out, "Starting Linux kernel...\r\n");
    });

    if let Err(e) = boot::start_image(image_handle) {
        print_status("StartImage failed: ", e.status());
        return e.status();
    }

    Status::SUCCESS
}

pub fn boot_canicula(kernel: &[u8], cmdline: Option<&str>) -> Status {
    #[cfg(target_arch = "x86_64")]
    {
        boot_canicula_elf_x86_64(kernel, cmdline)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = cmdline;
        uefi::system::with_stdout(|out| {
            let _ = write!(
                out,
                "Canicula ELF boot is currently only implemented for x86_64.\r\n"
            );
        });
        Status::UNSUPPORTED
    }
}

#[cfg(target_arch = "x86_64")]
static mut BOOT_INFO: BootInfo = BootInfo {
    memory_regions: MemoryRegions::new(),
    framebuffer: None,
    physical_memory_offset: None,
    rsdp_addr: None,
};

#[cfg(target_arch = "x86_64")]
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

#[cfg(target_arch = "x86_64")]
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
#[cfg(target_arch = "x86_64")]
fn boot_canicula_elf_x86_64(kernel: &[u8], _cmdline: Option<&str>) -> Status {
    use log::info;
    use xmas_elf::ElfFile;
    use xmas_elf::program::Type;

    info!("Canicula ELF Boot (x86_64)");
    info!("  Kernel ELF size: {} bytes", kernel.len());

    let elf = ElfFile::new(kernel).expect("Failed to parse ELF");
    let entry_point = elf.header.pt2.entry_point();
    info!("ELF entry point: {:#x}", entry_point);

    // Compute the virtual memory range covered by all PT_LOAD segments.
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
    let num_pages = (total_size + crate::PAGE_SIZE - 1) / crate::PAGE_SIZE;

    info!("Kernel virtual range: {:#x} - {:#x}", min_virt, max_virt);
    info!("Kernel size: {} pages", num_pages);

    // Allocate physical memory (2 MiB-aligned so huge-page identity mapping
    // doesn't accidentally overlap kernel pages).
    let num_pages_aligned = ((total_size + 0x20_0000 - 1) / 0x20_0000) * 512;
    let kernel_phys_ptr = boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        num_pages_aligned,
    )
    .expect("Failed to allocate memory for kernel");

    let kernel_phys_base = kernel_phys_ptr.as_ptr() as u64;
    info!("Kernel physical base: {:#x}", kernel_phys_base);

    // Load each ELF segment into the allocated physical memory.
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

    // Derive the PML4 index from the kernel's virtual base address.
    let kernel_pml4_index = ((min_virt >> 39) & 0x1FF) as usize;

    // Allocate page tables (must happen before exit_boot_services).
    info!("Allocating page tables...");
    let pt_config =
        unsafe { page_table::allocate_page_tables(kernel_phys_base, total_size, kernel_pml4_index) };
    info!("Page table memory allocated at: {:#x}", pt_config.root());

    // Allocate kernel stack (1 MiB).
    const KERNEL_STACK_SIZE: usize = 1024 * 1024;
    let stack_pages = (KERNEL_STACK_SIZE + crate::PAGE_SIZE - 1) / crate::PAGE_SIZE;
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

    // Collect framebuffer information from GOP.
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

    // Locate the ACPI RSDP from the UEFI configuration table.
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

    // Exit UEFI boot services — no more UEFI calls after this point.
    info!("Exiting boot services...");
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };

    // Convert the UEFI memory map into BootInfo format.
    unsafe {
        let boot_info_ptr = core::ptr::addr_of_mut!(BOOT_INFO);

        for desc in memory_map.entries() {
            let start = desc.phys_start;
            let end = start + desc.page_count * crate::PAGE_SIZE as u64;
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

    // Initialize page tables (after exit_boot_services).
    let pml4_phys = unsafe { page_table::init_page_tables(&pt_config) };

    crate::serial_str("[LOADER] Jumping to kernel at ");
    crate::serial_hex(entry_point);
    crate::serial_str("\r\n");

    // Switch to the new page tables and jump to the kernel entry point.
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
