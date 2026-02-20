#![no_std]
#![no_main]

extern crate alloc;

mod config;
mod menu;

use alloc::vec;
use core::fmt::Write;
use core::panic::PanicInfo;
use uefi::prelude::*;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;

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
    let selected = menu::show(&cfg);

    uefi::system::with_stdout(|out| {
        let _ = write!(
            out,
            "Selected: [{}] {}\r\n",
            cfg.entry[selected].protocol, cfg.entry[selected].name,
        );
    });

    // TODO: actually boot the selected entry
    uefi::boot::stall(core::time::Duration::from_secs(3));

    Status::SUCCESS
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
