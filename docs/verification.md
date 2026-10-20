# Verified build

Local verification completed on 2026-10-20.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust behavior and protocol tests | 17 passed |
| Native integration against isolated loopback API | 71 checks passed |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless SDL rendering smoke test | Passed |
| Rust Switch `no_std` + `alloc` cross-compilation | Passed, nightly-2026-10-01 |
| Native Switch compilation and NRO link | Passed without warnings, devkitPro 20260219 |
| Embedded NRO icon and CA bundle | Verified |
| Long-history fixture, 20 turns / about 1.28 MiB text | 0.27 seconds to first frame, 44.6 MiB peak RSS on desktop software renderer |

Final NRO: `build/switch/wodex.nro`, 9,810,030 bytes.

SHA-256: `ca053438c7c74c6084ddfa721aab2fbe22838483cd46bde92fa30a45002c4960`.

Native tests cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. Rust tests additionally cover BOM and line-ending variations, malformed stream events, state limits, escaped-JSON storage bounds and exhausted saved IDs.

Physical Switch graphics, controls, keyboard, SD filesystem and live provider HTTPS behavior have not been exercised on hardware. No live DeepSeek API key was used, and no API credits were spent. GitHub Actions is configured but has not run remotely because the project has not been published.
