use uefi::prelude::*;

#[cfg(target_arch = "x86_64")]
mod x86_64;

pub fn boot_canicula(kernel: &[u8], cmdline: Option<&str>) -> Status {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::boot_canicula_elf(kernel, cmdline)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (kernel, cmdline);
        uefi::println!("Canicula ELF boot is currently only implemented for x86_64.");
        Status::UNSUPPORTED
    }
}
