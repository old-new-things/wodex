//! Executable inspection and rollback policy. The filesystem adapter only
//! translates individual native system calls; every ordering decision is here.
use alloc::{ffi::CString, format, string::String};
use core::ffi::c_char;

use crate::{
  backend::storage::{File, Fs},
  update::{self, WodexSha256},
};

use super::PendingUpdate;

pub(super) fn backup_path(target: &str) -> String {
  format!("{}.previous.nro", &target[..target.len() - 4])
}

pub(super) fn validate_target_path(target: &str) -> Result<(), String> {
  let start = if target.starts_with("sdmc:/") {
    6
  } else if target.starts_with('/') {
    1
  } else {
    return Err("The running NRO path is unknown. Install updates manually.".into());
  };
  if target.len() > 4096
    || !target.ends_with(".nro")
    || target.ends_with(".previous.nro")
    || target.contains(['\0', '\\'])
    || target[start..].contains(':')
    || target.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
  {
    return Err("The running NRO path cannot be safely replaced.".into());
  }
  if target[start..].split('/').any(|part| matches!(part, "" | "." | "..")) {
    return Err("The update path contains unsupported traversal.".into());
  }
  Ok(())
}

fn validate_pending(pending: &PendingUpdate) -> Result<(), String> {
  validate_target_path(&pending.target_path)?;
  if pending.staged_path != format!("{}.update.part", pending.target_path)
    || pending.backup_path != backup_path(&pending.target_path)
    || !(0x80..=update::MAX_NRO_BYTES).contains(&pending.size)
    || pending.sha256.len() != 64
    || !pending.sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
  {
    return Err("The pending update has invalid local paths or integrity metadata.".into());
  }
  Ok(())
}

