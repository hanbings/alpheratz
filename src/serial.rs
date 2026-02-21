/// Low-level serial output for use after exit_boot_services(),
/// when UEFI stdout is no longer available.

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

#[allow(dead_code)]
pub fn serial_hex(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    serial_str("0x");
    for i in (0..16).rev() {
        serial_byte(HEX[((val >> (i * 4)) & 0xF) as usize]);
    }
}
