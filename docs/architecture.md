# Architecture and Switch builds

Wodex is a native Nintendo Switch homebrew client with a desktop development build. It sends requests directly to the configured DeepSeek-compatible API. It has no application backend or hosted frontend.

## Responsibilities

- `core/` contains the Rust application and conversation engine. The Switch build uses `no_std` plus `alloc`; desktop builds and host tests use the same code with Rust's standard library.
- `core/src/app.rs` owns application actions, navigation, streaming coordination, saving and update decisions. It calls the conversation engine through typed commands and borrows validated state directly. Views never serialize or copy the conversation history to cross a language boundary.
- `core/src/app/view.rs` draws chat, settings, updates, sidebar and the editor at a logical 1280×720 resolution. Interactive regions carry typed actions rather than native closures.
- `core/src/app/editor.rs` owns UTF-8 cursor movement, selection, clipboard insertion and IME composition. Oversized or NUL-containing insertion preserves the existing draft.
- `core/src/app/canvas.rs` owns font and texture handles, bounded caches, UTF-8 wrapping, clipping and rounded geometry. Layouts store byte ranges into their cached text. Only visible paragraph lines create textures, and rounded cards draw only rows inside the viewport.
- `native/src/main.cpp` owns process startup, the SDL event/frame loop, screenshots and teardown. `native/src/render_backend.cpp` supplies synchronous SDL2/SDL2_ttf primitives through a C callback table. Rust releases its font and texture handles before the renderer, SDL or platform services are destroyed.
- `core/src/app/host.rs` owns the Rust storage, transport and updater instances and calls them directly. It also validates the loader-supplied installation path and Switch keyboard input limits. Its native callback table carries only SDL/libnx input, clipboard, keyboard and timing operations; `native/src/host.cpp` translates those calls. Borrowed native buffers are copied immediately when Rust retains them.
- `core/src/backend/storage.rs` owns directory and font selection, file bounds and permissions policy, durable writes, unique recovery copies, Switch backup rotation and rollback. Corrupt saves are preserved as raw bytes, including invalid UTF-8. `native/src/fs_bindings.cpp` normalizes POSIX/newlib flags, stat layouts, offsets and errno without implementing storage algorithms.
- `core/src/backend/http.rs` owns secure HTTP options, headers, callback handling and URL inspection. `transport.rs` validates endpoints and credentials, owns the streaming worker and bounded queue, and converts network/HTTP failures into user messages. The main thread alone calls the conversation engine. `native/src/http_bindings.cpp` translates typed primitives into libcurl calls; HTTPS verification and redirect policy are configured by Rust.
- `core/src/backend/runtime.rs` owns worker closures, join handles, mutex guards and shared-state lifetimes through `core`/`alloc`. `native/src/runtime_bindings.cpp` only forwards native thread and mutex operations, preserving compatibility with the existing libnx runtime.
- `core/src/backend/updater.rs` and `updater/files.rs` own GitHub/repository/redirect policy, download workers, staging, checksum and NRO/ASET/RomFS verification, installation, backup rotation and rollback. A changed or cleared source invalidates earlier checked actions; explicit installation requires an idle chat and a successful save.
- `native/src/platform.cpp` initializes and shuts down libnx services and calls shared-font/system-keyboard APIs. Rust selects resource filenames and prepares the data directory. The bundled CA and Noto font remain embedded in RomFS.
- `native/include/wodex_app.h`, `wodex_host.h`, `wodex_render.h` and `wodex_backend.h` define the application and primitive interfaces. Returned Rust strings/buffers/handles have explicit free functions. Test-only C++ facades in `tests/` call Rust to retain the existing native integration harnesses; those facades and nlohmann/json are excluded from the application and NRO.

Rust's [Nintendo Switch target](https://doc.rust-lang.org/rustc/platform-support/aarch64-nintendo-switch-freestanding.html) has Tier 3 status and no supported upstream `std` backend. Wodex deliberately builds `core` and `alloc` with a pinned nightly compiler. A custom standard-library port could change this choice later; it is independent of the application/view language. libnx and devkitPro own startup, service access, the final ELF link and NRO packaging.

The callback tables keep Rust independent of platform-specific libc layouts, variadic curl options and SDL/libnx structures. They also permit deterministic fault injection without a Rust standard-library port. Native integration tests exercise the production Rust backends with real filesystem, curl, threads and SDL/FreeType bindings.

When installation is requested, Rust transfers an owned pending-update handle to the process shell. Dropping the application cancels and joins its Rust workers and releases font/texture handles. The shell then destroys SDL and Platform, unmounting RomFS. Only after that teardown does it call the Rust installer through filesystem primitives. The pending handle contains metadata and owns no open native resources.

Wiliwili is the interaction reference: a controller-first application with readable text and direct platform integration. Wodex uses SDL2 rather than Wiliwili's Borealis UI library, so its small custom renderer can be built and exercised on desktop with the same native sources. See the official [libnx application template](https://github.com/switchbrew/switch-examples/tree/master/templates/application) and [SDL2 example](https://github.com/switchbrew/switch-examples/tree/master/graphics/sdl2/sdl2-simple) for the underlying homebrew build conventions.

## Chinese font resources

Both builds bundle Noto Sans CJK SC Regular as an OpenType font, together with its SIL Open Font License. The full font is retained rather than a subset of sample characters, so ordinary Chinese prompts and answers do not depend on the console's system language or a developer's installed fonts. The pinned font and attribution are documented in `THIRD_PARTY.md`.

On Switch, SDL2_ttf opens `romfs:/fonts/NotoSansCJKsc-Regular.otf`. On desktop, the build copies resources beside the executable and the application resolves `resources/fonts/NotoSansCJKsc-Regular.otf` through `SDL_GetBasePath`, so launching from a different working directory still finds the font. An explicit desktop `WODEX_FONT` setting takes precedence.

The platform retains system-font fallbacks for development builds whose bundled resources are absent. Normal Switch builds reject missing font or license files, including when Make is invoked directly in the SDK container. The Makefile tracks nested RomFS files and directory changes so replacing a font, adding an asset or removing a resource repackages the NRO.

## Reproduce the Switch build

The verified toolchain is Rust `nightly-2026-10-01` and the official `devkitpro/devkita64:20260219` image, containing devkitA64 GCC 15.2.0 and libnx 4.12.0. The CA bundle, Chinese font and font license are vendored in this repository. The vendored C++ JSON header is used only by desktop tests.

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

The Rust Switch-target archive and complete native NRO have been built with the toolchain above. Native integration tests use a local mock API to exercise the transport and persistence without a paid API key. [GitHub Actions](https://github.com/old-new-things/wodex/actions/workflows/build.yml) repeats Rust checks, the desktop build and the Switch NRO build on pushes and pull requests. Stable release tags reuse the build workflow before publishing verified assets.

Execution on physical Switch hardware remains necessary to verify graphics performance, bundled-font rendering, the software keyboard and networking under the device's firmware and homebrew launch mode. The generated NRO is a build-verified artifact, not evidence of a completed hardware test.
