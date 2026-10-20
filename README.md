# Wodex

A native DeepSeek chat client for Nintendo Switch homebrew, with a desktop build for development. Inspired by wiliwili's controller navigation and readable handheld interface.

The app streams replies, saves conversations, supports thinking mode, includes a Chinese-capable font, and uses the Switch system keyboard. It also checks for verified releases and can replace its Switch NRO on exit. A real ARM64 `.nro` has been cross-compiled and linked. Physical Switch testing and a live request with your own DeepSeek key are still required.

![Desktop preview](docs/preview.png)

## Use on Switch

1. Copy `build/switch/wodex.nro` to `sdmc:/switch/wodex/wodex.nro`.
2. Launch hbmenu with full application memory through your homebrew environment. For a typical title-override setup, hold **R** while launching a game, then select Wodex.
3. Open **Settings**, enter your [DeepSeek API key](https://platform.deepseek.com/), and return to chat.
4. Press **X** to write, confirm the keyboard, then select **Send**.

DeepSeek runs remotely; the Switch is the chat client. Wodex connects directly to your configured provider. There is no application server to deploy.

| Control | Action |
| --- | --- |
| D-pad | Move focus |
| A | Select |
| B | Back; stop an active reply |
| X | Write a message |
| Y | Show/hide reasoning text |
| L / R | Previous/next saved conversation |
| ZL / ZR or right stick | Scroll |
| − | Settings |
| + | Save and exit |

Touch is supported. Settings provide a configurable model name, endpoint, thinking mode, response token limit, and assistant instructions. The default model is `deepseek-flash`, matching the [current DeepSeek API](https://api-docs.deepseek.com/). Change the model name in Settings when your provider changes its available models.

## Build and run on desktop

Requirements: Rust/Cargo, a C++17 compiler, CMake 3.20+, SDL2 **2.0.18+**, SDL2_ttf **2.0.18+**, libcurl **7.62+**, and Python 3 for integration tests. SDL2_ttf needs FreeType. Ninja is optional.

On Debian/Ubuntu:

```sh
sudo apt-get install build-essential cmake ninja-build pkg-config \
  libsdl2-dev libsdl2-ttf-dev libcurl4-openssl-dev fonts-dejavu-core
cmake -S . -B build/desktop -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/desktop --parallel
./build/desktop/wodex
```

On macOS, install SDL2, SDL2_ttf, FreeType, CMake, and Ninja with your package manager. If SDL2_ttf is unavailable, `bash scripts/setup-desktop.sh` downloads its checksum-verified 2.24.0 source for the CMake fallback. For installations that CMake cannot discover, pass `-DCMAKE_PREFIX_PATH=/your/package/prefix` or `-DSDL2_DIR=/your/prefix/lib/cmake/SDL2`.

```sh
bash scripts/setup-desktop.sh  # only needed without system SDL2_ttf
cmake -S . -B build/desktop -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/desktop --parallel
./build/desktop/wodex --demo
```

Desktop controls: arrows and Enter select; **X** opens the composer; **Ctrl/Cmd + Enter** sends; **Esc** goes back/stops; **F1** opens Settings; **[ / ]** changes chats; **Page Up/Down**, the mouse wheel, and **End** navigate history. The text editor supports UTF-8, IME input, cursor movement, selection of all text, paste, and Shift+Enter for newlines. Desktop gamepads use SDL's standard button mapping. Keep the generated `resources/` directory beside the desktop executable when moving it.

`--demo` displays an explicitly labeled sample conversation without network requests or saving history. `--screenshot FILE.bmp` renders one frame and exits; combine it with `SDL_VIDEODRIVER=dummy` for a headless preview.

| Environment variable | Purpose |
| --- | --- |
| `DEEPSEEK_API_KEY` | Desktop session key override; never writes that override to disk |
| `WODEX_DATA_DIR` | Desktop data directory override |
| `WODEX_FONT` | Desktop TrueType/OpenType font override; otherwise uses the bundled CJK font |
| `WODEX_CA_BUNDLE` | Desktop custom CA bundle for HTTPS verification |

## Build for Switch

Use the official devkitPro Switch SDK and Rust `nightly-2026-10-01` with `rust-src`:

```sh
rustup toolchain install nightly-2026-10-01 --profile minimal --component rust-src
export DEVKITPRO=/opt/devkitpro
bash scripts/build-switch.sh
```

Required SDK packages: `switch-dev`, `switch-sdl2`, `switch-sdl2_ttf`, `switch-curl`, and `switch-pkg-config`. Output: `build/switch/wodex.nro`.

The verified SDK image is `devkitpro/devkita64:20260219`. [Architecture and build details](docs/architecture.md) include the exact two-stage commands used to build the NRO on a host with Rust and a container engine. GitHub Actions is configured to build both targets and upload the Switch artifact after the project is pushed to GitHub.

## Data and behavior

- Switch data lives in `sdmc:/switch/wodex/state.json`. Desktop defaults to `~/Library/Application Support/Wodex/` on macOS or `$XDG_DATA_HOME/wodex/` / `~/.local/share/wodex/` on Linux.
- History and keys entered in Settings are stored **unencrypted** locally. Desktop directories/files use user-only permissions; an SD card does not provide that isolation. The environment key override is session-only.
- Storage is versioned and bounded. Writes preserve the previous save on interruption. Unreadable saves are copied to a unique `state.json.recovery-N` file before starting fresh; they are never silently overwritten.
- Interrupted replies retain their partial text and can be retried from Settings. Incomplete assistant replies and prior reasoning are omitted from subsequent API context. Sending is serialized so responses cannot cross between conversations.
- HTTPS certificates are verified, redirects are refused, and provider error bodies are not displayed. Plain HTTP is limited to loopback addresses for local tests.
- Limits: 32 conversations, 128 messages per conversation, 64 KiB per prompt, 512 KiB combined answer/reasoning, 2 MiB total message text, and 4 MiB saved JSON. Delete an older conversation when storage is full. Long answers remain fully scrollable.

Current scope is text chat. Markdown is displayed as readable text; tool execution, attachments, image generation, search, and account login are outside this release. Switch keyboard input follows libnx's size limits; a very long existing draft cannot be reopened without shortening it.

## Fonts and Chinese text

Wodex includes **Noto Sans CJK SC Regular 2.004**, an unmodified, openly licensed OpenType font. It supports broad Simplified Chinese, Traditional Chinese, Japanese, Korean and Latin text. Regional glyph shapes follow the Simplified Chinese variant; it does not contain every Unicode character or all emoji. Its license and provenance are included in `romfs/fonts/`.

The full 16,437,364-byte font is embedded in the Switch NRO's RomFS, so Chinese replies do not depend on the console's language or a separately installed font. A UI-text-only subset would miss characters in model replies. Desktop builds copy the same font into `resources/fonts/` beside the executable, and locate it independently of the working directory.

SDL_ttf/FreeType reads UTF-8 text using the font's stream API; Wodex does not manually copy the full file for every point size. Text layout and visible texture caches remain bounded. If the bundled font is absent at runtime, Switch falls back to its system-language shared font and desktop falls back to a system font. This is a whole-font fallback, not a per-character fallback chain. `WODEX_FONT` overrides the desktop choice, so that font needs the glyph coverage you want.

## Self-updates

Open **Settings → Updates**, set a public GitHub `owner/repo` release source, then choose **Check for updates**, **Download**, and **Install and exit**. The release source is unset by default; configure it after choosing where to publish releases. Checks are manual. Desktop builds can check release metadata; self-installation is supported on Switch.

Wodex downloads a newer stable `wodex.nro` to a sibling temporary file and verifies the release's advertised size, SHA-256, executable headers and embedded assets. Installing saves conversations, closes the interface and embedded resources, preserves a launchable `.previous.nro`, and replaces the actual NRO path supplied by the loader. Reopen Wodex from hbmenu afterward. A normal exit never installs a downloaded update.

The SD-card replacement uses two renames, so power interruption can require restoring the `.previous.nro` backup manually. [Update and recovery instructions](docs/updates.md) explain that boundary and the included GitHub release workflow. No repository or release is published by local builds.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cmake --build build/desktop --parallel
ctest --test-dir build/desktop --output-on-failure
```

The integration tests start isolated loopback mock servers. They drive the Rust application through SDL input and real curl transport, and test fragmented UTF-8 streams, reasoning, usage, retry, HTTP errors, cancellation, queue limits, and durable storage. Update tests verify downloads, checksums, release source restrictions, cancellation, backup preservation and rollback failures. Font tests exercise the production Rust renderer with real Chinese and mixed-language fonts; UI smoke tests render chat and update pages. No tests require a real API key or spend API credits.

## Source map

- `core/src/app/`: Rust views, editor, text layout/cache, platform interfaces, and tests.
- `core/src/app.rs`: Rust application actions, navigation, chat/update coordination, and saving.
- `core/src/`: conversation state, validation, persistence model, SSE parser, release validation, and C ABI.
- `native/`: SDL/TTF primitives, input normalization, libnx services, storage, curl/update workers, and process lifecycle.
- `scripts/`: desktop dependency fallback and Switch build.
- `tests/`: native integration suite and local mock API.
- `romfs/`: bundled CJK font, license/attribution, and public CA certificates.

Wodex code is MIT licensed. Third-party notices are in [THIRD_PARTY.md](THIRD_PARTY.md).
