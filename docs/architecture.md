# Architecture and Switch builds

Wodex is a native Nintendo Switch homebrew client with a desktop development build. It sends requests directly to the configured DeepSeek-compatible API. It has no application backend or hosted frontend.

## Responsibilities

- `core/` contains the Rust application and conversation engine. The Switch build uses `no_std` plus `alloc`; desktop builds and host tests use the same code with Rust's standard library.
- `core/src/app.rs` owns application actions, navigation, streaming coordination, saving and update decisions. It calls the conversation engine through typed commands and borrows validated state directly. Views never serialize or copy the conversation history to cross a language boundary.
- `core/src/app/view.rs` draws chat, settings, updates, sidebar and the editor at a logical 1280×720 resolution. Interactive regions carry typed actions rather than native closures.
- `core/src/app/editor.rs` owns UTF-8 cursor movement, selection, clipboard insertion and IME composition. Oversized or NUL-containing insertion preserves the existing draft.
- `core/src/app/canvas.rs` owns font and texture handles, bounded caches, UTF-8 wrapping, clipping and rounded geometry. Layouts store byte ranges into their cached text. Only visible paragraph lines create textures, and rounded cards draw only rows inside the viewport.
- `native/src/main.cpp` owns process startup, the SDL event/frame loop, screenshots and teardown. `native/src/render_backend.cpp` supplies synchronous SDL2/SDL2_ttf primitives through a C callback table. Rust releases its font and texture handles before the renderer, SDL or platform services are destroyed.
- `core/src/app/host.rs` and `native/src/host.cpp` adapt storage, normalized input, network events, the system keyboard and updater operations through a C callback table. Native buffers are borrowed until the next call and copied immediately when Rust retains them. Corrupt saved data is preserved as raw bytes, including invalid UTF-8.
- `native/src/platform.cpp` owns Switch services, bundled-font selection, the system keyboard and local storage. The desktop implementation provides the same interface for development and testing.
- `native/src/transport.cpp` runs libcurl on a worker thread and passes bounded data chunks to the main thread. Only the main thread calls the Rust engine. HTTPS verifies certificates; the Switch package embeds a public CA bundle in RomFS.
- `native/src/updater.cpp` owns download workers, native filesystem checks and installation/rollback. Rust decides when checking, downloading or installation may begin. A changed or cleared release source invalidates earlier checked actions; explicit installation requires an idle chat and a successful save.
- `native/include/wodex_app.h`, `wodex_host.h` and `wodex_render.h` define the application interface. Callback tables contain only synchronous primitive operations and borrowed native contexts. Rust owns the application and all view state. The existing `wodex_core.h` interface remains available to native protocol/update tests; returned Rust strings are released with `wodex_string_free`.

Rust's [Nintendo Switch target](https://doc.rust-lang.org/rustc/platform-support/aarch64-nintendo-switch-freestanding.html) has Tier 3 status and no supported upstream `std` backend. Wodex deliberately builds `core` and `alloc` with a pinned nightly compiler. A custom standard-library port could change this choice later; it is independent of the application/view language. libnx and devkitPro own startup, service access, the final ELF link and NRO packaging.

The SDL/TTF callback interface keeps the Rust renderer independent of target-specific binding generation and lets its layout/resource behavior run against a small deterministic test backend. Native font tests use these same Rust drawing methods with real SDL/FreeType. Networking and platform storage retain their existing tested native implementations behind the host interface.

Wiliwili is the interaction reference: a controller-first application with readable text and direct platform integration. Wodex uses SDL2 rather than Wiliwili's Borealis UI library, so its small custom renderer can be built and exercised on desktop with the same native sources. See the official [libnx application template](https://github.com/switchbrew/switch-examples/tree/master/templates/application) and [SDL2 example](https://github.com/switchbrew/switch-examples/tree/master/graphics/sdl2/sdl2-simple) for the underlying homebrew build conventions.

## Chinese font resources

Both builds bundle Noto Sans CJK SC Regular as an OpenType font, together with its SIL Open Font License. The full font is retained rather than a subset of sample characters, so ordinary Chinese prompts and answers do not depend on the console's system language or a developer's installed fonts. The pinned font and attribution are documented in `THIRD_PARTY.md`.

On Switch, SDL2_ttf opens `romfs:/fonts/NotoSansCJKsc-Regular.otf`. On desktop, the build copies resources beside the executable and the application resolves `resources/fonts/NotoSansCJKsc-Regular.otf` through `SDL_GetBasePath`, so launching from a different working directory still finds the font. An explicit desktop `WODEX_FONT` setting takes precedence.

The platform retains system-font fallbacks for development builds whose bundled resources are absent. Normal Switch builds reject missing font or license files, including when Make is invoked directly in the SDK container. The Makefile tracks nested RomFS files and directory changes so replacing a font, adding an asset or removing a resource repackages the NRO.

## Reproduce the Switch build

The verified toolchain is Rust `nightly-2026-10-01` and the official `devkitpro/devkita64:20260219` image, containing devkitA64 GCC 15.2.0 and libnx 4.12.0. The JSON header, CA bundle, Chinese font and font license are vendored in this repository.

When the host has a devkitPro installation, install `switch-dev`, `switch-sdl2`, `switch-sdl2_ttf`, `switch-curl` and `switch-pkg-config`, then run:

```sh
rustup toolchain install nightly-2026-10-01 --profile minimal --component rust-src
export DEVKITPRO=/opt/devkitpro
bash scripts/build-switch.sh
```

The same build works in two stages when the host has Rust and a container engine but no Switch SDK. Run these commands from the repository root.

First, build the ARM64 Rust static library on the host:

```sh
rustup toolchain install nightly-2026-10-01 --profile minimal --component rust-src
CARGO_TARGET_DIR="$PWD/target" \
  RUSTFLAGS='-C relocation-model=pic -C target-cpu=cortex-a57' \
  cargo +nightly-2026-10-01 build \
  -Zbuild-std=core,alloc,compiler_builtins \
  --locked --release --no-default-features \
  --target aarch64-nintendo-switch-freestanding \
  --package wodex-core
```

Then compile and link the C++ shell with the official SDK container:

```sh
docker pull docker.io/devkitpro/devkita64:20260219
docker run --rm --network=none \
  --volume "$PWD:/workspace" \
  --workdir /workspace \
  docker.io/devkitpro/devkita64:20260219 \
  make -f scripts/Makefile.switch -j4
```

Podman's Docker-compatible command also works. The native link consumes the already-built Rust archive; it does not require Rust inside the container. Repeat both stages after changing Rust code, and repeat the second stage after changing native code. An installed compiler with the identical nightly manifest can be selected through `WODEX_RUST_TOOLCHAIN` when using `build-switch.sh`.

Outputs are `build/switch/wodex.nro`, `wodex.elf`, `wodex.nacp`, `wodex.map` and `wodex.lst`. Copy the NRO to `sdmc:/switch/wodex/wodex.nro`. The application stores its own data under `sdmc:/switch/wodex/`.

## Verification boundary

The Rust Switch-target archive and complete native NRO have been built with the toolchain above. Native integration tests use a local mock API to exercise the transport and persistence without a paid API key. GitHub Actions repeats Rust checks, the desktop build and the Switch NRO build. CI has been configured locally; it runs after the repository is pushed to GitHub.

Execution on physical Switch hardware remains necessary to verify graphics performance, bundled-font rendering, the software keyboard and networking under the device's firmware and homebrew launch mode. The generated NRO is a build-verified artifact, not evidence of a completed hardware test.
