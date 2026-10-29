# HOME Menu tile

Current nightlies can install a Wodex tile on the Switch HOME Menu. The tile starts the NRO with application memory, without opening hbmenu.

1. Place Wodex at `sdmc:/switch/wodex/wodex.nro` and launch it.
2. Open **Settings → Install to dock**.
3. Read the installation information, then choose **Install to dock**.
4. Wait for installation to finish, exit Wodex, and return to HOME.

The Switch must already support launching unsigned homebrew titles through its custom firmware. Wodex does not install or modify custom firmware. Desktop builds show the instructions but cannot install a Switch title.

Keep the NRO at the path above. The tile contains a small forwarder, not a second copy of Wodex. Installing a newer NRO manually or using **Settings → Updates** updates what the tile launches; neither the Versions nor Nightly channel needs a separate tile. The base app version remains 0.2.1.

If Wodex was started from another directory, move it to the canonical path and relaunch before installing the tile. This avoids a shortcut launching an older copy while the updater replaces a different file.

The installer uses the fixed homebrew application ID `05776F6465780000`. It checks for an existing application record and content metadata before writing. It does not overwrite an existing title at that ID. Remove an existing tile through **System Settings → Data Management → Software** before installing it again. The title ID stays the same across app updates.

Installation runs in a worker while the interface displays its status. Wodex saves conversations first and blocks installation during a response or an app update. Once installation starts, wait for it to finish before exiting or powering off the console. Failed operations attempt to remove only the content and metadata created by that attempt.

## Build and implementation

The distributed NRO embeds a key-free forwarder template and its license. CI and source control contain no console key file. The Switch prepares the template's content headers locally when installing.

The forwarder is built from pinned ISC-licensed [nx-hbloader](https://github.com/switchbrew/nx-hbloader) sources. [hacBrewPack](https://github.com/dragonflylee/hacBrewPack) is an external build tool; its implementation is not linked into Wodex. The template generator, installer, validation, worker state, and UI are Wodex code. See [third-party notices](../THIRD_PARTY.md) for the upstream licenses.

`core/src/backend/home_menu.rs` and its `home_menu/` modules own format validation and installation policy using `core` and `alloc`. The native adapter translates the libnx service and cryptographic calls. The ordinary self-updater continues replacing the NRO at its loader-supplied path.

The committed template allows ordinary builds without regenerating the forwarder. Run `python3 scripts/build-home-menu.py --check` to verify it offline. See the generator's help for regeneration and reproducibility checks.

Host-side checks validate the template and exercise installation failures with fake platform services. Physical Switch installation, HOME display, launch, and exit still require hardware verification.
