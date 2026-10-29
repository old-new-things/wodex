# Verified build

Verification updated on 2026-10-29.

| Check | Result |
| --- | --- |
| Rust formatting and Clippy with warnings denied | Passed |
| Rust application, backend, editor, renderer, image decoding, protocol, update-feed validation and hashing tests | 197 passed |
| Compiled nightly/version identity and repeat-update detection | Passed with both publication identities; partial or invalid build metadata rejected |
| Native integration against isolated loopback API | 91 checks passed, including strict OpenAI-compatible requests with both token-limit fields |
| Rust application through real SDL input, rendering, curl and save/reload | 35 checks passed |
| Native SDL/libnx callback interface and input normalization | 18 checks passed |
| Bundled CJK glyph coverage and mixed-language rendering | 216 checks passed, including styled fonts, the monospace code face and baseline metrics |
| Desktop CMake Release build | Passed, AppleClang 17 + Rust 1.99 |
| Headless chat and update-page rendering smoke tests | Passed; Versions/Nightly and empty-source screens visually inspected |
| HOME Menu UI, installation ordering and failure handling | 8 UI tests and 11 backend tests passed; desktop HOME Menu page rendered and visually inspected |
| PC transfer schemas, server access/framing/lifetime and application import policy | 10 state, 16 backend and 11 application tests passed |
| PC browser protocol through the real Rust app and native sockets | 176 checks passed, including provider selection, saved-key exports, independent imports, confirmation, rejection, recovery and responsive shutdown |
| PC transfer native UI | Ready, active, confirmation and Settings screens rendered and visually inspected |
| Embedded TypeScript browser client | Strict compilation and 26 behavior tests passed; CI checks regenerated JavaScript and executes the tests |
| Key-free forwarder validation and reproducibility | 6 Python tests passed; offline rebuild matched all committed template bytes |
| Native forwarder header encryption | Pinned libnx AES-XTS encrypted six sectors with a public test key; independent mbedTLS decryption reproduced the original header |
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
| Switch Album/applet launch guard | Compiled against libnx; rejected before Platform/SDL/font initialization, with full-memory launch instructions; console behavior still needs hardware verification |
| Embedded NRO icon, CA bundle, fonts, HOME Menu template, local transfer page, manifests and license notices | All thirteen RomFS files and the icon verified byte for byte |
| Nested RomFS resource rebuilds and required-font guard | Passed |
| Long-history fixture, 20 turns / about 1.28 MiB text, bundled CJK font | 0.10 seconds to render and exit, 50.4 MiB peak RSS on desktop software renderer |
| Maximum-size 512 KiB reply, 262,141 short lines | 0.124 seconds for startup/render/screenshot, 51.2 MiB peak RSS; final marker visible |
| Maximum-size 512 KiB reply, one long paragraph | 0.075 seconds for startup/render/screenshot, 48.8 MiB peak RSS; final marker visible |

Local NRO: `build/switch/wodex.nro`, version 0.2.1, 27,145,050 bytes (about 25.9 MiB). The embedded NACP version is verified. This local build uses the development identity; CI embeds the actual publication identity.

SHA-256: `9129ea1e30ef03d19a71efb719133bb08aa431a2eabe1b0ee5b8d77820f42626`. Published CI binaries embed their actual workflow identity and have their own download checksums.

Native tests call the production Rust transport and storage through test-only C++ facades. They cover actual curl transfer, every-byte fragmented UTF-8 SSE, reasoning/content/usage, retry context, authentication errors without response-body leakage, redirect refusal, truncation, cancellation, queue limits, private desktop writes, unique recovery backups, and persistence roundtrips. The application test drives the production Rust application through SDL compose, editor cancellation, send, retry, update navigation and exit/reload, including an environment-only API key. Adapter tests verify normalized SDL/libnx input and clipboard values. Font tests exercise the Rust filename selection and renderer for Simplified/Traditional Chinese, Latin, kana, Hangul, punctuation and representative rare CJK characters, wrapping and clipping at four UI point sizes, font overrides and resource selection.

Rust tests additionally cover UTF-8 editing and IME composition, clipboard limits, environment-key persistence, streaming/HTTP completion ordering, stale update actions after changing the source or channel, failed-save installation guards, cache/resource lifetimes and bounds, long-history scrolling, BOM/line-ending variations, malformed events, state limits, escaped-JSON storage bounds and exhausted saved IDs. Markdown checks cover incomplete fences and inline syntax, nested lists/styles, CJK emphasis, table alignment and responsive layout, code indentation and combining marks, wrapping across style boundaries, repeated-header bounds and compact plain fallback. A 20-million-pixel rounded card emits only its 50 visible row callbacks; a far-scrolled 60,000-line paragraph draws its visible tail.

Backend fault-injection tests cover interrupted/short writes, byte-exact corrupt storage, failed flush cleanup, Switch save rotation, rollback and failed-rollback recovery, exclusive recovery files, permissions, directory/font resolution and font symlinks. HTTP/runtime tests cover suppressed error bodies, redirect/interim headers, header limits, sink failures, cancellation, queue bounds, mutex/worker ownership and failed-spawn cleanup. The same backend code cross-compiles with `no_std` and `alloc`, using native primitive tables rather than a Rust `std` port.

