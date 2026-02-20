#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[repr(C)]
struct EfiTableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[repr(C)]
struct EfiSimpleTextOutputProtocol {
    reset: unsafe extern "efiapi" fn(*mut EfiSimpleTextOutputProtocol, bool) -> usize,
    output_string: unsafe extern "efiapi" fn(*mut EfiSimpleTextOutputProtocol, *const u16) -> usize,
}

#[repr(C)]
pub struct EfiSystemTable {
    hdr: EfiTableHeader,
    firmware_vendor: *const u16,
    firmware_revision: u32,
    _pad: u32,
    console_in_handle: usize,
    con_in: usize,
    console_out_handle: usize,
    con_out: *mut EfiSimpleTextOutputProtocol,
}

#[cfg(target_arch = "x86_64")]
const HELLO: &[u16] = &[
    0x0048, 0x0065, 0x006C, 0x006C, 0x006F, 0x0020, // Hello
    0x0066, 0x0072, 0x006F, 0x006D, 0x0020,          // from
    0x0078, 0x0038, 0x0036, 0x005F, 0x0036, 0x0034,  // x86_64
    0x0020, 0x0055, 0x0045, 0x0046, 0x0049, 0x0021,  //  UEFI!
    0x000D, 0x000A, 0x0000,
];

#[cfg(target_arch = "aarch64")]
const HELLO: &[u16] = &[
    0x0048, 0x0065, 0x006C, 0x006C, 0x006F, 0x0020, // Hello
    0x0066, 0x0072, 0x006F, 0x006D, 0x0020,          // from
    0x0041, 0x0041, 0x0072, 0x0063, 0x0068, 0x0036, 0x0034, // AArch64
    0x0020, 0x0055, 0x0045, 0x0046, 0x0049, 0x0021,  //  UEFI!
    0x000D, 0x000A, 0x0000,
];

#[cfg(target_arch = "riscv64")]
const HELLO: &[u16] = &[
    0x0048, 0x0065, 0x006C, 0x006C, 0x006F, 0x0020, // Hello
    0x0066, 0x0072, 0x006F, 0x006D, 0x0020,          // from
    0x0052, 0x0049, 0x0053, 0x0043, 0x002D, 0x0056,  // RISC-V
    0x0020, 0x0055, 0x0045, 0x0046, 0x0049, 0x0021,  //  UEFI!
    0x000D, 0x000A, 0x0000,
];

#[cfg(target_arch = "loongarch64")]
const HELLO: &[u16] = &[
    0x0048, 0x0065, 0x006C, 0x006C, 0x006F, 0x0020, // Hello
    0x0066, 0x0072, 0x006F, 0x006D, 0x0020,          // from
    0x004C, 0x006F, 0x006F, 0x006E, 0x0067,          // Loong
    0x0041, 0x0072, 0x0063, 0x0068, 0x0036, 0x0034,  // Arch64
    0x0020, 0x0055, 0x0045, 0x0046, 0x0049, 0x0021,  //  UEFI!
    0x000D, 0x000A, 0x0000,
];

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(_handle: usize, system_table: *mut EfiSystemTable) -> usize {
    let st = unsafe { &*system_table };
    let con_out = unsafe { &mut *st.con_out };

    unsafe {
        (con_out.output_string)(con_out as *mut _, HELLO.as_ptr());
    }

    0
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
