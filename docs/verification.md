# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust application, backend, editor, renderer, protocol, update-feed validation and hashing tests | 81 passed |
| Compiled nightly/version identity and repeat-update detection | Passed with both publication identities; partial or invalid build metadata rejected |
| Native integration against isolated loopback API | 71 checks passed |
| Rust application through real SDL input, rendering, curl and save/reload | 35 checks passed |
| Native SDL/libnx callback interface and input normalization | 18 checks passed |
| Bundled CJK glyph coverage and mixed-language rendering | 76 checks passed |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless chat and update-page rendering smoke tests | Passed; Versions/Nightly and empty-source screens visually inspected |
| Update download/install integration | 237 mock checks passed; 239 with the actual packaged NRO |
| Rust Pages publisher integrity, retention and immutability tests | 12 passed |
| Source workflow build identity and publication metadata tests | 6 passed; API shape also verified against an existing Actions run |
| Publication-state source validation, restoration and archive-extraction tests | 23 passed |
| Live-publication verification, stale content, integrity and deadline tests | 11 passed |
| Pages workflow syntax and local publication simulation | Passed |
| Developer home/project paths removed from Switch NRO | Passed; neutral compiler paths verified |
| Desktop resource discovery from a different working directory | Passed |
| Rust Switch `no_std` + `alloc` cross-compilation | Passed, nightly-2026-10-01 |
| Native Switch compilation and NRO link | Passed without warnings, devkitPro 20260219 |
| Embedded NRO icon, CA bundle, Noto CJK font, OFL and attribution | Verified byte for byte |
| Nested RomFS resource rebuilds and required-font guard | Passed |
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.10 seconds to render and exit, 50.4 MiB peak RSS on desktop software renderer |

Local NRO: `build/switch/wodex.nro`, version 0.2.1, 26,130,102 bytes (about 24.9 MiB). The embedded NACP version is verified. This local build used fixed publication metadata for validation.

SHA-256: `4da5dd59bbe29aceeb781a8977315c985852c4a8eac6162d8d229525fd97a3e5`. Published CI binaries embed their actual workflow identity and have their own download checksums.

Native tests call the production Rust transport and storage through test-only C++ facades. They cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. The application test drives the production Rust application through SDL compose, editor cancellation, send, retry, update navigation and exit/reload, including an environment-only API key. Adapter tests verify normalized SDL/libnx input and clipboard values. Font tests exercise the Rust filename selection and renderer for Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection.

Rust tests additionally cover UTF-8 editing and IME composition, clipboard limits, environment-key persistence, streaming/HTTP completion ordering, stale update actions after changing the source or channel, failed-save installation guards, cache/resource lifetimes and bounds, long-history scrolling, BOM/line-ending variations, malformed events, state limits, escaped-JSON storage bounds and exhausted saved IDs. A 20-million-pixel rounded card emits only its 50 visible row callbacks; a far-scrolled 60,000-line paragraph draws its visible tail.

Backend fault-injection tests cover interrupted/short writes, byte-exact corrupt storage, failed flush cleanup, Switch save rotation, rollback and failed-rollback recovery, exclusive recovery files, permissions, directory/font resolution and font symlinks. HTTP/runtime tests cover suppressed error bodies, redirect/interim headers, header limits, sink failures, cancellation, queue bounds, mutex/worker ownership and failed-spawn cleanup. The same backend code cross-compiles with `no_std` and `alloc`, using native primitive tables rather than a Rust `std` port.

Update tests additionally cover redirect rejection, exact Pages project/channel URL binding, mixed-case repository paths, no chat-credential leakage, feed schema/digest/size requirements, nightly timestamp/commit/ID agreement, equal-time ordering, repeat-update suppression, same-version Nightly-to-Versions transitions, semantic version downgrade rejection, cancellation cleanup, checksum/size mismatch, NRO/ASET/RomFS bounds and missing assets, tampered stages, actual packaged-NRO replacement in a temporary folder, retained backups, failed installation rollback and failed-rollback recovery. Desktop CTest passes all seven suites. New installations use the official Pages source with Versions selected; existing empty sources remain disabled. Nightly is an explicit saved choice.

Physical Switch graphics, controls, keyboard, SD filesystem, self-replacement and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. Remote build results and downloadable artifacts are recorded in [GitHub Actions](https://github.com/old-new-things/wodex/actions/workflows/build.yml); the table above records the local verification. The separate [publication workflow](https://github.com/old-new-things/wodex/actions/workflows/publish.yml) verifies the served page, catalog, feed and newest channel downloads before reporting success.
