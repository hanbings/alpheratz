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
mod serial;

use alloc::vec;
use core::panic::PanicInfo;
use uefi::prelude::*;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;

pub const PAGE_SIZE: usize = 4096;

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

        uefi::println!(
            "Selected: [{}] {}",
            cfg.entry[selected].protocol,
            cfg.entry[selected].name,
        );

        let entry = &cfg.entry[selected];
        let resolved = match download::resolve_all(&cfg, entry) {
            Ok(r) => r,
            Err(e) => {
                uefi::println!("Failed to load files: {:?}", e.status());
                uefi::println!("Press any key to return to menu...");
                wait_for_key();
                continue;
            }
        };

        let Some(kernel) = resolved.kernel.as_deref() else {
            uefi::println!("No kernel found in entry.");
            uefi::println!("Press any key to return to menu...");
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
