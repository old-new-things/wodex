# Architecture and Switch builds

Wodex is a native Nintendo Switch homebrew client with a desktop development build. It sends requests directly to the configured DeepSeek-compatible API. Its interface runs locally; a static GitHub Pages site distributes builds and the Versions and Nightly update feeds.

## Responsibilities

- `core/` contains the Rust application and conversation engine. The Switch build uses `no_std` plus `alloc`; desktop builds and host tests use the same code with Rust's standard library.
- `core/src/app.rs` owns application actions, navigation, streaming coordination, saving and update decisions. It calls the conversation engine through typed commands and borrows validated state directly. Views never serialize or copy the conversation history to cross a language boundary.
- `core/src/app/view.rs` draws chat, settings, updates, HOME Menu installation, sidebar and the editor at a logical 1280×720 resolution. Interactive regions carry typed actions rather than native closures.
- `core/src/app/editor.rs` owns UTF-8 cursor movement, selection, clipboard insertion and IME composition. Oversized or NUL-containing insertion preserves the existing draft.
- `core/src/app/canvas.rs` owns font and texture handles, bounded caches, UTF-8 wrapping, clipping and rounded geometry. Layouts store byte ranges into their cached text. Only visible paragraph lines create text textures, and rounded cards draw only rows inside the viewport. Image cards collect visible download candidates, fit decoded RGBA pixels to stable rectangles, and share bounded image textures by source URL.
- `core/src/app/markdown.rs` converts a pinned `no_std` CommonMark parser's events into bounded, flat blocks and styled spans. `rich_text.rs` measures and caches their layout for assistant replies and thinking text. Code uses fixed cells measured from the bundled font; wide tables become labeled rows. Excessive structure uses compact plain-text layout without losing source content. Native SDL_ttf bindings only supply font styles and metrics; stored messages and API requests retain their original Markdown.
- `native/src/main.cpp` owns process startup, the SDL event/frame loop, screenshots and teardown. `native/src/render_backend.cpp` supplies synchronous SDL2/SDL2_ttf primitives through a C callback table. Rust releases its font and texture handles before the renderer, SDL or platform services are destroyed.
- `core/src/app/host.rs` owns the Rust storage, transport, image-loader, updater and HOME Menu installer instances and calls them directly. It also validates the loader-supplied installation path and Switch keyboard input limits. Its native callback table carries only SDL/libnx input, clipboard, keyboard and timing operations; `native/src/host.cpp` translates those calls. Borrowed native buffers are copied immediately when Rust retains them.
- `core/src/backend/storage.rs` owns directory and font selection, file bounds and permissions policy, durable writes, unique recovery copies, Switch backup rotation and rollback. Corrupt saves are preserved as raw bytes, including invalid UTF-8. `native/src/fs_bindings.cpp` normalizes POSIX/newlib flags, stat layouts, offsets and errno without implementing storage algorithms.
- `core/src/backend/http.rs` owns secure HTTP options, headers, callback handling and URL inspection. `transport.rs` validates endpoints and credentials, owns the streaming worker and bounded queue, and converts network/HTTP failures into user messages. The main thread alone calls the conversation engine. `native/src/http_bindings.cpp` translates typed primitives into libcurl calls; HTTPS verification and redirect policy are configured by Rust.
- `core/src/backend/images.rs` owns direct HTTPS image policy, a single download/decode worker, source and output bounds, PNG metadata filtering, and PNG/JPEG conversion to RGBA. The `zune` decoders build without `std`. Image requests never borrow chat credentials or messages. The main thread uploads completed RGBA pixels through an SDL texture callback; native code performs no image parsing. The app cancels pending images when leaving their conversation and avoids stale results, hidden-image downloads, and repeated failure requests. Demo and command-line screenshot modes disable image loading.
- `core/src/backend/runtime.rs` owns worker closures, join handles, mutex guards and shared-state lifetimes through `core`/`alloc`. `native/src/runtime_bindings.cpp` only forwards native thread and mutex operations, preserving compatibility with the existing libnx runtime.
- The HOME Menu backend validates the bundled forwarder, prepares content metadata, sequences installation and cleanup, and owns the worker and status in Rust. `native/src/home_menu_bindings.cpp` exposes the necessary libnx services and header encryption primitives. The installed forwarder launches the canonical NRO, independently of its app version or update channel. [HOME Menu documentation](home-menu.md) describes the package and installation constraints.
- `core/src/backend/updater.rs` and `updater/files.rs` own GitHub Pages source policy, download workers, staging, checksum and NRO/ASET/RomFS verification, installation, backup rotation and rollback. Metadata and NRO requests use exact project URLs over HTTPS and reject redirects. A changed source or channel invalidates earlier checked actions; explicit installation requires an idle chat and a successful save.
- `native/src/platform.cpp` initializes and shuts down libnx services and calls shared-font/system-keyboard APIs. Rust selects resource filenames and prepares the data directory. The bundled CA and Noto font remain embedded in RomFS.
- `native/include/wodex_app.h`, `wodex_host.h`, `wodex_render.h` and `wodex_backend.h` define the application and primitive interfaces. Returned Rust strings/buffers/handles have explicit free functions. Test-only C++ facades in `tests/` call Rust to retain the existing native integration harnesses; those facades and nlohmann/json are excluded from the application and NRO.
- `tools/publish-pages/` is a separate host Rust tool using `std`. It verifies build bundles and previous publication state, enforces immutable versions and retention bounds, and writes the static download page, catalog and version feed. It is not linked into the Switch application.
- `scripts/pages-state.py` coordinates publication history through the GitHub Actions API. It validates snapshot producers and accepted Pages deployments, safely extracts bounded snapshots, and keeps the current and preceding recovery artifacts after deployment. It never copies GitHub actor metadata into the public site.

