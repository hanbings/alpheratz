extern crate alloc;

use core::ffi::c_void;
use core::fmt::Write;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use uefi::boot::{self, LoadImageSource};
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;

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

    let image_handle = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: kernel,
            file_path: None,
        },
    )
    .expect("load vmlinuz as EFI image");

    // cmdline_buf must outlive set_load_options → start_image
    let mut cmdline_buf = [0u16; 1024];

    if let Some(cl) = cmdline {
        uefi::system::with_stdout(|out| {
            let _ = write!(out, "  Cmdline: {}\r\n", cl);
        });

        let cl16 = uefi::CStr16::from_str_with_buf(cl, &mut cmdline_buf)
            .expect("command line too long");
        let size =
            (cl16.to_u16_slice_with_nul().len() * core::mem::size_of::<u16>()) as u32;

        let mut loaded_image = boot::open_protocol_exclusive::<LoadedImage>(image_handle)
            .expect("open LoadedImage on vmlinuz");
        unsafe {
            loaded_image.set_load_options(cmdline_buf.as_ptr() as *const u8, size);
        }
    }

    uefi::system::with_stdout(|out| {
        let _ = write!(out, "Starting Linux kernel...\r\n");
    });

    boot::start_image(image_handle).expect("start Linux kernel");

    Status::SUCCESS
}
