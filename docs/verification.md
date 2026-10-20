# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust application, backend, editor, renderer, image decoding, protocol, update-feed validation and hashing tests | 124 passed |
| Compiled nightly/version identity and repeat-update detection | Passed with both publication identities; partial or invalid build metadata rejected |
| Native integration against isolated loopback API | 71 checks passed |
| Rust application through real SDL input, rendering, curl and save/reload | 35 checks passed |
| Native SDL/libnx callback interface and input normalization | 18 checks passed |
| Bundled CJK glyph coverage and mixed-language rendering | 216 checks passed, including styled fonts, the monospace code face and baseline metrics |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless chat and update-page rendering smoke tests | Passed; Versions/Nightly and empty-source screens visually inspected |
| Markdown typography, CJK, code/quotes/tasks, regular/wide tables, interrupted replies and offline image cards | Six saved-state previews rendered and visually inspected |
| PNG/JPEG downloads and native SDL display | 41 checks passed; transparency, aspect ratio, cache reuse, limits, cancellation and credential isolation; PNG/JPEG previews visually inspected |
| Update download/install integration | 239 mock checks passed; 241 with the actual packaged NRO |
| Rust Pages publisher integrity, retention and immutability tests | 12 passed |
| Source workflow build identity and publication metadata tests | 6 passed; API shape also verified against an existing Actions run |
| Publication-state source validation, restoration and archive-extraction tests | 23 passed |
| Live-publication verification, stale content, integrity and deadline tests | 11 passed |
| Pages workflow syntax and local publication simulation | Passed |
| Developer home/project paths removed from Switch NRO | Passed; neutral compiler paths verified |
| Desktop resource discovery from a different working directory | Passed |
| Rust Switch `no_std` + `alloc` cross-compilation | Passed, nightly-2026-10-01 |
| Native Switch compilation and NRO link | Passed without warnings, devkitPro 20260219 |
| Embedded NRO icon, CA bundle, Noto CJK/Source Code Pro fonts, OFL and attribution | Verified byte for byte |
| Nested RomFS resource rebuilds and required-font guard | Passed |
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.10 seconds to render and exit, 50.4 MiB peak RSS on desktop software renderer |
| Maximum-size 512 KiB reply, 262,141 short lines | 0.124 seconds for startup/render/screenshot, 51.2 MiB peak RSS; final marker visible |
| Maximum-size 512 KiB reply, one long paragraph | 0.075 seconds for startup/render/screenshot, 48.8 MiB peak RSS; final marker visible |

Local NRO: `build/switch/wodex.nro`, version 0.2.1, 26,692,982 bytes (about 25.5 MiB). The embedded NACP version is verified. This local build used a representative nightly identity for validation.

SHA-256: `780529a73cda98ae5fce6ca2684460faab1c9096f7a3cda5663125f6d63f1201`. Published CI binaries embed their actual workflow identity and have their own download checksums.

Native tests call the production Rust transport and storage through test-only C++ facades. They cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. The application test drives the production Rust application through SDL compose, editor cancellation, send, retry, update navigation and exit/reload, including an environment-only API key. Adapter tests verify normalized SDL/libnx input and clipboard values. Font tests exercise the Rust filename selection and renderer for Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection.

Rust tests additionally cover UTF-8 editing and IME composition, clipboard limits, environment-key persistence, streaming/HTTP completion ordering, stale update actions after changing the source or channel, failed-save installation guards, cache/resource lifetimes and bounds, long-history scrolling, BOM/line-ending variations, malformed events, state limits, escaped-JSON storage bounds and exhausted saved IDs. Markdown checks cover incomplete fences and inline syntax, nested lists/styles, CJK emphasis, table alignment and responsive layout, code indentation and combining marks, wrapping across style boundaries, repeated-header bounds and compact plain fallback. A 20-million-pixel rounded card emits only its 50 visible row callbacks; a far-scrolled 60,000-line paragraph draws its visible tail.

Backend fault-injection tests cover interrupted/short writes, byte-exact corrupt storage, failed flush cleanup, Switch save rotation, rollback and failed-rollback recovery, exclusive recovery files, permissions, directory/font resolution and font symlinks. HTTP/runtime tests cover suppressed error bodies, redirect/interim headers, header limits, sink failures, cancellation, queue bounds, mutex/worker ownership and failed-spawn cleanup. The same backend code cross-compiles with `no_std` and `alloc`, using native primitive tables rather than a Rust `std` port.

Image tests cover PNG grayscale/palette/transparency, baseline/progressive/grayscale JPEG, malformed headers and entropy, PNG chunk lengths/order and metadata filtering, source/pixel limits, downsampling, credential-free requests, failed downloads, cancellation and bounded texture lifetime. A delayed completion for the same URL after changing conversations cannot upload stale pixels. Image fixtures use an isolated loopback transport wrapper; ordinary screenshot and demo modes remain offline.

Update tests additionally cover redirect rejection, exact Pages project/channel URL binding, mixed-case repository paths, no chat-credential leakage, feed schema/digest/size requirements, nightly timestamp/commit/ID agreement, equal-time ordering, repeat-update suppression, timestamp ordering across different semantic versions, returning from Nightly to lower/equal/higher Versions builds, Versions-to-Versions downgrade rejection, cancellation cleanup, checksum/size mismatch, NRO/ASET/RomFS bounds and missing assets, tampered stages, actual packaged-NRO replacement in a temporary folder, retained backups, failed installation rollback and failed-rollback recovery. Desktop CTest includes nine suites, including Markdown fixtures and image integration. New installations use the official Pages source with Versions selected; existing empty sources remain disabled. Nightly is an explicit saved choice.

Physical Switch graphics, controls, keyboard, SD filesystem, self-replacement and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. Remote build results and downloadable artifacts are recorded in [GitHub Actions](https://github.com/old-new-things/wodex/actions/workflows/build.yml); the table above records the local verification. The separate [publication workflow](https://github.com/old-new-things/wodex/actions/workflows/publish.yml) verifies the served page, catalog, feed and newest channel downloads before reporting success.
