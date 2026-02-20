#![allow(dead_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use uefi::proto::network::http::HttpHelper;

use crate::config::{BootFile, Config, Entry, SearchMethod};
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

fn expand_vars(s: &str) -> String {
    let mut out = String::from(s);
    if out.contains("${arch}") {
        out = out.replace("${arch}", arch_name());
    }
    out
}

#[derive(Debug, Clone)]
pub struct DownloadedFile {
    pub url: String,
    pub data: Vec<u8>,
}

fn https_files(entry: &Entry) -> Vec<&BootFile> {
    entry
        .files
        .iter()
        .filter(|f| matches!(f.search, SearchMethod::Https))
        .collect()
}

pub fn fetch_needed(cfg: &Config, entry: &Entry) -> uefi::Result<Vec<DownloadedFile>> {
    let targets = https_files(entry);
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    // Pre-load any additional EFI drivers (NIC / HTTP stack) before we start.
    let _ = fsutil::load_drivers_from_config(cfg);

    let nic = net::select_nic_handle(cfg)?;
    net::bring_up_ipv4(cfg, nic)?;

    uefi::system::with_stdout(|out| {
        let _ = write!(out, "Creating HTTP client...\r\n");
    });
    let mut http = match HttpHelper::new(nic) {
        Ok(h) => h,
        Err(e) => {
            uefi::system::with_stdout(|out| {
                let _ = write!(out, "  HttpHelper::new failed: {:?}\r\n", e.status());
            });
            return Err(e);
        }
    };
    if let Err(e) = http.configure() {
        uefi::system::with_stdout(|out| {
            let _ = write!(out, "  http.configure failed: {:?}\r\n", e.status());
        });
        return Err(e);
    }

    let mut out_files = Vec::new();

    for f in targets {
        let Some(raw_url) = f.file.as_deref() else {
            continue;
        };
        let url = expand_vars(raw_url);

        uefi::system::with_stdout(|out| {
            let _ = write!(out, "Downloading {}...\r\n", url);
        });

        http.request_get(&url)?;
        let rsp = http.response_first(true)?;

        let mut data = rsp.body;
        loop {
            let more = http.response_more()?;
            if more.is_empty() {
                break;
            }
            data.extend_from_slice(&more);
        }

        uefi::system::with_stdout(|out| {
            let _ = write!(out, "  {} bytes\r\n", data.len());
        });

        out_files.push(DownloadedFile {
            url,
            data,
        });
    }

    Ok(out_files)
}
