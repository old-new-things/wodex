# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust behavior and protocol tests | 17 passed |
| Native integration against isolated loopback API | 71 checks passed |
| Bundled CJK glyph coverage and mixed-language rendering | 76 checks passed |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless SDL rendering smoke test | Passed |
| Desktop resource discovery from a different working directory | Passed |
| Rust Switch `no_std` + `alloc` cross-compilation | Passed, nightly-2026-10-01 |
| Native Switch compilation and NRO link | Passed without warnings, devkitPro 20260219 |
| Embedded NRO icon, CA bundle, Noto CJK font, OFL and attribution | Verified byte for byte |
| Nested RomFS resource rebuilds and required-font guard | Passed |
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.45 seconds to first frame, 60.4 MiB peak RSS on desktop software renderer |

Final NRO: `build/switch/wodex.nro`, 26,257,078 bytes (about 25.0 MiB).

SHA-256: `e933b93ca403c9cd216276ed5c98764688984e42f6868cf07cf2ba0655754ba1`.

Native tests cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. Font tests verify Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection. Rust tests additionally cover BOM and line-ending variations, malformed stream events, state limits, escaped-JSON storage bounds and exhausted saved IDs.

Physical Switch graphics, controls, keyboard, SD filesystem and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. GitHub Actions is configured but has not run remotely because the project has not been published.
