#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
toolchain="${WODEX_RUST_TOOLCHAIN:-nightly-2026-10-01}"
target="aarch64-nintendo-switch-freestanding"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${project_root}/target}"

if ! rustup run "${toolchain}" rustc --version >/dev/null 2>&1; then
  printf 'Install the Rust build toolchain: rustup toolchain install %s --profile minimal --component rust-src\n' "${toolchain}" >&2
  exit 1
fi
sysroot="$(rustup run "${toolchain}" rustc --print sysroot)"
cargo_root="${CARGO_HOME:-${HOME}/.cargo}"

# Cargo's encoded flags preserve paths containing spaces. Respect its precedence
# over RUSTFLAGS, otherwise split ordinary flags using Cargo's whitespace rules.
flags=()
if [[ -z "${CARGO_ENCODED_RUSTFLAGS+x}" ]]; then
  IFS=$' \t\n\r\v\f' read -r -d '' -a flags < <(printf '%s\0' "${RUSTFLAGS:-}")
  CARGO_ENCODED_RUSTFLAGS=""
fi

# Apply broad mappings first: rustc chooses the last matching prefix. Mapping
# toolchain sources also covers panic locations in the rebuilt core/alloc.
flags+=(
  "-C" "relocation-model=pic"
  "-C" "target-cpu=cortex-a57"
  "--remap-path-prefix=${HOME}=/wodex/home"
  "--remap-path-prefix=${cargo_root}=/wodex/cargo"
  "--remap-path-prefix=${sysroot}=/wodex/rust-toolchain"
  "--remap-path-prefix=${project_root}=/wodex/source"
)
for flag in "${flags[@]}"; do
  CARGO_ENCODED_RUSTFLAGS+="${CARGO_ENCODED_RUSTFLAGS:+$'\x1f'}${flag}"
done
export CARGO_ENCODED_RUSTFLAGS

cd "${project_root}"
exec cargo "+${toolchain}" build \
  -Zbuild-std=core,alloc,compiler_builtins \
  --locked \
  --release \
  --no-default-features \
  --target "${target}" \
  --package wodex-core
