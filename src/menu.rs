use core::fmt::Write;
use core::time::Duration;

use uefi::prelude::*;
use uefi::proto::console::text::{Color, Key, ScanCode};
use uefi::runtime::{ResetType, VariableAttributes, VariableVendor};

use crate::config::Config;

enum Selection {
    Entry(usize),
    Firmware,
    Shutdown,
}

fn total_items(cfg: &Config) -> usize {
    cfg.entry.len() + cfg.firmware as usize + cfg.shutdown as usize
}

fn index_to_selection(cfg: &Config, idx: usize) -> Selection {
    if idx < cfg.entry.len() {
        return Selection::Entry(idx);
    }
    let extra = idx - cfg.entry.len();
    if cfg.firmware && extra == 0 {
        return Selection::Firmware;
    }
    Selection::Shutdown
}

/// Display the boot menu and return the index of the selected boot entry.
///
/// Firmware / Shutdown selections never return — they call `uefi::runtime::reset`.
pub fn show(cfg: &Config) -> usize {
    let total = total_items(cfg);
    if total == 0 {
        uefi::system::with_stdout(|out| {
            let _ = write!(out, "No boot entries found in config.\r\n");
        });
        loop {
            uefi::boot::stall(Duration::from_secs(1));
        }
    }

    let mut selected = cfg.default_entry_index().min(total - 1);
    let mut timeout: Option<usize> = if cfg.timeout > 0 {
        Some(cfg.timeout)
    } else {
        None
    };
    let mut tick_count: usize = 0;

    uefi::system::with_stdout(|out| {
        let _ = out.clear();
        let _ = out.enable_cursor(false);
    });

    draw(cfg, selected, timeout);

    loop {
        uefi::boot::stall(Duration::from_millis(100));

        let key = uefi::system::with_stdin(|stdin| stdin.read_key());

        if let Ok(Some(key)) = key {
            timeout = None;

            match key {
                Key::Special(ScanCode::UP) if selected > 0 => {
                    selected -= 1;
                }
                Key::Special(ScanCode::DOWN) if selected < total - 1 => {
                    selected += 1;
                }
                Key::Printable(c) if u16::from(c) == 0x000D => {
                    return confirm(cfg, selected);
                }
                _ => {}
            }

            draw(cfg, selected, timeout);
        }

        tick_count += 1;
        if tick_count >= 10 {
            tick_count = 0;
            if let Some(ref mut t) = timeout {
                if *t == 0 {
                    return confirm(cfg, selected);
                }
                *t -= 1;
                draw(cfg, selected, timeout);
            }
        }
    }
}

/// Act on the current selection. Returns the boot-entry index if it's an
/// `Entry`; firmware/shutdown paths diverge and never return.
fn confirm(cfg: &Config, selected: usize) -> usize {
    match index_to_selection(cfg, selected) {
        Selection::Entry(idx) => {
            uefi::system::with_stdout(|out| {
                let _ = out.set_color(Color::White, Color::Black);
                let _ = out.clear();
                let _ = write!(out, "Booting {}...\n", cfg.entry[idx].name);
            });
            idx
        }
        Selection::Firmware => reboot_to_firmware(),
        Selection::Shutdown => {
            uefi::runtime::reset(ResetType::SHUTDOWN, uefi::Status::SUCCESS, None);
        }
    }
}

/// Set OsIndications bit 0 (EFI_OS_INDICATIONS_BOOT_TO_FW_UI) and cold-reset.
fn reboot_to_firmware() -> ! {
    const EFI_OS_INDICATIONS_BOOT_TO_FW_UI: u64 = 0x0000_0000_0000_0001;

    let name = cstr16!("OsIndications");
    let vendor = &VariableVendor::GLOBAL_VARIABLE;
    let attrs = VariableAttributes::NON_VOLATILE
        | VariableAttributes::BOOTSERVICE_ACCESS
        | VariableAttributes::RUNTIME_ACCESS;

    let _ = uefi::runtime::set_variable(
        name,
        vendor,
        attrs,
        &EFI_OS_INDICATIONS_BOOT_TO_FW_UI.to_le_bytes(),
    );

    uefi::runtime::reset(ResetType::COLD, uefi::Status::SUCCESS, None);
}

fn draw(cfg: &Config, selected: usize, timeout: Option<usize>) {
    uefi::system::with_stdout(|out| {
        let _ = out.set_cursor_position(0, 0);

        let _ = out.set_color(Color::White, Color::Black);
        let _ = write!(out, "\n");
        let _ = write!(out, "  Alpheratz Boot Loader\n");
        let _ = write!(out, "\n");

        let mut row: usize = 0;

        for (i, entry) in cfg.entry.iter().enumerate() {
            draw_item(out, i == selected, &entry.name);
            row += 1;
        }

        if (cfg.firmware || cfg.shutdown) && !cfg.entry.is_empty() {
            let _ = write!(out, "\n");
        }

        if cfg.firmware {
            draw_item(out, row == selected, "UEFI Firmware Settings");
            row += 1;
        }
        if cfg.shutdown {
            draw_item(out, row == selected, "Shutdown");
            #[allow(unused_assignments)]
            {
                row += 1;
            }
        }

        let _ = out.set_color(Color::LightGray, Color::Black);
        let _ = write!(out, "\n");

        match timeout {
            Some(secs) => {
                let _ = write!(
                    out,
                    "  Auto boot in {}s...                              \n",
                    secs
                );
            }
            None => {
                let _ = write!(out, "                                                   \n");
            }
        }

        let _ = out.set_color(Color::DarkGray, Color::Black);
        let _ = write!(out, "\n  Up/Down to select, Enter to boot\n");
        let _ = out.set_color(Color::White, Color::Black);
    });
}

fn draw_item(out: &mut uefi::proto::console::text::Output, is_selected: bool, label: &str) {
    if is_selected {
        let _ = out.set_color(Color::White, Color::Blue);
        let _ = write!(out, "  > {:<66}\n", label);
        let _ = out.set_color(Color::White, Color::Black);
    } else {
        let _ = out.set_color(Color::LightGray, Color::Black);
        let _ = write!(out, "    {:<66}\n", label);
    }
}
