extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use uefi::proto::network::http::HttpHelper;

use crate::config;
use crate::config::{Config, Entry, SearchMethod};
use crate::fsutil;
use crate::net;

fn arch_name() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    { "x86_64" }
    #[cfg(target_arch = "aarch64")]
    { "aarch64" }
    #[cfg(target_arch = "riscv64")]
    { "riscv64" }
    #[cfg(target_arch = "loongarch64")]
    { "loongarch64" }
}

pub fn expand_vars(s: &str) -> String {
    let mut out = String::from(s);
    if out.contains("${arch}") {
        out = out.replace("${arch}", arch_name());
    }
    out
}

/// All resolved boot data for a single entry.
pub struct ResolvedFiles {
    pub kernel: Option<Vec<u8>>,
    pub initrd: Option<Vec<u8>>,
    pub cmdline: Option<String>,
}

/// Resolve every file listed in `entry` — reading from ESP, downloading via
/// HTTPS, or extracting inline content — and return the combined result.
pub fn resolve_all(cfg: &Config, entry: &Entry) -> uefi::Result<ResolvedFiles> {
    let needs_https = entry.files.iter().any(|f| matches!(f.search, SearchMethod::Https));
    let needs_esp = entry.files.iter().any(|f| matches!(f.search, SearchMethod::Esp));

    let mut esp_root = if needs_esp {
        Some(fsutil::open_esp_root()?)
    } else {
        None
    };

    let mut http: Option<HttpHelper> = if needs_https {
        let _ = fsutil::load_drivers_from_config(cfg);
        let nic = net::select_nic_handle(cfg)?;
        net::bring_up_ipv4(cfg, nic)?;

        uefi::println!("Creating HTTP client...");
        let mut h = HttpHelper::new(nic).map_err(|e| {
            uefi::println!("  HttpHelper::new failed: {:?}", e.status());
            e
        })?;
        h.configure().map_err(|e| {
            uefi::println!("  http.configure failed: {:?}", e.status());
            e
        })?;
        Some(h)
    } else {
        None
    };

    let mut kernel: Option<Vec<u8>> = None;
    let mut initrd_parts: Vec<Vec<u8>> = Vec::new();
    let mut cmdline: Option<String> = None;

    for f in &entry.files {
        let data = match f.search {
            SearchMethod::Esp => {
                let path = f.file.as_deref().unwrap_or("");
                if path.is_empty() {
                    continue;
                }
                let path = expand_vars(path);
                uefi::println!("Reading {}...", path);
                let root = esp_root.as_mut().unwrap();
                let data = fsutil::read_file(root, &path)?;
                uefi::println!("  {} bytes", data.len());
                data
            }
            SearchMethod::Https => {
                let raw_url = f.file.as_deref().unwrap_or("");
                if raw_url.is_empty() {
                    continue;
                }
                let url = expand_vars(raw_url);
                uefi::println!("Downloading {}...", url);
                let h = http.as_mut().unwrap();
                h.request_get(&url)?;
                let rsp = h.response_first(true)?;
                let mut data = rsp.body;
                loop {
                    let more = h.response_more()?;
                    if more.is_empty() {
                        break;
                    }
                    data.extend_from_slice(&more);
                }
                uefi::println!("  {} bytes", data.len());
                data
            }
            SearchMethod::Inline => {
                if let Some(content) = &f.content {
                    Vec::from(content.as_bytes())
                } else {
                    continue;
                }
            }
        };

        match f.file_type {
            config::FileType::Kernel => kernel = Some(data),
            config::FileType::Initrd => initrd_parts.push(data),
            config::FileType::Cmdline => {
                if let Ok(s) = core::str::from_utf8(&data) {
                    cmdline = Some(String::from(s.trim_end_matches('\n')));
                }
            }
        }
    }

    let initrd = if initrd_parts.is_empty() {
        None
    } else if initrd_parts.len() == 1 {
        Some(initrd_parts.remove(0))
    } else {
        let total: usize = initrd_parts.iter().map(|p| p.len()).sum();
        let mut combined = Vec::with_capacity(total);
        for p in initrd_parts {
            combined.extend_from_slice(&p);
        }
        Some(combined)
    };

    Ok(ResolvedFiles {
        kernel,
        initrd,
        cmdline,
    })
}
