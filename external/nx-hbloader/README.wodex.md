# nx-hbloader source provenance

The files `source/main.c`, `source/trampoline.s`, `hbl.json`, and `LICENSE.md` are unmodified from [switchbrew/nx-hbloader commit 82b95122c5ae8dc059bf23893ba7623c72c86773](https://github.com/switchbrew/nx-hbloader/tree/82b95122c5ae8dc059bf23893ba7623c72c86773). Their upstream ISC license and copyright remain intact.

[`scripts/build-home-menu.py`](../../scripts/build-home-menu.py) verifies their SHA-256 checksums before applying Wodex's changes to temporary build copies:

- Launch `sdmc:/switch/wodex/wodex.nro`.
- Use the application-manager exit handshake when the NRO returns without selecting another NRO, or when its target is missing. This also supports older Wodex versions whose NROs return directly to the loader. The exit helper opens the normal application proxy directly to avoid waiting for foreground focus while HOME is closing the app.
- Use Wodex's fixed application ID and application process permissions.
- Build without debug paths or a generated ELF build ID.

The resulting forwarder uses libnx from the same pinned devkitPro image as Wodex. Its ISC notice is embedded in Wodex's RomFS and included in `licenses.tar.gz`. The upstream loader remains a native platform component; it does not replace Wodex's Rust application or its `no_std` runtime.
