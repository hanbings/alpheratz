#![allow(dead_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use core::fmt::Write;

use uefi::Identify;
use uefi::boot;
use uefi::prelude::*;
use uefi::proto::network::ip4config2::Ip4Config2;
use uefi::proto::network::snp::SimpleNetwork;

use crate::config::{Config, NetworkType};

fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split(|c| c == ':' || c == '-').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 {
            return None;
        }
        out[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(out)
}

fn snp_mac6(snp: &SimpleNetwork) -> [u8; 6] {
    let mac = snp.mode().current_address;
    let mut out = [0u8; 6];
    out.copy_from_slice(&mac.0[0..6]);
    out
}

fn mac_to_string(mac: [u8; 6]) -> String {
    let mut s = String::with_capacity(17);
    for (i, b) in mac.iter().enumerate() {
        if i > 0 {
            s.push(':');
        }
        let _ = write!(s, "{:02X}", b);
    }
    s
}

fn locate_snp_handles() -> uefi::Result<Vec<Handle>> {
    let handles = boot::locate_handle_buffer(boot::SearchType::ByProtocol(&SimpleNetwork::GUID))?;
    Ok(handles.to_vec())
}

pub fn select_nic_handle(cfg: &Config) -> uefi::Result<Handle> {
    let handles = locate_snp_handles()?;
    if handles.is_empty() {
        return Err(uefi::Error::from(Status::NOT_FOUND));
    }
    let fallback = handles[0];

    let want = cfg
        .network
        .as_ref()
        .and_then(|n| n.bind.as_deref())
        .and_then(parse_mac);

    if let Some(want) = want {
        for &h in handles.iter() {
            if let Ok(snp) = boot::open_protocol_exclusive::<SimpleNetwork>(h) {
                if snp_mac6(&snp) == want {
                    return Ok(h);
                }
            }
        }
        // If bind is set but no match, fall back to first NIC.
        Ok(fallback)
    } else {
        Ok(fallback)
    }
}

pub fn bring_up_ipv4(cfg: &Config, nic: Handle) -> uefi::Result<()> {
    if let Ok(snp) = boot::open_protocol_exclusive::<SimpleNetwork>(nic) {
        let _ = snp.start();
        let _ = snp.initialize(0, 0);

        let mac = snp_mac6(&snp);
        uefi::system::with_stdout(|out| {
            let _ = writeln!(out, "NIC: {}", mac_to_string(mac));
        });
    }

    // Recursively connect all controllers to ensure the full network driver
    // stack (MNP → ARP → IP4 → DHCP4 → TCP4 → HTTP) is bound.
    // BDS may not have connected network drivers if no network boot was attempted.
    if let Ok(all) = boot::locate_handle_buffer(boot::SearchType::AllHandles) {
        for &h in all.iter() {
            let _ = boot::connect_controller(h, None, None, true);
        }
    }

    let want_dhcp = cfg
        .network
        .as_ref()
        .and_then(|n| n.network_type)
        .unwrap_or(NetworkType::Dhcp);

    match want_dhcp {
        NetworkType::Dhcp => {
            uefi::system::with_stdout(|out| {
                let _ = write!(out, "Waiting for DHCP...\r\n");
            });
            let mut ip4 = match Ip4Config2::new(nic) {
                Ok(v) => v,
                Err(e) => {
                    uefi::system::with_stdout(|out| {
                        let _ = write!(out, "  Ip4Config2::new failed: {:?}\r\n", e.status());
                    });
                    return Err(e);
                }
            };
            if let Err(e) = ip4.ifup() {
                uefi::system::with_stdout(|out| {
                    let _ = write!(out, "  ifup failed: {:?}\r\n", e.status());
                });
                return Err(e);
            }
            uefi::system::with_stdout(|out| {
                let _ = write!(out, "IPv4 ready.\r\n");
            });
        }
    }
    Ok(())
}
