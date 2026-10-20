#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export DEVKITPRO="${DEVKITPRO:-/opt/devkitpro}"
export DEVKITA64="${DEVKITA64:-${DEVKITPRO}/devkitA64}"
export PATH="${DEVKITA64}/bin:${DEVKITPRO}/tools/bin:${PATH}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${project_root}/target}"

toolchain="${WODEX_RUST_TOOLCHAIN:-nightly-2026-10-01}"
target="aarch64-nintendo-switch-freestanding"
pkg_config="${DEVKITPRO}/portlibs/switch/bin/aarch64-none-elf-pkg-config"
jobs="${WODEX_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '4\n')}"

if [[ ! -f "${project_root}/VERSION" ]]; then
  printf 'Missing VERSION file. Restore the canonical application version.\n' >&2
  exit 1
fi
version="$(awk 'NR == 1 && length($0) <= 15 && $0 ~ /^(0|[1-9][0-9]*)[.](0|[1-9][0-9]*)[.](0|[1-9][0-9]*)$/ { version=$0; next } { invalid=1 } END { if (NR == 1 && !invalid) print version }' "${project_root}/VERSION")"
if [[ -z "${version}" ]]; then
  printf 'VERSION must contain one stable X.Y.Z version of at most 15 characters.\n' >&2
  exit 1
fi

if [[ ! -f "${DEVKITPRO}/libnx/switch_rules" ]]; then
  printf 'Missing devkitPro Switch SDK at %s. Install switch-dev or use devkitpro/devkita64:20260219.\n' "${DEVKITPRO}" >&2
  exit 1
fi

for command_name in cargo rustup make aarch64-none-elf-g++ elf2nro nacptool; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    printf 'Required build tool is missing: %s\n' "${command_name}" >&2
    exit 1
  fi
done

if [[ ! -x "${pkg_config}" ]] || ! "${pkg_config}" --exists sdl2 SDL2_ttf libcurl; then
  printf 'Missing Switch portlibs. Install switch-sdl2, switch-sdl2_ttf, switch-curl, and switch-pkg-config.\n' >&2
  exit 1
fi

for resource_path in \
  romfs/cacert.pem \
  romfs/fonts/NotoSansCJKsc-Regular.otf \
  romfs/fonts/OFL.txt \
  assets/icon.jpg; do
  if [[ ! -f "${project_root}/${resource_path}" || ! -s "${project_root}/${resource_path}" ]]; then
    printf 'Missing or empty build resource: %s. Restore the vendored file before building.\n' "${resource_path}" >&2
    exit 1
  fi
done

if ! rustup run "${toolchain}" rustc --version >/dev/null 2>&1; then
  printf 'Install the Rust build toolchain: rustup toolchain install %s --profile minimal --component rust-src\n' "${toolchain}" >&2
  exit 1
fi

# libnx owns process startup and the final link. Rust contributes a PIC static
# library, with only core/alloc and the C allocator supplied by the native shell.
export RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }-C relocation-model=pic -C target-cpu=cortex-a57"
cd "${project_root}"
cargo "+${toolchain}" build \
  -Zbuild-std=core,alloc,compiler_builtins \
  --locked \
  --release \
  --no-default-features \
  --target "${target}" \
  --package wodex-core

make -f scripts/Makefile.switch -j "${jobs}" \
  RUST_LIB="${CARGO_TARGET_DIR}/${target}/release/libwodex_core.a"
printf 'Wodex %s built: %s/build/switch/wodex.nro\n' "${version}" "${project_root}"
