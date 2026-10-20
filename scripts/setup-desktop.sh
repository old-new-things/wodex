#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -d external/SDL2_ttf-2.24.0 ]]; then
  exit 0
fi
archive="$(mktemp -t wodex-ttf.XXXXXX)"
trap 'rm -f "$archive"' EXIT
curl --fail --location --proto '=https' --tlsv1.2 \
  https://github.com/libsdl-org/SDL_ttf/releases/download/release-2.24.0/SDL2_ttf-2.24.0.tar.gz \
  --output "$archive"
expected="0b2bf1e7b6568adbdbc9bb924643f79d9dedafe061fa1ed687d1d9ac4e453bfd"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$archive" | cut -d ' ' -f 1)"
else
  actual="$(shasum -a 256 "$archive" | cut -d ' ' -f 1)"
fi
if [[ "$actual" != "$expected" ]]; then
  printf 'SDL2_ttf archive checksum mismatch.\n' >&2
  exit 1
fi
tar -xzf "$archive" -C external
