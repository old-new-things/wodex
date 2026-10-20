# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust application, backend, editor, renderer, protocol, release validation and hashing tests | 70 passed |
| Native integration against isolated loopback API | 71 checks passed |
| Rust application through real SDL input, rendering, curl and save/reload | 28 checks passed |
| Native SDL/libnx callback interface and input normalization | 18 checks passed |
| Bundled CJK glyph coverage and mixed-language rendering | 76 checks passed |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless chat and update-page rendering smoke tests | Passed; migrated demo is pixel-identical to the previous preview |
| Update download/install integration | 185 mock checks passed; 187 with the actual packaged NRO |
| Local release-workflow validation and publishing simulation | Passed; no remote publication |
| Desktop resource discovery from a different working directory | Passed |
| Rust Switch `no_std` + `alloc` cross-compilation | Passed, nightly-2026-10-01 |
| Native Switch compilation and NRO link | Passed without warnings, devkitPro 20260219 |
| Embedded NRO icon, CA bundle, Noto CJK font, OFL and attribution | Verified byte for byte |
| Nested RomFS resource rebuilds and required-font guard | Passed |
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.10 seconds to render and exit, 50.4 MiB peak RSS on desktop software renderer |

Final NRO: `build/switch/wodex.nro`, version 0.2.0, 26,121,910 bytes (about 24.9 MiB). Embedded NACP version is verified.

SHA-256: `332f32417387b208f2ca1bc6b47ce9477368182155ec373e0ccadbc973254cf4`.

Native tests call the production Rust transport and storage through test-only C++ facades. They cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. The application test drives the production Rust application through SDL compose, editor cancellation, send, retry, update navigation and exit/reload, including an environment-only API key. Adapter tests verify normalized SDL/libnx input and clipboard values. Font tests exercise the Rust filename selection and renderer for Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection.

Rust tests additionally cover UTF-8 editing and IME composition, clipboard limits, environment-key persistence, streaming/HTTP completion ordering, stale update actions after changing or clearing the source, failed-save installation guards, cache/resource lifetimes and bounds, long-history scrolling, BOM/line-ending variations, malformed events, state limits, escaped-JSON storage bounds and exhausted saved IDs. A 20-million-pixel rounded card emits only its 50 visible row callbacks; a far-scrolled 60,000-line paragraph draws its visible tail.

Backend fault-injection tests cover interrupted/short writes, byte-exact corrupt storage, failed flush cleanup, Switch save rotation, rollback and failed-rollback recovery, exclusive recovery files, permissions, directory/font resolution and font symlinks. HTTP/runtime tests cover suppressed error bodies, redirect/interim headers, header limits, sink failures, cancellation, queue bounds, mutex/worker ownership and failed-spawn cleanup. The same backend code cross-compiles with `no_std` and `alloc`, using native primitive tables rather than a Rust `std` port.

Update tests additionally cover allowed and rejected GitHub redirects, source binding, no chat-credential leakage, release version/digest/size requirements, cancellation cleanup, checksum/size mismatch, NRO/ASET/RomFS bounds and missing assets, tampered stages, actual packaged-NRO replacement in a temporary folder, retained backups, failed installation rollback and failed-rollback recovery. Desktop CTest passes all seven suites. The update repository remains unset as requested, and no live release endpoint has been checked.

Physical Switch graphics, controls, keyboard, SD filesystem, self-replacement and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. GitHub Actions is configured but has not run remotely because the project has not been published.
