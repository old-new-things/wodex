//! Bounded reader for our own template, not a general NSP/NCA installer.
//! Format structures follow the public libnx NCM ABI and Switch NCA/PFS0
//! layouts. Only one application, three plaintext NCAs and two CNMT records
//! are accepted. No ticket, certificate, production key or remote package input.
use super::{MAX_TEMPLATE_SIZE, TITLE_ID};
use alloc::{string::String, vec::Vec};
use core::ops::Range;
use sha2::{Digest, Sha256};

const HEADER: usize = 0xc00;
const NCA_TYPES: [u8; 3] = [0, 2, 1];
const CONTENT_TYPES: [u8; 3] = [1, 3, 0];

struct MetaLayout {
  cnmt: Range<usize>,
  pfs: Range<usize>,
  hashes: Range<usize>,
  block_size: usize,
}

pub(super) struct Package {
  bytes: Vec<u8>,
  ranges: [Range<usize>; 3],
  layout: MetaLayout,
  ids: [[u8; 16]; 3],
  database: [u8; 96],
  sealed: bool,
}

fn invalid() -> String {
  "The bundled HOME Menu shortcut failed validation. Install a current official Wodex build.".into()
}

fn number(bytes: &[u8], offset: usize, length: usize) -> Result<u64, String> {
  let range = within(offset, length, bytes.len())?;
  Ok(bytes[range].iter().enumerate().fold(0, |value, (i, byte)| value | (*byte as u64) << (i * 8)))
}

fn within(start: usize, size: usize, total: usize) -> Result<Range<usize>, String> {
  match start.checked_add(size) {
    Some(end) if end <= total => Ok(start..end),
    _ => Err(invalid()),
  }
}

fn put(bytes: &mut [u8], offset: usize, length: usize, value: u64) {
  for i in 0..length {
    bytes[offset + i] = (value >> (i * 8)) as u8;
  }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
  Sha256::digest(bytes).into()
}

