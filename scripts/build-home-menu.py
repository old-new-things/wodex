#!/usr/bin/env python3
"""Reproduce or verify Wodex's key-free HOME Menu forwarder template.

Normal builds use the committed template and --check, requiring only Python's
standard library. Regeneration uses the pinned devkitPro container and the
explicitly fetched hacBrewPack tool; neither mode reads any console key file.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
RESOURCES = ROOT / "romfs/home-menu"
DEPENDENCY = ROOT / "build/home-menu-deps/hacBrewPack"
TOOL_URL = "https://github.com/dragonflylee/hacBrewPack.git"
TOOL_REVISION = "745b16ecfc9ce055743067d200572204cb2aac6c"
LOADER_REVISION = "82b95122c5ae8dc059bf23893ba7623c72c86773"
TITLE_ID = 0x05776F6465780000
TARGET = "sdmc:/switch/wodex/wodex.nro"
MAGIC = b"WDXHOME1"
TITLE = "Wodex"
AUTHOR = "Wodex contributors"
FORWARDER_VERSION = "Forwarder 1"
TOOLCHAIN = "docker.io/devkitpro/devkita64:20260219"
SOURCES = {
  "external/nx-hbloader/source/main.c": "7f5328181d63f38ebb1bc4786cb9fbb48fa32fdea160abbb6a26013761f2976a",
  "external/nx-hbloader/source/trampoline.s": "2f11a79cc9a4a162dcef557274076b66480bb256f353d47cea1ef56ab8e11f4e",
  "external/nx-hbloader/hbl.json": "1efef8edb477e36ac9d1f60f53d7efcb03e0220276aaeb61cb67fa4c59ef4eba",
  "external/nx-hbloader/LICENSE.md": "db88a9c4d4d3148cbe397ca0acb7886e7aebcc8b984d4ce3b949e9caabdc7223",
  "assets/icon.jpg": "a206f1ab97e708ec8c31b604f1a75e08c6f5287bd1e4f46f87c01233988bb4cb",
}


def require(condition, message):
  if not condition:
    raise ValueError(message)


def digest(data):
  return hashlib.sha256(data).hexdigest()


def run(arguments, **kwargs):
  return subprocess.run([str(arg) for arg in arguments], check=True, **kwargs)


def u32(data, offset):
  return struct.unpack_from("<I", data, offset)[0]


def u64(data, offset):
  return struct.unpack_from("<Q", data, offset)[0]


def checked_slice(data, offset, length):
  require(offset <= len(data) and length <= len(data) - offset, "Invalid package range")
  return data[offset:offset + length]


def pfs_entries(data):
  require(len(data) >= 16 and data[:4] == b"PFS0", "Invalid PFS0 header")
  count, strings_size = struct.unpack_from("<II", data, 4)
  require(0 < count <= 16, "Unexpected PFS0 file count")
  table_end = 16 + count * 24
  strings = checked_slice(data, table_end, strings_size)
  base = table_end + strings_size
  result = {}
  for index in range(count):
    offset, length, name_offset, reserved = struct.unpack_from("<QQII", data, 16 + index * 24)
    require(reserved == 0 and name_offset < len(strings), "Invalid PFS0 file record")
    end = strings.find(b"\0", name_offset)
    require(end >= 0, "Unterminated PFS0 file name")
    name = strings[name_offset:end].decode("ascii")
    require(name not in result and "/" not in name, "Unexpected PFS0 file name")
    result[name] = checked_slice(data, base + offset, length)
  return result


def read_bundle(data):
  require(20 <= len(data) <= 4 * 1024 * 1024 and data[:8] == MAGIC, "Invalid WDXHOME1 bundle")
  sizes = struct.unpack_from("<III", data, 8)
  require(sum(sizes) == len(data) - 20, "Bundle lengths do not match")
  parts, offset = [], 20
  for size in sizes:
    parts.append(checked_slice(data, offset, size))
    offset += size
  return parts


def lz4_block(data, expected_size):
  """Decode the bounded raw LZ4 blocks written by devkitPro's elf2nso."""
  require(expected_size <= 4 * 1024 * 1024, "Oversized NSO segment")
  output, cursor = bytearray(), 0

  def length(base):
    nonlocal cursor
    if base == 15:
      while True:
        require(cursor < len(data), "Truncated LZ4 length")
        extra = data[cursor]
        cursor += 1
        base += extra
        if extra != 255:
          break
    return base

  while cursor < len(data):
    token = data[cursor]
    cursor += 1
    literals = length(token >> 4)
    require(literals <= expected_size - len(output), "LZ4 output exceeds segment size")
    output.extend(checked_slice(data, cursor, literals))
    cursor += literals
    if cursor == len(data):
      break
    distance = int.from_bytes(checked_slice(data, cursor, 2), "little")
    cursor += 2
    require(0 < distance <= len(output), "Invalid LZ4 back-reference")
    count = length(token & 15) + 4
    require(count <= expected_size - len(output), "LZ4 match exceeds segment size")
    for _ in range(count):
      output.append(output[-distance])
  require(len(output) == expected_size, "LZ4 segment size differs")
  return bytes(output)


