use core::fmt::Write;
use core::time::Duration;

use uefi::proto::console::text::{Color, Key, ScanCode};

use crate::config::{DEFAULT_ENTRY, ENTRIES, TIMEOUT_SECS};

/// Display the boot menu and return the index of the selected entry.
pub fn show() -> usize {
    let mut selected = DEFAULT_ENTRY;
    let mut timeout: Option<usize> = Some(TIMEOUT_SECS);
    let mut tick_count: usize = 0;

    uefi::system::with_stdout(|out| {
        let _ = out.clear();
        let _ = out.enable_cursor(false);
    });

    draw(selected, timeout);

    loop {
        uefi::boot::stall(Duration::from_millis(100));

        let key = uefi::system::with_stdin(|stdin| stdin.read_key());

        if let Ok(Some(key)) = key {
            timeout = None;

            match key {
                Key::Special(ScanCode::UP) if selected > 0 => {
                    selected -= 1;
                }
                Key::Special(ScanCode::DOWN) if selected < ENTRIES.len() - 1 => {
                    selected += 1;
                }
                Key::Printable(c) if u16::from(c) == 0x000D => {
                    boot_selected(selected);
                    return selected;
                }
                _ => {}
            }

            draw(selected, timeout);
        }

        tick_count += 1;
        if tick_count >= 10 {
            tick_count = 0;
            if let Some(ref mut t) = timeout {
                if *t == 0 {
                    boot_selected(selected);
                    return selected;
                }
                *t -= 1;
                draw(selected, timeout);
            }
        }
    }
}

fn boot_selected(idx: usize) {
    uefi::system::with_stdout(|out| {
        let _ = out.set_color(Color::White, Color::Black);
        let _ = out.clear();
        let _ = write!(out, "Booting {}...\n", ENTRIES[idx].name);
    });
}

fn draw(selected: usize, timeout: Option<usize>) {
    uefi::system::with_stdout(|out| {
        let _ = out.set_cursor_position(0, 0);

        let _ = out.set_color(Color::White, Color::Black);
        let _ = write!(out, "\n");
        let _ = write!(out, "  Alpheratz Boot Loader\n");
        let _ = write!(out, "\n");

        for (i, entry) in ENTRIES.iter().enumerate() {
            if i == selected {
                let _ = out.set_color(Color::White, Color::Blue);
                let _ = write!(out, "  > {:<66}\n", entry.name);
                let _ = out.set_color(Color::White, Color::Black);
            } else {
                let _ = out.set_color(Color::LightGray, Color::Black);
                let _ = write!(out, "    {:<66}\n", entry.name);
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