impl Package {
  pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
    if bytes.len() < 20 || bytes.len() > MAX_TEMPLATE_SIZE || &bytes[..8] != b"WDXHOME1" {
      return Err(invalid());
    }
    let mut ranges = [0..0, 0..0, 0..0];
    let mut cursor = 20;
    for (index, range) in ranges.iter_mut().enumerate() {
      let size = number(&bytes, 8 + index * 4, 4)? as usize;
      *range = within(cursor, size, bytes.len())?;
      let nca = &bytes[range.clone()];
      Self::validate_nca(nca, index)?;
      cursor = range.end;
    }
    if cursor != bytes.len() {
      return Err(invalid());
    }
    let layout = Self::meta_layout(&bytes, &ranges)?;
    Ok(Self { bytes, ranges, layout, ids: [[0; 16]; 3], database: [0; 96], sealed: false })
  }

  fn validate_nca(nca: &[u8], index: usize) -> Result<(), String> {
    if nca.len() < HEADER
      || !nca.len().is_multiple_of(0x200)
      || &nca[0x200..0x204] != b"NCA3"
      || nca[0x205] != NCA_TYPES[index]
      || nca[0x204] != 0
      || number(nca, 0x208, 8)? != nca.len() as u64
      || number(nca, 0x210, 8)? != TITLE_ID
      || nca[0x230..0x240].iter().any(|byte| *byte != 0)
      || nca[0x300..0x400].iter().any(|byte| *byte != 0)
    {
      return Err(invalid());
    }
    let mut previous_end = HEADER;
    let mut sections = 0;
    for slot in 0..4 {
      let start = number(nca, 0x240 + slot * 16, 4)? as usize * 0x200;
      let end = number(nca, 0x244 + slot * 16, 4)? as usize * 0x200;
      if start == 0 && end == 0 {
        continue;
      }
      if start < previous_end || end <= start || end > nca.len() {
        return Err(invalid());
      }
      let fs = 0x400 + slot * 0x200;
      if nca[fs + 4] != 1
        || digest(&nca[fs..fs + 0x200]) != nca[0x280 + slot * 32..0x2a0 + slot * 32]
      {
        return Err(invalid());
      }
      previous_end = end;
      sections += 1;
    }
    if sections == 0 || (index == 2 && sections != 1) {
      return Err(invalid());
    }
    Ok(())
  }

  fn meta_layout(bytes: &[u8], ranges: &[Range<usize>; 3]) -> Result<MetaLayout, String> {
    let base = ranges[2].start;
    let nca = &bytes[ranges[2].clone()];
    // Filesystem header: PFS0 / hierarchical SHA-256 / plaintext section.
    if nca[0x402] != 1 || nca[0x403] != 2 {
      return Err(invalid());
    }
    let section = number(nca, 0x240, 4)? as usize * 0x200;
    let end = number(nca, 0x244, 4)? as usize * 0x200;
    let section_size = end.checked_sub(section).ok_or_else(invalid)?;
    let block_size = number(nca, 0x428, 4)? as usize;
    let layers = number(nca, 0x42c, 4)?;
    let table_offset = usize::try_from(number(nca, 0x430, 8)?).map_err(|_| invalid())?;
    let table_size = usize::try_from(number(nca, 0x438, 8)?).map_err(|_| invalid())?;
    let data_offset = usize::try_from(number(nca, 0x440, 8)?).map_err(|_| invalid())?;
    let data_size = usize::try_from(number(nca, 0x448, 8)?).map_err(|_| invalid())?;
    let table = within(table_offset, table_size, section_size)?;
    let data = within(data_offset, data_size, section_size)?;
    if !(0x200..=1024 * 1024).contains(&block_size)
      || !block_size.is_power_of_two()
      || layers != 2
      || table.end > data.start
      || data_size < 40
      || table_size != data_size.div_ceil(block_size) * 32
    {
      return Err(invalid());
    }
    let hashes = base + section + table.start..base + section + table.end;
    let pfs = base + section + data.start..base + section + data.end;
    if digest(&bytes[hashes.clone()]) != nca[0x408..0x428] {
      return Err(invalid());
    }
    for (chunk, expected) in
      bytes[pfs.clone()].chunks(block_size).zip(bytes[hashes.clone()].as_chunks::<32>().0)
    {
      if digest(chunk) != *expected {
        return Err(invalid());
      }
    }
    let pfs_bytes = &bytes[pfs.clone()];
    if &pfs_bytes[..4] != b"PFS0" || number(pfs_bytes, 4, 4)? != 1 {
      return Err(invalid());
    }
    let strings_size = number(pfs_bytes, 8, 4)? as usize;
    let data_start = 40usize.checked_add(strings_size).ok_or_else(invalid)?;
    let file_offset = usize::try_from(number(pfs_bytes, 16, 8)?).map_err(|_| invalid())?;
    let file_size = usize::try_from(number(pfs_bytes, 24, 8)?).map_err(|_| invalid())?;
    let name_offset = number(pfs_bytes, 32, 4)? as usize;
    let strings = within(40, strings_size, pfs_bytes.len())?;
    if name_offset >= strings_size || file_size != 192 || file_offset != 0 {
      return Err(invalid());
    }
    let name = &pfs_bytes[strings.start + name_offset..strings.end];
    let name_end = name.iter().position(|byte| *byte == 0).ok_or_else(invalid)?;
    if !name[..name_end].ends_with(b".cnmt") || name[..name_end].contains(&b'/') {
      return Err(invalid());
    }
    let relative =
      within(data_start.checked_add(file_offset).ok_or_else(invalid)?, file_size, data_size)?;
    let cnmt = pfs.start + relative.start..pfs.start + relative.end;
    let content = &bytes[cnmt.clone()];
    if number(content, 0, 8)? != TITLE_ID
      || number(content, 8, 4)? != 0
      || content[12] != 0x80
      || number(content, 14, 2)? != 16
      || number(content, 16, 2)? != 2
      || number(content, 18, 2)? != 0
      || number(content, 32, 8)? != TITLE_ID + 0x800
      || number(content, 40, 8)? != 0
      // The key-free template may use an empty digest placeholder; sealing
      // writes the final digest after replacing both content records.
      || (content[160..192].iter().any(|byte| *byte != 0)
        && digest(&content[..160]) != content[160..192])
    {
      return Err(invalid());
    }
    for (index, &content_type) in CONTENT_TYPES.iter().take(2).enumerate() {
      let record = &content[48 + index * 56..104 + index * 56];
      if number(record, 48, 6)? != ranges[index].len() as u64
        || record[54] != content_type
        || record[55] != 0
      {
        return Err(invalid());
      }
    }
    Ok(MetaLayout { cnmt, pfs, hashes, block_size })
  }

  pub fn seal(
    &mut self,
    mut seal_header: impl FnMut(&mut [u8]) -> Result<(), String>,
  ) -> Result<(), String> {
    if self.sealed {
      return Err("The HOME Menu template has already been prepared.".into());
    }
    for (index, &content_type) in CONTENT_TYPES.iter().take(2).enumerate() {
      let range = self.ranges[index].clone();
      seal_header(&mut self.bytes[range.start..range.start + HEADER])?;
      let hash = digest(&self.bytes[range.clone()]);
      self.ids[index].copy_from_slice(&hash[..16]);
      let record = self.layout.cnmt.start + 48 + index * 56;
      self.bytes[record..record + 32].copy_from_slice(&hash);
      self.bytes[record + 32..record + 48].copy_from_slice(&hash[..16]);
      put(&mut self.bytes, record + 48, 6, range.len() as u64);
      self.bytes[record + 54] = content_type;
      self.bytes[record + 55] = 0;
    }
    let cnmt = self.layout.cnmt.start;
    let hash = digest(&self.bytes[cnmt..cnmt + 160]);
    self.bytes[cnmt + 160..cnmt + 192].copy_from_slice(&hash);
    let pfs = self.layout.pfs.clone();
    for (index, start) in (pfs.start..pfs.end).step_by(self.layout.block_size).enumerate() {
      let hash = digest(&self.bytes[start..(start + self.layout.block_size).min(pfs.end)]);
      let out = self.layout.hashes.start + index * 32;
      self.bytes[out..out + 32].copy_from_slice(&hash);
    }
    let meta = self.ranges[2].start;
    let master = digest(&self.bytes[self.layout.hashes.clone()]);
    self.bytes[meta + 0x408..meta + 0x428].copy_from_slice(&master);
    let fs_hash = digest(&self.bytes[meta + 0x400..meta + 0x600]);
    self.bytes[meta + 0x280..meta + 0x2a0].copy_from_slice(&fs_hash);
    seal_header(&mut self.bytes[meta..meta + HEADER])?;
    let hash = digest(&self.bytes[self.ranges[2].clone()]);
    self.ids[2].copy_from_slice(&hash[..16]);

    // NCM database: 8-byte header, 16-byte application extension, three
    // 24-byte content descriptors. Meta content must be included here.
    put(&mut self.database, 0, 2, 16);
    put(&mut self.database, 2, 2, 3);
    put(&mut self.database, 8, 8, TITLE_ID + 0x800);
    for (slot, index) in [2, 0, 1].into_iter().enumerate() {
      let offset = 24 + slot * 24;
      self.database[offset..offset + 16].copy_from_slice(&self.ids[index]);
      put(&mut self.database, offset + 16, 5, self.ranges[index].len() as u64);
      self.database[offset + 22] = CONTENT_TYPES[index];
    }
    self.sealed = true;
    Ok(())
  }

  pub fn size(&self) -> usize {
    self.bytes.len() - 20
  }
  pub fn contents(&self) -> impl Iterator<Item = &[u8]> {
    self.ranges.iter().map(|range| &self.bytes[range.clone()])
  }
  pub fn ids(&self) -> &[[u8; 16]; 3] {
    &self.ids
  }
  pub fn database(&self) -> &[u8; 96] {
    &self.database
  }
}
