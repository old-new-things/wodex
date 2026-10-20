# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust behavior, protocol, release validation and hashing tests | 26 passed |
| Native integration against isolated loopback API | 71 checks passed |
| Bundled CJK glyph coverage and mixed-language rendering | 76 checks passed |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless chat and update-page rendering smoke tests | Passed |
| Update download/install integration | 185 mock checks passed; 187 with the actual packaged NRO |
| Local release-workflow validation and publishing simulation | Passed; no remote publication |
| Desktop resource discovery from a different working directory | Passed |
| Rust Switch `no_std` + `alloc` cross-compilation | Passed, nightly-2026-10-01 |
| Native Switch compilation and NRO link | Passed without warnings, devkitPro 20260219 |
| Embedded NRO icon, CA bundle, Noto CJK font, OFL and attribution | Verified byte for byte |
| Nested RomFS resource rebuilds and required-font guard | Passed |
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.45 seconds to first frame, 60.4 MiB peak RSS on desktop software renderer |

Final NRO: `build/switch/wodex.nro`, version 0.2.0, 26,355,382 bytes (about 25.1 MiB). Embedded NACP version is verified.

SHA-256: `e782f421d5b2cfc3481777756e24f1edbee484ff44eb60698e96d90997253961`.

Native tests cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. Font tests verify Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection. Rust tests additionally cover BOM and line-ending variations, malformed stream events, state limits, escaped-JSON storage bounds and exhausted saved IDs.

Update tests additionally cover allowed and rejected GitHub redirects, source binding, no chat-credential leakage, release version/digest/size requirements, cancellation cleanup, checksum/size mismatch, NRO/ASET/RomFS bounds and missing assets, tampered stages, actual packaged-NRO replacement in a temporary folder, retained backups, failed installation rollback and failed-rollback recovery. Desktop CTest passes all five suites. The update repository remains unset as requested, and no live release endpoint has been checked.

Physical Switch graphics, controls, keyboard, SD filesystem, self-replacement and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. GitHub Actions is configured but has not run remotely because the project has not been published.
