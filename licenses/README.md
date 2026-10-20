# Binary redistribution notices

This directory accompanies Wodex Switch binaries as `licenses.tar.gz`. Preserve it, `LICENSE`, `THIRD_PARTY.md`, and the bundled font notices when redistributing a binary.

The native inventory below was checked against the linked archives in the Switch link map and the installed packages in `devkitpro/devkita64:20260219`. These are library/runtime notices; the build tools themselves are not distributed in the NRO. Upstream copyright holders and their contact details are preserved as required attribution.

The HOME Menu forwarder also embeds native code from [nx-hbloader commit `82b95122`](https://github.com/switchbrew/nx-hbloader/tree/82b95122c5ae8dc059bf23893ba7623c72c86773), under the ISC license in `native/nx-hbloader-82b9512/LICENSE.md`. It uses libnx from the SDK listed below. The pinned source, Wodex's build-time changes, and reproducible template instructions are in `external/nx-hbloader/README.wodex.md` and `romfs/home-menu/README.md` in the source repository.

| Component | SDK package version | Notice source |
| --- | --- | --- |
| SDL2 | 2.28.5-4 | [Upstream release license](https://github.com/libsdl-org/SDL/blob/release-2.28.5/LICENSE.txt), copied to `native/SDL2-2.28.5/LICENSE.txt` |
| SDL2_ttf | 2.22.0-2 | SDK `switch-sdl2_ttf/LICENSE.txt`, copied to `native/SDL2_ttf-2.22.0/LICENSE.txt` |
| FreeType | 2.13.3-1 | SDK `switch-freetype/FTL.TXT` and `LICENSE.TXT`, plus the [BDF](https://github.com/freetype/freetype/blob/VER-2-13-3/src/bdf/README) and [PCF](https://github.com/freetype/freetype/blob/VER-2-13-3/src/pcf/README) driver notices, in `native/freetype-2.13.3/` |
| HarfBuzz | 10.0.1-1 | [Upstream COPYING](https://github.com/harfbuzz/harfbuzz/blob/10.0.1/COPYING), in `native/harfbuzz-10.0.1/` |
| libcurl | 7.69.1-5 | [Upstream COPYING](https://github.com/curl/curl/blob/curl-7_69_1/COPYING), in `native/curl-7.69.1/` |
| zlib | 1.3.1-1 | SDK `switch-zlib/LICENSE`, in `native/zlib-1.3.1/` |
| bzip2 | 1.0.8-1 | SDK `switch-bzip2/LICENSE`, in `native/bzip2-1.0.8/` |
| libpng | 1.6.48-1 | SDK `switch-libpng/LICENSE`, in `native/libpng-1.6.48/` |
| libnx | 4.12.0-1 | [Upstream LICENSE.md](https://github.com/switchbrew/libnx/blob/v4.12.0/LICENSE.md), in `native/libnx-4.12.0/` |
| Mesa, EGL and glapi | 20.1.0-5 | [Switch port license document](https://github.com/devkitPro/mesa/blob/05fc4b1d449b8fe12cf1a47b46c4a1c1f4e4e3a6/docs/license.html), in `native/mesa-20.1.0/`; the SDK package uses the 20.1.0-rc3 source with Switch patches |
| libdrm_nouveau | 1.0.1-2 | Copyright/permission blocks from the [v1.0.1 source and headers](https://github.com/devkitPro/libdrm_nouveau/tree/v1.0.1), in `native/libdrm_nouveau-1.0.1/` |
| Newlib C/math/thread/system runtime | 4.6.0.20260123-4 | `COPYING.NEWLIB` and `COPYING.LIBGLOSS` from the [source archive](https://sourceware.org/pub/newlib/newlib-4.6.0.20260123.tar.gz), plus added copyright/permission blocks from the [devkitA64 patch](https://github.com/devkitPro/buildscripts/blob/devkitA64_r29.2/patches/newlib-4.6.0.20260123-4.patch), in `native/newlib-4.6.0.20260123/` |
| GCC support and C++ runtime | 15.2.0-7 | [GPLv3](https://github.com/gcc-mirror/gcc/blob/releases/gcc-15.2.0/COPYING3) and [GCC Runtime Library Exception](https://github.com/gcc-mirror/gcc/blob/releases/gcc-15.2.0/COPYING.RUNTIME), in `native/gcc-15.2.0/` |

The Newlib source archive was verified against its SDK recipe SHA-256: `6ff27e3bf022666f43f7802255be680eeff722ac181b1725d21e2e8318604ee3`. SDK package recipes and patches are maintained in [devkitPro/pacman-packages](https://github.com/devkitPro/pacman-packages).

FreeType is used under the FreeType Project License. Portions of this software are copyright © 1996–2024 The FreeType Project (https://www.freetype.org/). All rights reserved. This product includes software developed by the University of California, Berkeley and its contributors, as acknowledged in the Newlib runtime notices.

`rust/README.md` identifies the locked Rust dependencies and the exact Rust `core`/`alloc`/compiler runtime, with original permission texts and public source links. The application uses `no_std` on Switch.

The embedded Mozilla CA certificate data is covered by MPL 2.0; its full license is in `mozilla/MPL-2.0.txt`, obtained from [Mozilla's canonical license text](https://www.mozilla.org/media/MPL/2.0/index.txt). The unmodified certificate bundle and its public source references are in `romfs/cacert.pem` in Wodex's source repository.

The Noto Sans CJK font's SIL Open Font License and attribution are in `romfs/fonts/OFL.txt` and `romfs/fonts/README.md`; both are embedded in the NRO and included separately with downloads.

Source Code Pro Regular 2.042 is copyright © 2023 Adobe, with Reserved Font Name "Source", and distributed under SIL Open Font License 1.1. Its unmodified upstream license from [commit `803b7e23`](https://github.com/adobe-fonts/source-code-pro/blob/803b7e23ec97ae58b6232ea76519a76d428ba268/LICENSE.md) is in `fonts/source-code-pro-2.042/OFL.txt` and embedded as `romfs/fonts/SourceCodePro-OFL.txt`. Font provenance and checksums are recorded in `romfs/fonts/README.md`.

When updating a linked library, the SDK image, Rust toolchain, or Cargo lockfile, refresh its corresponding notices and provenance before distributing the resulting binary.
