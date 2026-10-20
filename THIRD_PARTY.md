# Third-party components

- [nlohmann/json 3.12.0](https://github.com/nlohmann/json/tree/v3.12.0): vendored single header, MIT license in `external/nlohmann/LICENSE.MIT`. Header SHA-256: `aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63`.
- [Mozilla CA bundle distributed by curl](https://curl.se/docs/caextract.html): `romfs/cacert.pem`, retrieved 2026-10-19, SHA-256 `a41b5d356aea97a529fe27e0f7316d2f9d946d75927476cf9cf1b90637d00505`. Mozilla's upstream certificate data is covered by the Mozilla Public License 2.0. Update from the public upstream when maintaining releases.
- [SDL2](https://github.com/libsdl-org/SDL) and [SDL2_ttf](https://github.com/libsdl-org/SDL_ttf): zlib license. The optional desktop download is SDL2_ttf 2.24.0, SHA-256 `0b2bf1e7b6568adbdbc9bb924643f79d9dedafe061fa1ed687d1d9ac4e453bfd`; its source archive includes its license.
- [FreeType](https://freetype.org/): FreeType Project License or GPLv2. Linked through SDL2_ttf under the FreeType Project License.
- [libcurl](https://curl.se/docs/copyright.html): curl license.
- [libnx](https://github.com/switchbrew/libnx): ISC license. The Switch SDK supplies libnx and native library builds; their notices remain in the SDK/package sources.
- Rust crates `serde` and `serde_json`: MIT OR Apache-2.0. Exact versions are recorded in `Cargo.lock`.

Wodex does not redistribute Nintendo's shared fonts; it opens the device's system font through libnx. Desktop fonts are loaded from the user's system. No wiliwili source code or assets are incorporated.
