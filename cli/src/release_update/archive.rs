use super::{MAX_UNPACKED, Platform, Release, Result, file_hash, platform, valid_hash};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const WINDOWS: &[&str] = &[
    "petri.exe",
    "Petri.cmd",
    "Petri.ico",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "THIRD_PARTY_LICENSES.md",
    "README.txt",
    "petri-update.json",
];
const MAC: &[&str] = &[
    "Contents/Info.plist",
    "Contents/MacOS/Petri",
    "Contents/Resources/petri",
    "Contents/Resources/petri.command",
    "Contents/Resources/Petri.icns",
    "Contents/Resources/LICENSE",
    "Contents/Resources/THIRD_PARTY_NOTICES.md",
    "Contents/Resources/THIRD_PARTY_LICENSES.md",
    "Contents/Resources/petri-update.json",
    "Contents/_CodeSignature/CodeResources",
    "Contents/_CodeSignature/CodeDirectory",
    "Contents/_CodeSignature/CodeRequirements-1",
    "Contents/_CodeSignature/CodeSignature",
    "Contents/_CodeSignature/CodeRequirements",
];

pub(super) fn managed(platform: Platform, name: &str) -> bool {
    match platform {
        Platform::WindowsX64 => WINDOWS.contains(&name),
        _ => MAC.contains(&name),
    }
}

pub(super) fn executable(platform: Platform, name: &str) -> bool {
    name == platform.executable()
        || (platform != Platform::WindowsX64
            && matches!(
                name,
                "Contents/MacOS/Petri" | "Contents/Resources/petri.command"
            ))
}

fn package_files(platform: Platform) -> Vec<String> {
    match platform {
        Platform::WindowsX64 => WINDOWS
            .iter()
            .map(|s| s.to_string())
            .chain(["install-preview.ps1".into(), "SHA256SUMS".into()])
            .collect(),
        _ => MAC
            .iter()
            .map(|s| format!("Petri.app/{s}"))
            .chain([
                "install-preview.sh".into(),
                "README.txt".into(),
                "SHA256SUMS".into(),
            ])
            .collect(),
    }
}

/// Bound allocation BEFORE the ZIP parser reads the central directory. Current
/// packages are small, single-disk ZIPs; ZIP64 and extension ambiguity fail closed.
fn bound_directory(file: &mut File) -> Result<usize> {
    let length = file
        .metadata()
        .map_err(|_| "Could not inspect the update archive.")?
        .len();
    let tail_size = length.min(65_557);
    file.seek(SeekFrom::End(-(tail_size as i64)))
        .map_err(|_| "Invalid update archive.")?;
    let mut tail = vec![0; tail_size as usize];
    file.read_exact(&mut tail)
        .map_err(|_| "Incomplete update archive.")?;
    let index = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            tail[i..i + 4] == *b"PK\x05\x06"
                && i + 22 + u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize == tail.len()
        })
        .ok_or("Invalid update archive directory.")?;
    let u16_at = |i| u16::from_le_bytes([tail[index + i], tail[index + i + 1]]);
    let u32_at = |i| {
        u32::from_le_bytes(
            tail[index + i..index + i + 4]
                .try_into()
                .expect("four bytes"),
        )
    };
    let count = u16_at(10);
    let directory_size = u32_at(12) as u64;
    let directory_offset = u32_at(16) as u64;
    if u16_at(4) != 0
        || u16_at(6) != 0
        || u16_at(8) != count
        || count == 0
        || count > 64
        || directory_size > 32_768
        || directory_offset + directory_size != length - tail_size + index as u64
    {
        return Err("Unsupported or oversized update archive directory.".into());
    }
    file.rewind()
        .map_err(|_| "Could not read the update archive.")?;
    Ok(count as usize)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateMetadata {
    protocol: u32,
    channel: String,
    version: String,
    platform: Platform,
}

