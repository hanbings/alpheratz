pub const DEFAULT_ENTRY: usize = 0;
pub const TIMEOUT_SECS: usize = 3;

pub struct BootEntry {
    pub name: &'static str,
    pub protocol: &'static str,
}

pub static ENTRIES: &[BootEntry] = &[
    BootEntry {
        name: "Canicula Network Boot",
        protocol: "canicula",
    },
    BootEntry {
        name: "Linux Local Boot",
        protocol: "linux",
    },
    BootEntry {
        name: "Linux Network Boot",
        protocol: "linux",
    },
];
