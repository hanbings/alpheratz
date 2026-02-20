#![allow(dead_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use uefi::boot::{self, LoadImageSource};
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{Directory, File, FileAttribute, FileInfo, FileMode, FileType};
use uefi::proto::media::fs::SimpleFileSystem;

use crate::config::Config;

pub fn open_esp_root() -> uefi::Result<Directory> {
    let loaded_image = boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle())?;
    let device = loaded_image
        .device()
        .ok_or_else(|| uefi::Error::from(Status::NOT_FOUND))?;

    let mut sfs = boot::open_protocol_exclusive::<SimpleFileSystem>(device)?;
    sfs.open_volume()
}

pub fn read_file(root: &mut Directory, path: &str) -> uefi::Result<Vec<u8>> {
    let path16 = uefi::CString16::try_from(path)
        .map_err(|_| uefi::Error::from(Status::INVALID_PARAMETER))?;

    let handle = root.open(path16.as_ref(), FileMode::Read, FileAttribute::empty())?;
    let mut file = handle
        .into_regular_file()
        .ok_or_else(|| uefi::Error::from(Status::INVALID_PARAMETER))?;

    let info = file.get_boxed_info::<FileInfo>()?;
    let size = info.file_size() as usize;
    let mut buf = Vec::with_capacity(size);
    buf.resize(size, 0);
    file.read(&mut buf)?;
    Ok(buf)
}

fn path_join(dir: &str, file: &str) -> String {
    if dir.ends_with('\\') {
        let mut s = String::from(dir);
        s.push_str(file);
        s
    } else {
        let mut s = String::from(dir);
        s.push('\\');
        s.push_str(file);
        s
    }
}

pub fn load_drivers_from_config(cfg: &Config) -> uefi::Result<()> {
    if cfg.drivers.is_empty() {
        return Ok(());
    }

    let mut root = open_esp_root()?;

    for p in &cfg.drivers {
        // Treat configured path as either a single driver .efi file or a directory containing drivers.
        let p16 = match uefi::CString16::try_from(p.as_str()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let handle = match root.open(p16.as_ref(), FileMode::Read, FileAttribute::empty()) {
            Ok(h) => h,
            Err(_) => continue,
        };

        match handle.into_type() {
            Ok(FileType::Dir(mut dir)) => {
                let _ = dir.reset_entry_readout();
                while let Ok(Some(info)) = dir.read_entry_boxed() {
                    if info.is_directory() {
                        continue;
                    }
                    let name = String::from(info.file_name());
                    // Skip "." and ".." if present.
                    if name == "." || name == ".." {
                        continue;
                    }
                    // Only attempt to load *.efi files.
                    let name_lc = name.to_ascii_lowercase();
                    if !name_lc.ends_with(".efi") {
                        continue;
                    }
                    let full = path_join(p, &name);
                    let _ = load_and_start_image(&mut root, &full);
                }
            }
            Ok(FileType::Regular(_)) => {
                let _ = load_and_start_image(&mut root, p);
            }
            Err(_) => {}
        }
    }

    Ok(())
}

pub fn load_and_start_image(root: &mut Directory, path: &str) -> uefi::Result<()> {
    let image = read_file(root, path)?;
    let h = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: &image,
            file_path: None,
        },
    )?;
    boot::start_image(h)?;
    Ok(())
}