def inspect_program(nso, npdm):
  require(len(nso) >= 0x100 and nso[:4] == b"NSO0", "Invalid NSO executable")
  segments = []
  for index in range(3):
    offset, _, size = struct.unpack_from("<III", nso, 0x10 + index * 0x10)
    compressed = u32(nso, 12) & (1 << index)
    file_size = u32(nso, 0x60 + index * 4) if compressed else size
    data = checked_slice(nso, offset, file_size)
    segment = lz4_block(data, size) if compressed else data
    require(hashlib.sha256(segment).digest() == nso[0xa0 + index * 32:0xc0 + index * 32],
      "Invalid NSO segment hash")
    segments.append(segment)
  executable = b"".join(segments)
  require(TARGET.encode() in executable, "Forwarder target path is absent")
  require(b"/Users/" not in executable and b"/home/" not in executable,
    "Forwarder contains a developer path")
  require(len(npdm) >= 0x80 and npdm[:4] == b"META", "Invalid NPDM")
  aci = checked_slice(npdm, u32(npdm, 0x70), u32(npdm, 0x74))
  acid = checked_slice(npdm, u32(npdm, 0x78), u32(npdm, 0x7c))
  require(aci[:4] == b"ACI0" and u64(aci, 0x10) == TITLE_ID, "Invalid NPDM program ID")
  require(acid[0x200:0x204] == b"ACID" and u64(acid, 0x210) == TITLE_ID and
    u64(acid, 0x218) == TITLE_ID, "Invalid NPDM program range")


