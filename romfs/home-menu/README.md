# HOME Menu template

`template.bin` is a key-free build input for Wodex's on-console installer. It is not an installable NSP. Its manifest records source revisions, generation scripts, SHA-256 checksums, content sizes, title identity, and target path.

The title ID is `05776F6465780000`. The NACP display version is `Forwarder 1`, independent of Wodex's app version and nightly timestamp. The forwarder loads `sdmc:/switch/wodex/wodex.nro`, so app updates do not require replacing the tile. Its exit helper performs the application-manager handshake if an older NRO returns to the loader, including when switching from Nightly to an existing Versions build. The same helper handles a missing target.

The original `nx-hbloader` source is pinned and vendored under [`external/nx-hbloader`](../../external/nx-hbloader/README.wodex.md). Preserve `LICENSE.nx-hbloader.txt` when distributing the template.

## Offline verification

```sh
python3 scripts/build-home-menu.py --check
```

This needs only Python's standard library. It verifies source and script hashes, all NCA filesystem headers, the program/meta PFS0 hash hierarchies, compressed NSO segment hashes, the launch path, NPDM identities, control metadata, the icon, and license notices. It rejects encrypted sections, signatures, rights IDs, and nonzero key areas.

## Rebuilding

Fetch the build-only [hacBrewPack source](https://github.com/dragonflylee/hacBrewPack/tree/745b16ecfc9ce055743067d200572204cb2aac6c) explicitly:

```sh
python3 scripts/build-home-menu.py --fetch-tools
```

Then generate entirely offline using the pinned Switch builder:

```sh
docker run --rm --network=none \
  -v "$PWD:/workspace" -w /workspace \
  docker.io/devkitpro/devkita64:20260219 \
  bash -c 'git config --global --add safe.directory /workspace/build/home-menu-deps/hacBrewPack && python3 scripts/build-home-menu.py'
```

Replace the final Python command with `python3 scripts/build-home-menu.py --verify-rebuild` to rebuild into a temporary directory and require byte-for-byte identical resources. A changed generator requires regenerating the manifest even if the resulting executable is unchanged.

hacBrewPack and its bundled mbedTLS are external build tools, downloaded into ignored `build/`. Their sources and license texts remain there. They are not linked into Wodex or the forwarder. No installed `prod.keys`, console key file, or environment secret is read. The generator supplies fixed public test vectors, asks the tool for plaintext sections without randomized signing, removes its test encryption from the headers, and discards the intermediate synthetic NSP.

## Container format

All integer fields use little-endian encoding.

| Offset | Length | Contents |
| --- | --- | --- |
| 0 | 8 | ASCII `WDXHOME1` |
| 8 | 4 | Program NCA length |
| 12 | 4 | Control NCA length |
| 16 | 4 | Meta NCA length |
| 20 | Variable | Program, control, then meta NCA bytes |

Each NCA starts with a clear `0xC00`-byte header and uses plaintext sections. It has no rights ID, encrypted key area, or signature. The program contains the executable and NPDM; control contains NACP and the icon; meta contains an application CNMT with version zero, a 16-byte extended header, and two content records. The CNMT content hashes refer to generation inputs until the installer replaces them.

At installation, Wodex seals the program and control headers locally, updates their CNMT hashes, updates the dependent meta hash hierarchy, then seals meta. It registers the resulting three content files and their metadata through libnx. Console-derived header material never becomes a build input or public artifact.

Format references: [NCA](https://switchbrew.org/wiki/NCA), [CNMT](https://switchbrew.org/wiki/CNMT), [NSO](https://switchbrew.org/wiki/NSO), and [NPDM](https://switchbrew.org/wiki/NPDM). Wodex's build script and format validation are independent implementations; no wiliwili, Leaf Installer, or NSteamLink implementation is incorporated.