Rust's [Nintendo Switch target](https://doc.rust-lang.org/rustc/platform-support/aarch64-nintendo-switch-freestanding.html) has Tier 3 status and no supported upstream `std` backend. Wodex deliberately builds `core` and `alloc` with a pinned nightly compiler. A custom standard-library port could change this choice later; it is independent of the application/view language. libnx and devkitPro own startup, service access, the final ELF link and NRO packaging.

The callback tables keep Rust independent of platform-specific libc layouts, variadic curl options and SDL/libnx structures. They also permit deterministic fault injection without a Rust standard-library port. Native integration tests exercise the production Rust backends with real filesystem, curl, threads and SDL/FreeType bindings.

When installation is requested, Rust transfers an owned pending-update handle to the process shell. Dropping the application cancels and joins its Rust workers and releases font/texture handles. The shell then destroys SDL and Platform, unmounting RomFS. Only after that teardown does it call the Rust installer through filesystem primitives. The pending handle contains metadata and owns no open native resources.

Wiliwili is the interaction reference: a controller-first application with readable text and direct platform integration. Wodex uses SDL2 rather than Wiliwili's Borealis UI library, so its small custom renderer can be built and exercised on desktop with the same native sources. See the official [libnx application template](https://github.com/switchbrew/switch-examples/tree/master/templates/application) and [SDL2 example](https://github.com/switchbrew/switch-examples/tree/master/graphics/sdl2/sdl2-simple) for the underlying homebrew build conventions.

## Build publication and update selection

GitHub Pages serves two build channels. Successful `main` pushes, daily runs at 00:17 UTC, and manual runs on `main` publish timestamped nightlies. Exact `vX.Y.Z` tags publish explicit versions only when the tag matches `VERSION` and its commit is reachable from `main`. All publications depend on successful Rust, desktop, Switch and packaged-updater checks. The build workflow dispatches a separate publication workflow on `main` with the source run and attempt; the latter validates the source and reads its artifact. This keeps tag and scheduled deployments on the working Pages dispatch path while catalog metadata retains the actual source commit. Other branches and pull requests produce CI artifacts only. No GitHub Releases are created.

The publisher retains up to seven nightlies and ten versions under a 450 MiB download budget, preserving the newest build in each channel when additional pruning is needed. Version digest records remain after files are pruned, preventing a version from later referring to different bytes. Downloads include checksums and licensing material.