fn little32(bytes: &[u8], offset: usize) -> u32 {
  u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn little64(bytes: &[u8], offset: usize) -> u64 {
  u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_exact(file: &mut File, bytes: &mut [u8]) -> Result<(), String> {
  let mut offset = 0;
  while offset < bytes.len() {
    let count = file.read(&mut bytes[offset..])?;
    if count == 0 {
      return Err("Cannot read the complete NRO header.".into());
    }
    offset += count;
  }
  Ok(())
}

fn validate_romfs(file: &mut File, offset: u64, size: u64) -> Result<(), String> {
  const NO_ENTRY: u32 = u32::MAX;
  let mut header = [0u8; 80];
  if size < header.len() as u64 || file.seek(offset).is_err() {
    return Err("The update has a truncated RomFS resource header.".into());
  }
  read_exact(file, &mut header)?;
  if little64(&header, 0) != header.len() as u64 {
    return Err("The update has an invalid RomFS resource header.".into());
  }
  let mut tables = [(0u64, 0u64); 4];
  for index in 0..tables.len() {
    let table_offset = little64(&header, 8 + index * 16);
    let table_size = little64(&header, 16 + index * 16);
    if table_offset < header.len() as u64
      || table_offset > size
      || table_size > size - table_offset
      || !table_offset.is_multiple_of(4)
      || !table_size.is_multiple_of(4)
      || ((index == 0 || index == 2) && table_size < 4)
      || (index == 1 && table_size < 24)
    {
      return Err("The update has RomFS metadata outside its resource section.".into());
    }
    for &(prior_offset, prior_size) in &tables[..index] {
      if table_size != 0
        && prior_size != 0
        && table_offset < prior_offset + prior_size
        && prior_offset < table_offset + table_size
      {
        return Err("The update has overlapping RomFS metadata tables.".into());
      }
    }
    tables[index] = (table_offset, table_size);
  }
  let data_offset = little64(&header, 72);
  if data_offset < header.len() as u64 || data_offset >= size {
    return Err("The update has an invalid RomFS file-data offset.".into());
  }
  if tables
    .iter()
    .any(|&(start, length)| length != 0 && data_offset >= start && data_offset < start + length)
  {
    return Err("The update's RomFS file-data offset overlaps metadata.".into());
  }
  let (directory_offset, directory_size) = tables[1];
  let (file_offset, file_size) = tables[3];
  let mut root = [0u8; 24];
  file
    .seek(offset + directory_offset)
    .map_err(|_| String::from("Cannot read the update's RomFS root."))?;
  read_exact(file, &mut root)?;
  let directory_reference = |entry: u32| {
    entry == NO_ENTRY || (entry.is_multiple_of(4) && u64::from(entry) <= directory_size - 24)
  };
  let child_directory = little32(&root, 8);
  let child_file = little32(&root, 12);
  if little32(&root, 0) != 0
    || little32(&root, 4) != NO_ENTRY
    || little32(&root, 20) != 0
    || !directory_reference(child_directory)
    || child_directory == 0
    || !directory_reference(little32(&root, 16))
    || (child_file != NO_ENTRY
      && (!child_file.is_multiple_of(4)
        || file_size < 32
        || u64::from(child_file) > file_size - 32))
  {
    return Err("The update has an invalid RomFS root directory.".into());
  }
  if child_file != NO_ENTRY {
    let mut entry = [0u8; 32];
    file
      .seek(offset + file_offset + u64::from(child_file))
      .map_err(|_| String::from("Cannot read the update's RomFS file metadata."))?;
    read_exact(file, &mut entry)?;
    let contents_offset = little64(&entry, 8);
    let contents_size = little64(&entry, 16);
    if little32(&entry, 0) != 0
      || u64::from(little32(&entry, 28)) > file_size - u64::from(child_file) - entry.len() as u64
      || contents_offset > size - data_offset
      || contents_size > size - data_offset - contents_offset
    {
      return Err("The update has a RomFS root file outside its resource section.".into());
    }
  }
  Ok(())
}

fn validate_nro(file: &mut File, size: u64, require_resources: bool) -> Result<(), String> {
  if !(0x80..=update::MAX_NRO_BYTES).contains(&size) || file.seek(0).is_err() {
    return Err("The update is not a supported NRO executable.".into());
  }
  let mut header = [0u8; 256];
  let header_size = core::cmp::min(header.len() as u64, size) as usize;
  read_exact(file, &mut header[..header_size])?;
  if !update::validate_nro(&header[..header_size], size) {
    return Err("The update has an invalid NRO executable header.".into());
  }
  let code_size = u64::from(little32(&header, 0x18));
  if size == code_size {
    return if require_resources {
      Err("The update is missing its packaged RomFS resources.".into())
    } else {
      Ok(())
    };
  }
  let mut assets = [0u8; 56];
  if size - code_size < assets.len() as u64 || file.seek(code_size).is_err() {
    return Err("The update has a truncated NRO asset header.".into());
  }
  read_exact(file, &mut assets)?;
  if &assets[..4] != b"ASET" || little32(&assets, 4) != 0 {
    return Err("The update has an invalid NRO asset header.".into());
  }
  let mut ranges = [(0u64, 0u64); 3];
  for index in 0..ranges.len() {
    let offset = little64(&assets, 8 + index * 16);
    let length = little64(&assets, 16 + index * 16);
    if length != 0
      && (offset < assets.len() as u64
        || offset > size - code_size
        || length > size - code_size - offset)
    {
      return Err("The update contains assets outside its NRO file.".into());
    }
    for &(prior_offset, prior_length) in &ranges[..index] {
      if length != 0
        && prior_length != 0
        && offset < prior_offset + prior_length
        && prior_offset < offset + length
      {
        return Err("The update contains overlapping NRO assets.".into());
      }
    }
    ranges[index] = (offset, length);
  }
  if require_resources {
    let (romfs_offset, romfs_size) = ranges[2];
    if romfs_size == 0 {
      return Err("The update is missing its packaged RomFS resources.".into());
    }
    validate_romfs(file, code_size + romfs_offset, romfs_size)?;
  }
  Ok(())
}

pub(super) fn candidate(target: &str, sha256: &str, size: u64) -> Result<PendingUpdate, String> {
  validate_target_path(target)?;
  let pending = PendingUpdate {
    target_path: target.into(),
    staged_path: format!("{target}.update.part"),
    backup_path: backup_path(target),
    sha256: sha256.into(),
    size,
  };
  validate_pending(&pending)?;
  Ok(pending)
}

pub(super) fn verify_stage(fs: &Fs, pending: &PendingUpdate) -> Result<(), String> {
  let mut file = fs.open_read(&pending.staged_path)?;
  let size = file.size()?;
  if size != pending.size {
    return Err("The downloaded update size changed. Download it again.".into());
  }
  let mut hash = WodexSha256::new();
  let mut buffer = [0u8; 65_536];
  loop {
    let count =
      file.read(&mut buffer).map_err(|_| String::from("Cannot verify the downloaded update."))?;
    if count == 0 {
      break;
    }
    if !hash.update(&buffer[..count]) {
      return Err("Cannot verify the update checksum.".into());
    }
  }
  if hash.finish() != pending.sha256 {
    return Err("The update checksum does not match the published release.".into());
  }
  validate_nro(&mut file, size, true)
}

pub fn install_update(fs: &Fs, pending: &PendingUpdate) -> Result<(), String> {
  install_with(fs, pending, |from, to| fs.rename(from, to))
}

/// Deterministic rename fault injection for the native integration tests. This
/// seam changes only the syscall, leaving validation and rollback policy intact.
pub fn install_update_with_rename(
  fs: &Fs,
  pending: &PendingUpdate,
  rename_file: unsafe extern "C" fn(*const c_char, *const c_char) -> i32,
) -> Result<(), String> {
  install_with(fs, pending, |from, to| {
    let from = CString::new(from).map_err(|_| String::from("Invalid update path."))?;
    let to = CString::new(to).map_err(|_| String::from("Invalid update path."))?;
    if unsafe { rename_file(from.as_ptr(), to.as_ptr()) } == 0 {
      Ok(())
    } else {
      Err("Cannot rename the update file.".into())
    }
  })
}

fn install_with(
  fs: &Fs,
  pending: &PendingUpdate,
  mut rename_file: impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
  validate_pending(pending)?;
  let current = fs.stat(&pending.target_path, false)?;
  let staged = fs.stat(&pending.staged_path, false)?;
  if !current.is_some_and(|info| info.is_regular()) || !staged.is_some_and(|info| info.is_regular())
  {
    return Err(
      "The current or staged NRO is missing or is not a regular file. Nothing was replaced.".into(),
    );
  }
  // Verify both executables before touching the last usable backup. In
  // particular, retrying after a failed rollback must preserve that backup.
  verify_stage(fs, pending)?;
  {
    let mut installed = fs.open_read(&pending.target_path)?;
    let size = installed.size()?;
    validate_nro(&mut installed, size, false)?;
  }
  if let Some(backup) = fs.stat(&pending.backup_path, false)? {
    if !backup.is_regular() {
      return Err(
        "The previous-version backup path is not a regular file. Nothing was replaced.".into(),
      );
    }
    fs.remove(&pending.backup_path, false).map_err(|_| {
      String::from("Cannot rotate the previous-version backup. Nothing was replaced.")
    })?;
  }
  rename_file(&pending.target_path, &pending.backup_path).map_err(|_| {
    String::from("Cannot back up the running NRO. The installed version was not replaced.")
  })?;
  if rename_file(&pending.staged_path, &pending.target_path).is_err() {
    if rename_file(&pending.backup_path, &pending.target_path).is_err() {
      return Err("Installation failed. Your previous version is preserved in .previous.nro; launch it manually.".into());
    }
    return Err("Installation failed. The previous installed version was restored.".into());
  }
  fs.sync_parent(&pending.target_path)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn replacement_paths_bind_the_stage_and_backup_to_the_running_executable() {
    for target in ["/wodex.nro", "/switch/wodex/wodex.nro", "sdmc:/switch/wodex/wodex.nro"] {
      let pending = candidate(target, &"a".repeat(64), 4096).unwrap();
      validate_pending(&pending).unwrap();
      for invalid in [
        PendingUpdate { staged_path: "/unrelated.nro".into(), ..pending.clone() },
        PendingUpdate { backup_path: "/unrelated.previous.nro".into(), ..pending.clone() },
        PendingUpdate { sha256: "A".repeat(64), ..pending.clone() },
        PendingUpdate { size: 127, ..pending.clone() },
      ] {
        assert!(validate_pending(&invalid).is_err());
      }
    }
    for target in [
      "",
      "wodex.nro",
      "../wodex.nro",
      "/switch/../wodex.nro",
      "/switch//wodex.nro",
      "/switch/./wodex.nro",
      "/switch/wodex.previous.nro",
      "/switch/wodex.nro\0",
      "/switch/wodex:\\bad.nro",
      "sdmc:/switch/../wodex.nro",
      "sdmc:/switch/with\nline.nro",
    ] {
      assert!(validate_target_path(target).is_err(), "{target:?}");
    }
  }
}
