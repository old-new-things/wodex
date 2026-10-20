"""Validate the shipped forwarder and reject malformed build inputs offline."""
import importlib.util
from pathlib import Path
import struct
import unittest

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("home_menu_template", ROOT / "scripts/build-home-menu.py")
TEMPLATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TEMPLATE)


class TemplateTests(unittest.TestCase):
  @classmethod
  def setUpClass(cls):
    cls.bundle = (ROOT / "romfs/home-menu/template.bin").read_bytes()
    cls.parts = TEMPLATE.read_bundle(cls.bundle)

  def program_files(self):
    program = self.parts[0]
    section = struct.unpack_from("<I", program, 0x240)[0] * 512
    offset, size = struct.unpack_from("<QQ", program, 0x440)
    return TEMPLATE.pfs_entries(program[section + offset:section + offset + size])

  def test_committed_template_matches_sources_manifest_and_licenses(self):
    TEMPLATE.check()
    self.assertEqual(len(TEMPLATE.inspect_parts(self.parts)), 3)

  def test_bundle_boundaries_are_exact(self):
    for data in (self.bundle[:19], self.bundle[:-1], self.bundle + b"\0",
                 b"OTHERHDR" + self.bundle[8:]):
      with self.subTest(length=len(data)):
        with self.assertRaises(ValueError):
          TEMPLATE.read_bundle(data)
    invalid = bytearray(self.bundle)
    struct.pack_into("<I", invalid, 8, 0xffffffff)
    with self.assertRaises(ValueError):
      TEMPLATE.read_bundle(invalid)

  def test_wrong_title_encryption_and_corrupt_payload_are_rejected(self):
    mutations = [(0, 0x210), (1, 0x404)]
    for part in (0, 2):
      section = struct.unpack_from("<I", self.parts[part], 0x240)[0] * 512
      files = struct.unpack_from("<Q", self.parts[part], 0x440)[0]
      mutations.append((part, section + files + 16))
    for part, offset in mutations:
      with self.subTest(part=part, offset=offset):
        parts = list(self.parts)
        changed = bytearray(parts[part])
        changed[offset] ^= 1
        parts[part] = bytes(changed)
        with self.assertRaises(ValueError):
          TEMPLATE.inspect_parts(parts)

  def test_nso_hashes_and_npdm_identity_are_verified(self):
    files = self.program_files()
    TEMPLATE.inspect_program(files["main"], files["main.npdm"])
    nso = bytearray(files["main"])
    nso[0xa0] ^= 1
    with self.assertRaisesRegex(ValueError, "segment hash"):
      TEMPLATE.inspect_program(nso, files["main.npdm"])
    for section, identity_offset in ((0x70, 0x10), (0x78, 0x210), (0x78, 0x218)):
      npdm = bytearray(files["main.npdm"])
      offset = struct.unpack_from("<I", npdm, section)[0]
      npdm[offset + identity_offset] ^= 1
      with self.subTest(section=section, identity_offset=identity_offset):
        with self.assertRaises(ValueError):
          TEMPLATE.inspect_program(files["main"], npdm)

  def test_pfs_rejects_truncated_and_out_of_range_records(self):
    valid = b"PFS0" + struct.pack("<III", 1, 5, 0)
    valid += struct.pack("<QQII", 0, 1, 0, 0) + b"file\0" + b"x"
    self.assertEqual(TEMPLATE.pfs_entries(valid), {"file": b"x"})
    for length in (0, 15, 39, 44, 45):
      with self.subTest(length=length):
        with self.assertRaises(ValueError):
          TEMPLATE.pfs_entries(valid[:length])
    invalid = bytearray(valid)
    struct.pack_into("<I", invalid, 32, 5)
    with self.assertRaises(ValueError):
      TEMPLATE.pfs_entries(invalid)

  def test_lz4_matches_can_overlap_without_exceeding_segment_bounds(self):
    self.assertEqual(TEMPLATE.lz4_block(b"\x14a\x01\0", 9), b"a" * 9)
    self.assertEqual(TEMPLATE.lz4_block(b"\x50hello", 5), b"hello")
    for data, size in ((b"\xf0", 15), (b"\x10", 1), (b"\0\0\0", 4),
                       (b"\x14a\x02\0", 9), (b"\x14a\x01\0", 8),
                       (b"", 4 * 1024 * 1024 + 1)):
      with self.subTest(data=data, size=size):
        with self.assertRaises(ValueError):
          TEMPLATE.lz4_block(data, size)


if __name__ == "__main__":
  unittest.main()