def inspect_parts(parts):
  records = []
  for name, content_type, data in zip(("program", "control", "meta"), (0, 2, 1), parts):
    require(len(data) >= 0xc00 and len(data) % 512 == 0, f"Invalid {name} size")
    require(data[0x200:0x204] == b"NCA3" and data[0x205] == content_type, f"Invalid {name} header")
    require(u64(data, 0x208) == len(data) and u64(data, 0x210) == TITLE_ID, f"Invalid {name} identity")
    require(data[:0x200] == bytes(0x200), f"{name} must have no signatures")
    require(data[0x230:0x240] == bytes(16), f"{name} must have no rights ID")
    require(data[0x300:0x340] == bytes(64), f"{name} must have no encrypted key area")
    sections = []
    for index in range(4):
      start, end = struct.unpack_from("<II", data, 0x240 + index * 16)
      if start == end == 0:
        continue
      require(6 <= start < end and end * 512 <= len(data), f"Invalid {name} section")
      fs = data[0x400 + index * 512:0x600 + index * 512]
      require(fs[4] == 1, f"{name} section must be plaintext")
      require(hashlib.sha256(fs).digest() == data[0x280 + index * 32:0x2a0 + index * 32],
        f"Invalid {name} filesystem header hash")
      sections.append((start * 512, end * 512, fs))
    require(len(sections) == 1, f"Unexpected {name} section count")
    if name in ("program", "meta"):
      start, end, fs = sections[0]
      require(fs[2:4] == b"\x01\x02", f"Expected {name} PFS0 hierarchy")
      table_offset, table_size, file_offset, file_size = struct.unpack_from("<QQQQ", fs, 0x30)
      block_size = u32(fs, 0x28)
      require(0 < block_size <= 1024 * 1024, "Invalid hash block size")
      section = data[start:end]
      table = checked_slice(section, table_offset, table_size)
      files = checked_slice(section, file_offset, file_size)
      hashes = b"".join(hashlib.sha256(files[i:i + block_size]).digest()
        for i in range(0, len(files), block_size))
      require(table == hashes and hashlib.sha256(table).digest() == fs[8:40], f"Invalid {name} hash hierarchy")
      entries = pfs_entries(files)
      if name == "program":
        require(set(entries) == {"main", "main.npdm"}, "Unexpected executable contents")
        inspect_program(entries["main"], entries["main.npdm"])
      else:
        require(len(entries) == 1, "Expected one CNMT file")
        cnmt = next(iter(entries.values()))
        require(len(cnmt) == 192 and u64(cnmt, 0) == TITLE_ID, "Invalid CNMT identity")
        require(u32(cnmt, 8) == 0 and cnmt[12] == 0x80, "Invalid application metadata")
        require(struct.unpack_from("<HHH", cnmt, 14) == (16, 2, 0), "Invalid CNMT record counts")
        require(cnmt[102] == 1 and cnmt[158] == 3, "Invalid CNMT content types")
    else:
      # The tiny control RomFS contains exactly the generated NACP and icon.
      title_record = nacp_title_record()
      nacp_offset = data.find(title_record)
      require(nacp_offset >= 0 and (ROOT / "assets/icon.jpg").read_bytes() in data,
        "Control metadata or icon differs from Wodex")
      nacp = checked_slice(data, nacp_offset, 0x4000)
      require(nacp[0x3060:0x3070].split(b"\0", 1)[0].decode() == FORWARDER_VERSION,
        "Incorrect forwarder display version")
      require(nacp[0x3025] == 0, "Forwarder must not require a user profile")
      require(u64(nacp, 0x3038) == TITLE_ID and u64(nacp, 0x3078) == TITLE_ID,
        "Incorrect control title identity")
    records.append({"name": name, "nca_content_type": content_type, "size": len(data), "sha256": digest(data)})
  return records


def nacp_title_record():
  # Validation constructs the expected title records and verifies them separately
  # from fields nacptool populates for the system.
  name = TITLE.encode().ljust(0x200, b"\0")
  author = AUTHOR.encode().ljust(0x100, b"\0")
  return name + author


def check_sources():
  for name, expected in SOURCES.items():
    require(digest((ROOT / name).read_bytes()) == expected, f"Source checksum mismatch: {name}")


def manifest_for(data, parts):
  return {
    "format": "WDXHOME1",
    "title_id": f"{TITLE_ID:016x}",
    "target_nro": TARGET,
    "title": TITLE,
    "author": AUTHOR,
    "forwarder_version": FORWARDER_VERSION,
    "application_version": 0,
    "size": len(data),
    "sha256": digest(data),
    "toolchain": TOOLCHAIN,
    "nx_hbloader_revision": LOADER_REVISION,
    "hacbrewpack_revision": TOOL_REVISION,
    "generator_sha256": digest(Path(__file__).read_bytes()),
    "header_helper_sha256": digest((ROOT / "scripts/home-menu/clear-header.c").read_bytes()),
    "source_sha256": SOURCES,
    "contents": inspect_parts(parts),
  }


