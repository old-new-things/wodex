# Third-party components

- [nlohmann/json 3.12.0](https://github.com/nlohmann/json/tree/v3.12.0): vendored single header, MIT license in `external/nlohmann/LICENSE.MIT`. Header SHA-256: `aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63`.
- [Mozilla CA bundle distributed by curl](https://curl.se/docs/caextract.html): `romfs/cacert.pem`, retrieved 2026-10-19, SHA-256 `a41b5d356aea97a529fe27e0f7316d2f9d946d75927476cf9cf1b90637d00505`. Mozilla's upstream certificate data is covered by the Mozilla Public License 2.0. Update from the public upstream when maintaining releases.
- [SDL2](https://github.com/libsdl-org/SDL) and [SDL2_ttf](https://github.com/libsdl-org/SDL_ttf): zlib license. The optional desktop download is SDL2_ttf 2.24.0, SHA-256 `0b2bf1e7b6568adbdbc9bb924643f79d9dedafe061fa1ed687d1d9ac4e453bfd`; its source archive includes its license.
- [FreeType](https://freetype.org/): FreeType Project License or GPLv2. Linked through SDL2_ttf under the FreeType Project License.
- [libcurl](https://curl.se/docs/copyright.html): curl license.
- [libnx](https://github.com/switchbrew/libnx): ISC license. The Switch SDK supplies libnx and native library builds; their notices remain in the SDK/package sources.
- Rust crates `serde`, `serde_json` and `sha2`: MIT OR Apache-2.0. Exact versions and transitive dependencies are recorded in `Cargo.lock`. `sha2` provides incremental SHA-256 verification using its software backend on Switch.
- [Noto Sans CJK SC Regular 2.004](https://github.com/notofonts/noto-cjk/blob/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf): Copyright © 2014–2021 Adobe (http://www.adobe.com/), SIL Open Font License 1.1. The unmodified font, full license and provenance are included in `romfs/fonts/` and packaged into RomFS. SHA-256: `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.

Wodex does not redistribute Nintendo's shared fonts; it opens them through libnx only when falling back from the bundled font. Desktop system fonts are likewise loaded only as a fallback or explicit user override. No wiliwili source code or assets are incorporated.
