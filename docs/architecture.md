# Architecture and Switch builds

Wodex is a native Nintendo Switch homebrew client with a desktop development build. It sends requests directly to the configured DeepSeek-compatible API. It has no application backend or hosted frontend.

## Responsibilities

- `core/` is the Rust conversation and protocol engine. It validates settings, constructs request bodies, incrementally decodes Server-Sent Events, tracks cancellation and retry, and serializes versioned local history. The Switch build uses `no_std` plus `alloc`; host tests use the same logic with Rust's standard library.
- `native/src/main.cpp` owns the application loop, focus navigation, conversation views and input. `ui.cpp` draws the controller-oriented interface with SDL2 and SDL2_ttf at a logical 1280×720 resolution.
- `native/src/platform.cpp` owns Switch services, shared fonts, the system keyboard and local storage. The desktop implementation provides the same interface for development and testing.
- `native/src/transport.cpp` runs libcurl on a worker thread and passes bounded data chunks to the main thread. Only the main thread calls the Rust engine. HTTPS verifies certificates; the Switch package embeds a public CA bundle in RomFS.
- `native/include/wodex_core.h` is the C ABI boundary. Rust owns each returned string until the native caller releases it with `wodex_string_free`.

Rust's [Nintendo Switch target](https://doc.rust-lang.org/rustc/platform-support/aarch64-nintendo-switch-freestanding.html) supports `no_std` and has Tier 3 status. Building `core` and `alloc` with a pinned nightly compiler avoids depending on an unofficial Rust standard-library port. libnx and devkitPro own startup, service access, the final ELF link and NRO packaging. This keeps the Rust core portable while using the established native Switch SDK.

Wiliwili is the interaction reference: a controller-first application with readable text and direct platform integration. Wodex uses SDL2 rather than Wiliwili's Borealis UI library, so its small custom renderer can be built and exercised on desktop with the same native sources. See the official [libnx application template](https://github.com/switchbrew/switch-examples/tree/master/templates/application) and [SDL2 example](https://github.com/switchbrew/switch-examples/tree/master/graphics/sdl2/sdl2-simple) for the underlying homebrew build conventions.

## Reproduce the Switch build

The verified toolchain is Rust `nightly-2026-10-01` and the official `devkitpro/devkita64:20260219` image, containing devkitA64 GCC 15.2.0 and libnx 4.12.0. The JSON header and CA bundle are vendored in this repository.

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

Execution on physical Switch hardware remains necessary to verify graphics performance, shared-font coverage, the software keyboard and networking under the device's firmware and homebrew launch mode. The generated NRO is a build-verified artifact, not evidence of a completed hardware test.