pub(super) fn extract(
    archive_path: &Path,
    destination: &Path,
    release: &Release,
) -> Result<BTreeMap<String, String>> {
    let mut input = File::open(archive_path).map_err(|_| "Could not open the verified update.")?;
    let entry_count = bound_directory(&mut input)?;
    let mut zip =
        zip::ZipArchive::new(input).map_err(|_| "Could not read the verified update ZIP.")?;
    if zip.len() != entry_count
        || zip.len() > 64
        || zip
            .has_overlapping_files()
            .map_err(|_| "Invalid ZIP entries.")?
    {
        return Err("The update archive has too many or overlapping entries.".into());
    }
    let files = package_files(release.platform);
    let prefix = format!("{}/", release.platform.stem());
    let allowed: BTreeSet<String> = files.iter().map(|file| format!("{prefix}{file}")).collect();
    let directories: BTreeSet<String> = allowed
        .iter()
        .flat_map(|name| {
            name.match_indices('/')
                .map(|(index, _)| name[..=index].to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut hashes = BTreeMap::new();
    let mut unpacked = 0u64;
    fs::create_dir(destination).map_err(|_| "Could not create the update package directory.")?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|_| "Invalid update ZIP entry.")?;
        let name = entry.name().to_string();
        if !seen.insert(name.clone())
            || entry.encrypted()
            || entry.is_symlink()
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err(
                "The update contains a duplicate, linked, encrypted, or special file.".into(),
            );
        }
        if entry.is_dir() {
            if !directories.contains(&name) {
                return Err("The update contains an unexpected directory.".into());
            }
            continue;
        }
        if !allowed.contains(&name) {
            return Err("The update contains an unexpected or unsafe file path.".into());
        }
        unpacked = unpacked
            .checked_add(entry.size())
            .ok_or("Invalid update size.")?;
        if unpacked > MAX_UNPACKED {
            return Err("The expanded update exceeds its size limit.".into());
        }
        let relative = name.strip_prefix(&prefix).ok_or("Invalid package root.")?;
        let path = destination.join(relative);
        fs::create_dir_all(path.parent().ok_or("Invalid package file.")?)
            .map_err(|_| "Could not stage an update directory.")?;
        let mut output = File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "Could not stage an update file.")?;
        let expected_size = entry.size();
        let copied = std::io::copy(&mut entry.by_ref().take(expected_size + 1), &mut output)
            .map_err(|_| "The update ZIP is corrupt or the disk is full.")?;
        if copied != expected_size {
            return Err("The expanded file size does not match the update.".into());
        }
        output
            .sync_all()
            .map_err(|_| "Could not save an update file.")?;
        hashes.insert(relative.to_string(), file_hash(&path)?);
    }
    let sums = hashes
        .remove("SHA256SUMS")
        .ok_or("The update has no file checksum manifest.")?;
    if !valid_hash(&sums) {
        return Err("Invalid file checksum manifest.".into());
    }
    let manifest = String::from_utf8(super::read_bounded(
        File::open(destination.join("SHA256SUMS"))
            .map_err(|_| "Could not read update checksums.")?,
        16_384,
    )?)
    .map_err(|_| "Invalid update checksum text.")?;
    let mut declared = BTreeMap::new();
    for line in manifest.lines() {
        let (hash, name) = line
            .split_once("  ")
            .ok_or("Invalid update checksum entry.")?;
        if !valid_hash(hash) || declared.insert(name.to_owned(), hash.to_owned()).is_some() {
            return Err("Duplicate or invalid update checksum.".into());
        }
    }
    if hashes != declared {
        return Err("The update's files do not match its checksum manifest.".into());
    }
    let package_root = if release.platform == Platform::WindowsX64 {
        destination.to_path_buf()
    } else {
        destination.join("Petri.app")
    };
    let metadata_path = if release.platform == Platform::WindowsX64 {
        "petri-update.json"
    } else {
        "Contents/Resources/petri-update.json"
    };
    let metadata: UpdateMetadata = serde_json::from_slice(&super::read_bounded(
        File::open(package_root.join(metadata_path))
            .map_err(|_| "This release does not support standalone updates.")?,
        1024,
    )?)
    .map_err(|_| "Invalid standalone update metadata.")?;
    if metadata.protocol != 1
        || metadata.channel != "preview"
        || metadata.version != release.version
        || metadata.platform != release.platform
    {
        return Err(
            "The update package has the wrong version, platform, or update channel.".into(),
        );
    }
    let mut managed_hashes = BTreeMap::new();
    for (name, hash) in hashes {
        let name = if release.platform == Platform::WindowsX64 {
            Some(name.as_str())
        } else {
            name.strip_prefix("Petri.app/")
        };
        if let Some(name) = name.filter(|name| managed(release.platform, name)) {
            platform::set_executable(&package_root.join(name), executable(release.platform, name))?;
            managed_hashes.insert(name.to_string(), hash);
        }
    }
    for name in [release.platform.executable(), metadata_path] {
        if !managed_hashes.contains_key(name) {
            return Err("The update is missing a required application file.".into());
        }
    }
    if release.platform == Platform::WindowsX64
        && WINDOWS
            .iter()
            .any(|name| !managed_hashes.contains_key(*name))
    {
        return Err("The Windows update is missing a required application file.".into());
    }
    if release.platform != Platform::WindowsX64 {
        for name in [
            "Contents/MacOS/Petri",
            "Contents/Resources/petri.command",
            "Contents/Info.plist",
            "Contents/_CodeSignature/CodeResources",
        ] {
            if !managed_hashes.contains_key(name) {
                return Err("The Mac update is missing a required app file.".into());
            }
        }
    }
    Ok(managed_hashes)
}
