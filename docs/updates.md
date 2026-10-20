# Updates

Wodex supports manually requested updates from a public GitHub repository. No release repository is configured by default. Enter `old-new-things/wodex` in Settings to use this project's [published releases](https://github.com/old-new-things/wodex/releases). The first update check requires a published stable release; CI artifacts can be installed manually. Wodex does not need a GitHub token or an account login.

## Install an update

1. Open Settings → Updates and choose **Check for updates**.
2. Review the newer stable version, then choose **Download**.
3. Wait for the download and verification to complete. Choose **Install and exit** to replace the running NRO.
4. Launch Wodex again from hbmenu. Installation exits the app; it does not restart it automatically.

Checks use GitHub's latest published stable release. Wodex requires a regular `X.Y.Z` version, optionally prefixed with `v`, that is newer than the installed version, one uploaded asset named exactly `wodex.nro`, an advertised size no larger than 128 MiB, and GitHub's `sha256:` asset digest. Drafts, prereleases, unsupported tags, missing digests and malformed release metadata are rejected. The download must match both the advertised byte count and SHA-256 before it becomes installable. The attached `.sha256` text file supports manual verification; the app uses the digest in the GitHub API response.

Checking and downloading do not replace the application. Installing targets the actual SD-card NRO path supplied by the homebrew loader in `argv[0]`, rather than assuming a fixed installation folder. A missing or unsupported executable path disables installation and leaves manual copying available.

## Installation and recovery

The app finishes its UI and network shutdown and unmounts RomFS before replacing the NRO. It retains the previous executable beside the destination as a `.previous.nro` backup. Conversations and API settings remain in the application's data directory.

SD-card FAT filesystems cannot atomically replace an existing destination with the same guarantees as a desktop filesystem. Installation therefore has a short rename gap between moving the old executable aside and putting the verified download in its place. Keep power connected during installation. If power is lost in that gap, use a computer or SD-card file manager to rename the previous NRO back to the original application filename, or manually copy a verified `wodex.nro` there. Then launch it from hbmenu. The app cannot repair its own missing launch file before it is launched.

If downloading, digest validation or staging fails, the installed application is left in place. A failed install attempts to restore the previous executable; retain the backup until the new version has been launched successfully. If an update source is unavailable or not configured, the existing app remains usable and installation by copying the NRO continues to work.

## Publish compatible releases

`VERSION` is the canonical application version. Both the desktop version string and Switch NACP metadata are built from it. It must contain one stable `X.Y.Z` version of at most 15 ASCII characters, without leading zeroes in a component. Stable publishing tags use `v` followed by that exact version, such as `v0.2.0` for a `VERSION` file containing `0.2.0`.

The [stable-release workflow](https://github.com/old-new-things/wodex/actions/workflows/release.yml) runs when a `v*` tag is pushed or when **Publish stable Switch release** is manually dispatched with an existing tag. Ordinary branch pushes run CI and upload build artifacts. To publish the version recorded in `VERSION`, commit its changes to `main`, push `main`, then create and push the matching tag:

```sh
git tag "v$(cat VERSION)"
git push origin "v$(cat VERSION)"
```

The workflow validates the strict stable tag against the tagged `VERSION`, runs the native and Rust checks, builds with `devkitpro/devkita64:20260219` and Rust `nightly-2026-10-01`, and tests update installation with the packaged NRO. It uploads `wodex.nro`, `wodex.nro.sha256`, the application license, third-party notices, a checksummed `licenses.tar.gz` bundle, and Noto font license/attribution. The font and its notices also remain embedded in the NRO.

Merge workflow changes into the repository's default branch before tagging a release that contains them. GitHub's default Actions token cannot publish a commit whose workflow files differ from the default branch without additional workflow permissions.

Publishing is a separate job with write permission. It creates a draft for an already-existing tag, verifies GitHub's uploaded asset size and SHA-256 against the local artifact, then publishes it as a regular stable release. GitHub determines the latest release using its stable-version policy. An upload or verification failure leaves a draft instead of exposing an incomplete update. Inspect and remove a failed draft before rerunning; the workflow does not overwrite an existing release or create a missing tag.

The release metadata and digest fields follow GitHub's [release API](https://docs.github.com/en/rest/releases/releases) and [release asset API](https://docs.github.com/en/rest/releases/assets). TLS and the digest protect transfer integrity against the selected release metadata; the repository owner remains the trusted publisher of executable updates.