def check():
  check_sources()
  data = (RESOURCES / "template.bin").read_bytes()
  expected = manifest_for(data, read_bundle(data))
  require(json.loads((RESOURCES / "manifest.json").read_text()) == expected, "Manifest differs from template")
  license_text = (ROOT / "external/nx-hbloader/LICENSE.md").read_bytes()
  require((RESOURCES / "LICENSE.nx-hbloader.txt").read_bytes() == license_text, "Missing embedded ISC notice")
  require((ROOT / "licenses/native/nx-hbloader-82b9512/LICENSE.md").read_bytes() == license_text,
    "Missing distribution ISC notice")
  print(f"HOME Menu template verified: {len(data)} bytes, SHA-256 {digest(data)}")


def fetch_tools():
  DEPENDENCY.parent.mkdir(parents=True, exist_ok=True)
  if not (DEPENDENCY / ".git").exists():
    run(["git", "init", "-q", DEPENDENCY])
    run(["git", "-C", DEPENDENCY, "remote", "add", "origin", TOOL_URL])
  actual_url = run(["git", "-C", DEPENDENCY, "remote", "get-url", "origin"], capture_output=True, text=True).stdout.strip()
  require(actual_url == TOOL_URL, "Unexpected packaging tool remote")
  run(["git", "-C", DEPENDENCY, "fetch", "--depth=1", "origin", TOOL_REVISION])
  run(["git", "-C", DEPENDENCY, "checkout", "--detach", TOOL_REVISION])


def replace_once(text, old, new):
  require(text.count(old) == 1, "Pinned loader patch no longer matches")
  return text.replace(old, new)


def prepare_loader(destination):
  source = (ROOT / "external/nx-hbloader/source/main.c").read_text()
  source = replace_once(source, '#define DEFAULT_NRO "sdmc:/hbmenu.nro"', f'#define DEFAULT_NRO "{TARGET}"')
  source = replace_once(source, "static char g_argv[2048];", "static bool wodex_started = false;\nstatic char g_argv[2048];")
  source = replace_once(source, "        memcpy(g_nextNroPath, DEFAULT_NRO, sizeof(DEFAULT_NRO));",
    "        if (wodex_started) wodexReturnHome();\n        wodex_started = true;\n"
    "        memcpy(g_nextNroPath, DEFAULT_NRO, sizeof(DEFAULT_NRO));")
  source = replace_once(source, "diagAbortWithResult(MAKERESULT(Module_HomebrewLoader, 3));", "wodexReturnHome();")
  # Older Wodex NROs close their applet services and return to the loader.
  # Reacquire the application proxy and perform the AM exit handshake so the
  # tile also exits cleanly after switching to an older Versions build.
  # hbloader closes SM before loading an NRO, and its exit() wrapper aborts.
  home_exit = """static void NX_NORETURN wodexReturnHome(void) {
  if (R_SUCCEEDED(smInitialize())) {
    // Open the normal application proxy directly: appletInitialize would wait
    // for foreground focus even when HOME is already closing the old NRO.
    Service manager = {0}, application = {0}, controller = {0};
    Result rc = smGetService(&manager, "appletOE");
    if (R_SUCCEEDED(rc)) rc = serviceConvertToDomain(&manager);
    if (R_SUCCEEDED(rc)) {
      const u64 reserved = 0;
      serviceAssumeDomain(&manager);
      do {
        rc = serviceDispatchIn(&manager, 0, reserved,
          .in_send_pid = true,
          .in_num_handles = 1,
          .in_handles = { CUR_PROCESS_HANDLE },
          .out_num_objects = 1,
          .out_objects = &application);
        if (R_VALUE(rc) == 0x19280) svcSleepThread(100000000);
      } while (R_VALUE(rc) == 0x19280);
    }
    if (R_SUCCEEDED(rc)) {
      serviceAssumeDomain(&application);
      rc = serviceDispatch(&application, 1,
        .out_num_objects = 1,
        .out_objects = &controller);
    }
    if (R_SUCCEEDED(rc)) {
      serviceAssumeDomain(&controller);
      rc = serviceDispatch(&controller, 0);
    }
    serviceClose(&controller);
    serviceClose(&application);
    serviceClose(&manager);
    smExit();
    // After ISelfController::Exit, AM terminates the process asynchronously.
    if (R_SUCCEEDED(rc)) while (true) svcSleepThread(86400000000000ULL);
  }
  svcExitProcess();
  __builtin_unreachable();
}

"""
  source = replace_once(source, "void loadNro(void)\n{", home_exit + "void loadNro(void)\n{")
  (destination / "main.c").write_text(source)
  shutil.copyfile(ROOT / "external/nx-hbloader/source/trampoline.s", destination / "trampoline.s")
  config = json.loads((ROOT / "external/nx-hbloader/hbl.json").read_text())
  config["name"] = TITLE
  for field in ("title_id", "title_id_range_min", "title_id_range_max"):
    config.pop(field)
  for field in ("program_id", "program_id_range_min", "program_id_range_max"):
    config[field] = f"0x{TITLE_ID:016x}"
  config["version"] = "0x0"
  config["system_resource_size"] = "0x1000000"
  config["optimize_memory_allocation"] = False
  config["disable_device_address_space_merge"] = False
  config["enable_alias_region_extra_size"] = False
  config["prevent_code_reads"] = False
  config["signature_key_generation"] = 0
  config["filesystem_access"].pop("content_owner_ids", None)
  config["service_host"] = []
  for capability in config["kernel_capabilities"]:
    if capability["type"] == "application_type":
      capability["value"] = 1
    if capability["type"] == "debug_flags":
      capability["value"] = {"allow_debug": False, "force_debug_prod": False, "force_debug": False}
  (destination / "application.json").write_text(json.dumps(config, indent=2) + "\n")


