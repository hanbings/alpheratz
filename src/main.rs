#![no_std]
#![no_main]

mod config;
mod menu;

use core::fmt::Write;
use core::panic::PanicInfo;
use uefi::prelude::*;

#[entry]
fn main() -> Status {
    let selected = menu::show();

    uefi::system::with_stdout(|out| {
        let _ = write!(
            out,
            "Selected: [{}] {}\r\n",
            config::ENTRIES[selected].protocol,
            config::ENTRIES[selected].name,
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
