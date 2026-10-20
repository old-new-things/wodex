# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust application, editor, renderer, protocol, release validation and hashing tests | 52 passed |
| Native integration against isolated loopback API | 71 checks passed |
| Rust application through real SDL input, rendering, curl and save/reload | 28 checks passed |
| Native callback interface, raw storage recovery and input normalization | 38 checks passed |
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
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.10 seconds to render and exit, 54.0 MiB peak RSS on desktop software renderer |

Final NRO: `build/switch/wodex.nro`, version 0.2.0, 26,236,598 bytes (about 25.0 MiB). Embedded NACP version is verified.

SHA-256: `680d2444bcb476d2252ab1e4822c70583dafb7447b295f89ca350179afd2d491`.

Native tests cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. The application test drives the production Rust application through SDL compose, editor cancellation, send, retry, update navigation and exit/reload, including an environment-only API key. Adapter tests preserve non-UTF-8/NUL saved bytes and verify the callback ABI's input and error paths. Font tests exercise the production Rust renderer for Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection.

Rust tests additionally cover UTF-8 editing and IME composition, clipboard limits, environment-key persistence, streaming/HTTP completion ordering, stale update actions after changing or clearing the source, failed-save installation guards, cache/resource lifetimes and bounds, long-history scrolling, BOM/line-ending variations, malformed events, state limits, escaped-JSON storage bounds and exhausted saved IDs. A 20-million-pixel rounded card emits only its 50 visible row callbacks; a far-scrolled 60,000-line paragraph draws its visible tail.

Update tests additionally cover allowed and rejected GitHub redirects, source binding, no chat-credential leakage, release version/digest/size requirements, cancellation cleanup, checksum/size mismatch, NRO/ASET/RomFS bounds and missing assets, tampered stages, actual packaged-NRO replacement in a temporary folder, retained backups, failed installation rollback and failed-rollback recovery. Desktop CTest passes all seven suites. The update repository remains unset as requested, and no live release endpoint has been checked.

Physical Switch graphics, controls, keyboard, SD filesystem, self-replacement and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. GitHub Actions is configured but has not run remotely because the project has not been published.