def generate(destination):
  check_sources()
  require((DEPENDENCY / ".git").exists(), "Run --fetch-tools first; normal generation never accesses the network")
  revision = run(["git", "-C", DEPENDENCY, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
  require(revision == TOOL_REVISION, "Wrong hacBrewPack revision")
  require(not run(["git", "-C", DEPENDENCY, "diff", "--name-only", "HEAD"], capture_output=True, text=True).stdout,
    "Packaging tool has modified tracked sources")
  shutil.copyfile(DEPENDENCY / "config.mk.template", DEPENDENCY / "config.mk")
  run(["make", "-C", DEPENDENCY, "-j4"], stdout=subprocess.DEVNULL)
  sdk = Path(os.environ.get("DEVKITPRO", "/opt/devkitpro"))
  compiler = sdk / "devkitA64/bin/aarch64-none-elf-gcc"
  tools = sdk / "tools/bin"
  work_root = ROOT / "build/home-menu"
  work_root.mkdir(parents=True, exist_ok=True)
  with tempfile.TemporaryDirectory(prefix="template-", dir=work_root) as temporary:
    work = Path(temporary)
    prepare_loader(work)
    for directory in ("exefs", "control", "out"):
      (work / directory).mkdir()
    flags = ["-march=armv8-a", "-mtune=cortex-a57", "-mtp=soft", "-fPIE", "-O2",
      "-ffunction-sections", "-fdata-sections", "-D__SWITCH__", '-DVERSION="v2.4.5"',
      f"-I{sdk}/libnx/include", f"-ffile-prefix-map={ROOT}=/wodex", "-g0"]
    run([compiler, *flags, "-c", work / "main.c", "-o", work / "main.o"])
    run([compiler, *flags, "-c", work / "trampoline.s", "-o", work / "trampoline.o"])
    run([compiler, *flags, f"-specs={sdk}/libnx/switch.specs", f"-L{sdk}/libnx/lib",
      "-Wl,--gc-sections", "-Wl,--build-id=none", work / "main.o", work / "trampoline.o",
      "-lnx", "-o", work / "forwarder.elf"])
    run([tools / "elf2nso", work / "forwarder.elf", work / "exefs/main"])
    run([tools / "npdmtool", work / "application.json", work / "exefs/main.npdm"])
    run([tools / "nacptool", "--create", TITLE, AUTHOR, FORWARDER_VERSION,
      work / "control/control.nacp", f"--titleid={TITLE_ID:016x}"])
    control_path = work / "control/control.nacp"
    nacp = bytearray(control_path.read_bytes())
    require(len(nacp) == 0x4000, "Unexpected generated control metadata")
    nacp[0x3025] = 0  # Wodex stores state on SD and needs no selected user profile.
    control_path.write_bytes(nacp)
    shutil.copyfile(ROOT / "assets/icon.jpg", work / "control/icon_AmericanEnglish.dat")
    # These public test vectors are unrelated to any console's keys.
    (work / "synthetic.keys").write_text(
      "header_key = " + bytes(range(32)).hex() + "\nkey_area_key_application_00 = " +
      bytes(range(16)).hex() + "\n")
    run([DEPENDENCY / "hacbrewpack", "-k", work / "synthetic.keys",
      "--titleid", f"{TITLE_ID:016x}", "--nspdir", work / "out",
      "--noromfs", "--nologo", "--plaintext", "--nosignncasig2", "--keygeneration", "1"],
      cwd=work, stdout=subprocess.DEVNULL)
    helper = work / "clear-header"
    run(["cc", "-O2", "-Wall", "-Wextra", "-Werror",
      f"-I{DEPENDENCY}/mbedtls/include", ROOT / "scripts/home-menu/clear-header.c",
      DEPENDENCY / "mbedtls/library/libmbedcrypto.a", "-o", helper])
    archive = (work / "out" / f"{TITLE_ID:016x}.nsp").read_bytes()
    content = {}
    for filename, raw in pfs_entries(archive).items():
      require(filename.endswith(".nca") and len(raw) >= 0xc00, "Unexpected generated archive entry")
      header = run([helper], input=raw[:0xc00], capture_output=True).stdout
      plain = bytearray(header + raw[0xc00:])
      plain[0x300:0x340] = bytes(64)
      kind = plain[0x205]
      require(kind not in content, "Duplicate generated content")
      content[kind] = bytes(plain)
    require(set(content) == {0, 1, 2}, "Unexpected generated content types")
    parts = [content[kind] for kind in (0, 2, 1)]
    data = MAGIC + struct.pack("<III", *(len(part) for part in parts)) + b"".join(parts)
    manifest = manifest_for(data, parts)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "template.bin").write_bytes(data)
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    shutil.copyfile(ROOT / "external/nx-hbloader/LICENSE.md", destination / "LICENSE.nx-hbloader.txt")
    print(f"Built HOME Menu template: {len(data)} bytes, SHA-256 {digest(data)}")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  modes = parser.add_mutually_exclusive_group()
  modes.add_argument("--check", action="store_true", help="Validate committed resources without build dependencies or network")
  modes.add_argument("--fetch-tools", action="store_true", help="Explicitly fetch pinned build-only sources")
  modes.add_argument("--verify-rebuild", action="store_true", help="Regenerate offline and require a byte-for-byte match")
  args = parser.parse_args()
  if args.check:
    check()
  elif args.fetch_tools:
    fetch_tools()
  elif args.verify_rebuild:
    check()
    with tempfile.TemporaryDirectory(prefix="wodex-home-menu-") as temporary:
      output = Path(temporary)
      generate(output)
      for name in ("template.bin", "manifest.json", "LICENSE.nx-hbloader.txt"):
        require((output / name).read_bytes() == (RESOURCES / name).read_bytes(), f"Non-reproducible resource: {name}")
    print("HOME Menu template rebuild is byte-for-byte identical.")
  else:
    generate(RESOURCES)


if __name__ == "__main__":
  try:
    main()
  except (OSError, ValueError, struct.error, subprocess.CalledProcessError) as error:
    print(f"HOME Menu template: {error}", file=sys.stderr)
    sys.exit(1)
