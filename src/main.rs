#![no_std]
#![no_main]

extern crate alloc;

mod boot;
mod config;
mod download;
mod fsutil;
mod menu;
mod net;
mod page_table;

use alloc::vec;
use core::fmt::Write;
use core::panic::PanicInfo;
use uefi::prelude::*;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;

pub const PAGE_SIZE: usize = 4096;
pub const FILE_BUFFER_SIZE: usize = 512;

fn serial_byte(b: u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b);
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::ptr::write_volatile(0x0900_0000 as *mut u8, b);
    }

    #[cfg(target_arch = "riscv64")]
    unsafe {
        core::ptr::write_volatile(0x1000_0000 as *mut u8, b);
    }

    #[cfg(target_arch = "loongarch64")]
    unsafe {
        core::ptr::write_volatile(0x1FE0_01E0 as *mut u8, b);
    }
}

pub fn serial_str(s: &str) {
    for b in s.bytes() {
        serial_byte(b);
    }
}

pub fn serial_hex(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    serial_str("0x");
    for i in (0..16).rev() {
        serial_byte(HEX[((val >> (i * 4)) & 0xF) as usize]);
    }
}

const CONFIG_PATH: &uefi::CStr16 = cstr16!("\\EFI\\BOOT\\bootloader.toml");

fn load_config() -> config::Config {
    let result = (|| -> Option<config::Config> {
        let loaded_image = uefi::boot::open_protocol_exclusive::<
            uefi::proto::loaded_image::LoadedImage,
        >(uefi::boot::image_handle())
        .ok()?;
        let device = loaded_image.device()?;

        let mut sfs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device).ok()?;
        let mut root = sfs.open_volume().ok()?;
        let handle = root
            .open(CONFIG_PATH, FileMode::Read, FileAttribute::empty())
            .ok()?;
        let mut file = handle.into_regular_file()?;

        let info = file.get_boxed_info::<FileInfo>().ok()?;
        let size = info.file_size() as usize;
        let mut buf = vec![0u8; size];
        file.read(&mut buf).ok()?;

        let text = core::str::from_utf8(&buf).ok()?;
        config::Config::from_str(text).ok()
    })();

    result.unwrap_or_default()
}

#[entry]
fn main() -> Status {
    let cfg = load_config();

    loop {
        let selected = menu::show(&cfg);

        uefi::system::with_stdout(|out| {
            let _ = write!(
                out,
                "Selected: [{}] {}\r\n",
                cfg.entry[selected].protocol, cfg.entry[selected].name,
            );
        });

        let entry = &cfg.entry[selected];
        let resolved = match download::resolve_all(&cfg, entry) {
            Ok(r) => r,
            Err(e) => {
                uefi::system::with_stdout(|out| {
                    let _ = write!(out, "Failed to load files: {:?}\r\n", e.status());
                    let _ = write!(out, "Press any key to return to menu...\r\n");
                });
                wait_for_key();
                continue;
            }
        };

        let Some(kernel) = resolved.kernel.as_deref() else {
            uefi::system::with_stdout(|out| {
                let _ = write!(out, "No kernel found in entry.\r\n");
                let _ = write!(out, "Press any key to return to menu...\r\n");
            });
            wait_for_key();
            continue;
        };

        match entry.protocol {
            config::Protocol::Linux => {
                let _ = boot::boot_linux(
                    kernel,
                    resolved.initrd.as_deref(),
                    resolved.cmdline.as_deref(),
                );
            }
            config::Protocol::Canicula => {
                let _ = boot::boot_canicula(kernel, resolved.cmdline.as_deref());
            }
        }

        return Status::SUCCESS;
    }
}

fn wait_for_key() {
    loop {
        uefi::boot::stall(core::time::Duration::from_millis(100));
        if let Ok(Some(_)) = uefi::system::with_stdin(|stdin| stdin.read_key()) {
            return;
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