Serialized publication jobs restore the latest accepted deployment snapshot, validate its files, add the incoming build and deploy a complete replacement site. Live verification compares the catalog, feed, page and newest downloads before the workflow succeeds or prunes recovery state. An accepted snapshot survives a later verification failure, preserving intended versions and digest locks for the next publication. Attempt-specific artifacts preserve the current and preceding recovery snapshots for 90 days, refreshed by successful daily publications. An expired latest snapshot, or a live catalog without trusted recovery state, stops publication rather than erasing history. See [publication recovery](updates.md#publication-state-and-recovery).

The application's manual updater defaults to `updates/stable.json`; optional Nightly uses `updates/nightly.json`. Both strict schemas contain `schema_version: 1`, a matching channel and a required nullable `build`. A populated build contains `version`, `url`, `size`, `sha256` and `notes`. Nightly additionally carries `id`, `timestamp` and `commit`, validated against each other and the exact `builds/nightly/<id>/wodex.nro` path. The stable feed keeps its original fields for older clients and uses `builds/version/X.Y.Z/wodex.nro`.

`scripts/pages-build.py` derives the source workflow attempt's timestamp and commit before cross-compilation. `core/build.rs` validates the three build identity environment variables and embeds them for the `no_std` application. Publication compares the artifact's CI-only identity file against independent source-run metadata. Nightly selection compares timestamps independently of semantic versions, with build IDs breaking equal-time ties between nightlies. Returning from an installed nightly to Versions allows the latest published version even when its number is lower; installed Versions builds still require a newer semantic version. The selected download undergoes the same staging, checksum, executable validation and post-teardown installation path described above.

Generated site metadata excludes developer identities and local paths. Builds remap the developer's own compiler paths; prebuilt SDK libraries can retain upstream diagnostic paths and required credits. GitHub's public push and workflow activity may still identify accounts; omitting Releases does not make repository activity anonymous.

## Chinese font resources

Both builds bundle Noto Sans CJK SC Regular as an OpenType font, together with its SIL Open Font License. The full font is retained rather than a subset of sample characters, so ordinary Chinese prompts and answers do not depend on the console's system language or a developer's installed fonts. The pinned font and attribution are documented in `THIRD_PARTY.md`.

On Switch, SDL2_ttf opens `romfs:/fonts/NotoSansCJKsc-Regular.otf`. On desktop, the build copies resources beside the executable and the application resolves `resources/fonts/NotoSansCJKsc-Regular.otf` through `SDL_GetBasePath`, so launching from a different working directory still finds the font. An explicit desktop `WODEX_FONT` setting takes precedence.

The platform retains system-font fallbacks for development builds whose bundled resources are absent. Normal Switch builds reject missing font or license files, including when Make is invoked directly in the SDK container. The Makefile tracks nested RomFS files and directory changes so replacing a font, adding an asset or removing a resource repackages the NRO.

## Reproduce the Switch build

The verified toolchain is Rust `nightly-2026-10-01` and the official `devkitpro/devkita64:20260219` image, containing devkitA64 GCC 15.2.0 and libnx 4.12.0. The CA bundle, fonts, key-free HOME Menu template and their licenses are vendored in this repository. The vendored C++ JSON header is used only by desktop tests.

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
bash scripts/build-core-switch.sh
```

Both build scripts remap the developer's home, Cargo, toolchain and project paths to neutral paths in Rust output. The native compilation also remaps home and project paths. Rust flags use Cargo's encoded form so paths containing spaces remain intact.

Then compile and link the C++ shell with the official SDK container:

```sh
docker pull docker.io/devkitpro/devkita64:20260219
docker run --rm --network=none \
  --volume "$PWD:/workspace" \
  --workdir /workspace \
  docker.io/devkitpro/devkita64:20260219 \
  make -f scripts/Makefile.switch -j4
```

Podman's Docker-compatible command also works. The native link consumes the already-built Rust archive; it does not require Rust inside the container. Repeat both stages after changing Rust code, and repeat the second stage after changing native code. An installed compiler with the identical nightly manifest can be selected through `WODEX_RUST_TOOLCHAIN` with either build script.

Outputs are `build/switch/wodex.nro`, `wodex.elf`, `wodex.nacp`, `wodex.map` and `wodex.lst`. Copy the NRO to `sdmc:/switch/wodex/wodex.nro`. The application stores its own data under `sdmc:/switch/wodex/`.

## Verification boundary

The Rust Switch-target archive and complete native NRO have been built with the toolchain above. Native integration tests use a local mock API to exercise the transport and persistence without a paid API key. [GitHub Actions](https://github.com/old-new-things/wodex/actions/workflows/build.yml) repeats Rust checks, the desktop build, the Switch NRO build and packaged updater tests. Publication tests cover artifact integrity, immutable versions, retention bounds, snapshot extraction and deployment-state recovery before eligible builds reach Pages.

Execution on physical Switch hardware remains necessary to verify graphics performance, bundled-font rendering, the software keyboard and networking under the device's firmware and homebrew launch mode. The generated NRO is a build-verified artifact, not evidence of a completed hardware test.
