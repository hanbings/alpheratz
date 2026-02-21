extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use core::fmt::Write;

use uefi::Identify;
use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams};
use uefi::prelude::*;
use uefi::proto::network::ip4config2::Ip4Config2;
use uefi::proto::network::snp::SimpleNetwork;
use uefi_raw::protocol::network::ip4_config2::Ip4Config2DataType;

use crate::config::{Config, NetworkType};

/// Open a protocol with GET_PROTOCOL attribute — does not affect driver binding.
unsafe fn open_snp_readonly(handle: Handle) -> uefi::Result<boot::ScopedProtocol<SimpleNetwork>> {
    unsafe {
        boot::open_protocol::<SimpleNetwork>(
            OpenProtocolParams {
                handle,
                agent: boot::image_handle(),
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )
    }
}

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
            if let Ok(snp) = unsafe { open_snp_readonly(h) } {
                if snp_mac6(&snp) == want {
                    return Ok(h);
                }
            }
        }
        Ok(fallback)
    } else {
        Ok(fallback)
    }
}

/// Recursively connect all controllers so higher-level network drivers get
/// loaded (MNP, ARP, IP4, DHCP4, TCP4, HTTP, …).
fn connect_all_controllers() {
    if let Ok(all) = boot::locate_handle_buffer(boot::SearchType::AllHandles) {
        for &h in all.iter() {
            let _ = boot::connect_controller(h, None, None, true);
        }
    }
}

/// Try to open Ip4Config2 — first on `preferred`, then by scanning all handles.
fn open_ip4config2(preferred: Handle) -> uefi::Result<boot::ScopedProtocol<Ip4Config2>> {
    if let Ok(p) = Ip4Config2::new(preferred) {
        return Ok(p);
    }

    let handles = boot::locate_handle_buffer(boot::SearchType::ByProtocol(&Ip4Config2::GUID))?;
    uefi::println!("  Ip4Config2 handles found: {}", handles.len());

    for &h in handles.iter() {
        if let Ok(p) = Ip4Config2::new(h) {
            return Ok(p);
        }
    }

    Err(uefi::Error::from(Status::NOT_FOUND))
}

fn count_protocol_handles(guid: &uefi::Guid) -> usize {
    boot::locate_handle_buffer(boot::SearchType::ByProtocol(guid))
        .map(|h| h.len())
        .unwrap_or(0)
}

pub fn bring_up_ipv4(cfg: &Config, nic: Handle) -> uefi::Result<()> {
    if let Ok(snp) = unsafe { open_snp_readonly(nic) } {
        uefi::println!("NIC: {}", mac_to_string(snp_mac6(&snp)));
    }

    for pass in 0..6u32 {
        let _ = boot::connect_controller(nic, None, None, true);
        connect_all_controllers();

        if count_protocol_handles(&Ip4Config2::GUID) > 0 {
            break;
        }
        if pass == 5 {
            uefi::println!("  Network stack failed to initialize");
        }
    }

    let want_dhcp = cfg
        .network
        .as_ref()
        .and_then(|n| n.network_type)
        .unwrap_or(NetworkType::Dhcp);

    match want_dhcp {
        NetworkType::Dhcp => {
            uefi::println!("Waiting for DHCP...");

            let mut ip4 = open_ip4config2(nic).map_err(|e| {
                uefi::println!("  Ip4Config2 not found on any handle: {:?}", e.status());
                e
            })?;

            ip4.ifup().map_err(|e| {
                uefi::println!("  ifup failed: {:?}", e.status());
                e
            })?;

            if let Ok(info) = ip4.get_interface_info() {
                uefi::println!("  IP:      {}", info.station_addr);
                uefi::println!("  Netmask: {}", info.subnet_mask);
            }
            if let Ok(gw) = ip4.get_data(Ip4Config2DataType::GATEWAY) {
                if gw.len() >= 4 {
                    uefi::println!("  Gateway: {}.{}.{}.{}", gw[0], gw[1], gw[2], gw[3]);
                }
            }
            if let Ok(dns) = ip4.get_data(Ip4Config2DataType::DNS_SERVER) {
                for c in dns.chunks_exact(4) {
                    uefi::println!("  DNS:     {}.{}.{}.{}", c[0], c[1], c[2], c[3]);
                }
            }
            uefi::println!("IPv4 ready.");
        }
    }
    Ok(())
}