Image tests cover PNG grayscale/palette/transparency, baseline/progressive/grayscale JPEG, malformed headers and entropy, PNG chunk lengths/order and metadata filtering, source/pixel limits, downsampling, credential-free requests, failed downloads, cancellation and bounded texture lifetime. A delayed completion for the same URL after changing conversations cannot upload stale pixels. Image fixtures use an isolated loopback transport wrapper; ordinary screenshot and demo modes remain offline.

Update tests additionally cover redirect rejection, exact Pages project/channel URL binding, mixed-case repository paths, no chat-credential leakage, feed schema/digest/size requirements, nightly timestamp/commit/ID agreement, equal-time ordering, repeat-update suppression, timestamp ordering across different semantic versions, returning from Nightly to lower/equal/higher Versions builds, Versions-to-Versions downgrade rejection, cancellation cleanup, checksum/size mismatch, NRO/ASET/RomFS bounds and missing assets, tampered stages, actual packaged-NRO replacement in a temporary folder, retained backups, failed installation rollback and failed-rollback recovery. Desktop CTest includes twelve suites, including PC transfer, HOME Menu rendering, Markdown fixtures and image integration. New installations use the official Pages source with Versions selected; existing empty sources remain disabled. Nightly is an explicit saved choice.

HOME Menu tests validate the pinned template, dependent hashes, content metadata and title ID. Failure injection exercises each pre-registration operation, partial placeholder creation/registration, uncertain metadata writes, uncertain HOME Menu registration, collisions and failed cleanup. Referenced or pre-existing content is retained when ownership or registration cannot be established. UI tests cover failed saves, demo mode, busy operations, stale actions, exit blocking and readable recovery messages. The 177,172-byte template's SHA-256 is `683bfe7ececc3ee317cd64b3c951c43df85353f050dcb9884fee2d719a635c92`; source and generator checksums are recorded in its manifest. Its return path requests normal application exit without waiting for foreground focus, including after older NROs return. The actual Switch service calls are compile-checked; host mocks cannot prove their behavior on a console.

PC transfer tests exercise real HTTP connections against the application, including inaccessible unauthenticated exports, Host/Origin checks, channel isolation, Unicode imports, saved-key inclusion and environment-key exclusion. Fault injection covers malformed framing, partial writes, expiry, rejected uploads with unread request bodies, import save failures before and after commit, and unknown storage outcomes that lock further writes. The generated browser script is executed in a Node VM with the actual HTML control structure and mocked DOM, HTTP responses and timers. Its 26 tests cover automatic configuration loading, masked-key clearing, complete form payloads, advanced settings, Unicode byte limits, device confirmation/rejection, dirty-draft reload, competing imports, failed loads and stale responses. Native screenshots cover transfer setup, active pairing, import confirmation, and both provider Settings layouts; browser visual interaction was not exercised.

Tokn was refreshed to [`d546ef2d`](https://github.com/tokn-ai/tokn/tree/d546ef2d306e9a952a17eae8a58366483832da05) on 2026-10-29. The breaking SDK refactor replaces `DirectClient` with the shared `Client` pipeline; Wodex imports only the endpoint/core schema crates, whose sources, manifests and inherited dependencies are unchanged from its previous pin. The update therefore requires no application migration. Formatting, Clippy, all 197 application and 12 publisher tests, all twelve desktop suites, and the Switch `no_std` build passed with the new pin.

Tokn integration uses pinned `no_std` endpoint/core crates for typed Chat Completions requests and streamed chunks. Tests cover DeepSeek options, generic requests without vendor-specific fields, both token-limit parameter names, legacy configuration defaults, incomplete generic setup, provider credential reset, per-provider environment keys, and imports that preserve saved/runtime credential separation. The generic loopback mock rejects unsupported request fields and verifies its distinct Bearer key. The final target dependency graph contains no `std`, Tokio or Reqwest dependency. Both provider settings screens were rendered through the native SDL harness and visually checked at 1280×720.

An intermittent freeze was reported after launching through Album. Startup now rejects `AppletType_LibraryApplet` before initializing application services, SDL or font caches, and uses the existing console error screen to explain title override. Low-memory pressure is a plausible cause, but the reported HOME-button freeze has not been reproduced or conclusively attributed to memory or updating. Full-memory Application/SystemApplication launches remain accepted.

Physical Switch graphics, controls, keyboard, SD filesystem, self-replacement, HOME Menu installation/launch/exit, PC-to-Switch LAN transfer and live provider HTTPS behavior have not been exercised on hardware. No live provider API key was used, and no API credits were spent. Remote build results and downloadable artifacts are recorded in [GitHub Actions](https://github.com/old-new-things/wodex/actions/workflows/build.yml); the table above records the local verification. The separate [publication workflow](https://github.com/old-new-things/wodex/actions/workflows/publish.yml) verifies the served page, catalog, feed and newest channel downloads before reporting success.
