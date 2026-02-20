#![no_std]
#![no_main]

use core::panic::PanicInfo;
use uefi::prelude::*;

#[cfg(target_arch = "x86_64")]
const HELLO: &uefi::CStr16 = cstr16!("Hello from x86_64 UEFI!\r\n");

#[cfg(target_arch = "aarch64")]
const HELLO: &uefi::CStr16 = cstr16!("Hello from AArch64 UEFI!\r\n");

#[cfg(target_arch = "riscv64")]
const HELLO: &uefi::CStr16 = cstr16!("Hello from RISC-V UEFI!\r\n");

#[cfg(target_arch = "loongarch64")]
const HELLO: &uefi::CStr16 = cstr16!("Hello from LoongArch64 UEFI!\r\n");

#[entry]
fn main() -> Status {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(HELLO);
    });

    Status::SUCCESS
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
