# Updates and build channels

Wodex publishes downloads through [GitHub Pages](https://old-new-things.github.io/wodex/). There are two channels: timestamped **Nightly** builds and explicitly tagged **Versions**. GitHub Releases are not created. Downloads and update checks need no GitHub token or account login.

New installations use `old-new-things/wodex` and the **Versions** channel. Existing saved sources are preserved, including an empty source that disables checks. Choose **Use Wodex** on the Updates page to select the official source, or **Change** to enter another project's `owner/repository`. Choose **Nightly** to opt into experimental builds. The source and channel are saved; changing either invalidates a previously checked or downloaded update until another check succeeds.

## Install an update

1. Open Settings → Updates, select **Versions** or **Nightly**, and choose **Check for updates**.
2. Review the selected build, then choose **Download**.
3. Wait for verification to complete. Choose **Install and exit** to replace the running NRO.
4. Launch Wodex again from hbmenu or its installed HOME Menu tile. Installation exits the app; it does not restart it automatically. The tile forwards to the NRO and does not need reinstalling after an update.

Use hbmenu through a game with **R** held so Wodex has full application memory. Album/applet mode is unsupported; current nightlies reject it before graphics and font initialization. Older builds may still enter it without a warning. A freeze in Album mode is a reason to repeat the same operation in full application mode before attributing it to a particular update.

Interrupted and cancelled downloads retain one fixed file beside the installed NRO: `wodex.nro.update.part`. Choose **Resume download** to retry the selected build. After closing and reopening Wodex, check the channel again and choose **Download**; it automatically checks and resumes the retained partial. A completed, valid cached download can become ready again without downloading its bytes. Checking updates clears install eligibility while preserving that file.

There are no per-build temporary files or metadata sidecars. Wodex rehashes the retained prefix and requests the remaining bytes with HTTP Range. It appends only an exact, identity-encoded `206 Partial Content` response for the selected file. If the server ignores Range or rejects the offset, Wodex restarts in the same slot. A stale or damaged prefix gets at most one fresh retry after the final digest fails. Cancellation and transient failures preserve useful bytes; an invalid completed download is discarded. No incomplete file is installable.

The completed file is flushed, reopened, and checked against the selected feed's full size and SHA-256 and its NRO/ASET/RomFS structure. These checks run on the worker with cancellation checkpoints. Installation rechecks it again before replacement. `.previous.nro` is the installed-version backup, separate from the single download slot. To discard a retained download manually, close Wodex and remove only `.update.part`.

Wodex 0.2.1 adds the optional Nightly channel. Current nightlies contain the timestamp-based selection and return-to-Versions behavior described below. The fixed 0.2.1 download retains its original comparison rules; select Nightly to receive the new behavior. Wodex 0.2.0 can update to 0.2.1 through Versions first.

If you installed the early nightly from commit `52dd555`, which displayed the provisional version 0.3.0, install a current nightly manually once: close Wodex, copy its `wodex.nro` from the [Nightly downloads](https://old-new-things.github.io/wodex/#nightly) over your existing NRO, and reopen it. Preserve the application data directory. That older executable rejects lower version numbers; it needs this manual replacement to receive the new comparison code. Afterward, choosing Versions can install the latest published version even if its version number is lower.

The app reads `https://old-new-things.github.io/wodex/updates/stable.json` when configured for this repository. The feed uses `schema_version: 1`, `channel: "version"` and a required `build` field. Before the first explicit version is published, `build` is `null` and no build is offered. Otherwise it contains:

```json
{
  "schema_version": 1,
  "channel": "version",
  "build": {
    "version": "0.2.0",
    "url": "https://old-new-things.github.io/wodex/builds/version/0.2.0/wodex.nro",
    "size": 26000000,
    "sha256": "<64 hexadecimal characters>",
    "notes": "Wodex 0.2.0."
  }
}
```

The size and digest above are illustrative. Publication generates the real values from the verified NRO.

The Versions feed selects the highest published canonical `X.Y.Z` version. From an installed nightly, switching to Versions offers that build regardless of whether its version number is lower, equal, or higher. From an installed Versions build, only a newer version number is offered. This prevents repeated installation after returning to Versions. The stable feed retains its original schema so Wodex 0.2.0 can update to a version with channel selection.

Nightly reads `https://old-new-things.github.io/wodex/updates/nightly.json`. Its schema uses `schema_version: 1`, `channel: "nightly"` and a required `build` field, which is `null` before a nightly exists. A populated build has the same fields as the version feed, plus:

```json
{
  "id": "20261020T001700Z-0123456",
  "timestamp": "2026-10-20T00:17:00Z",
  "commit": "0123456789abcdef0123456789abcdef01234567"
}
```

The nightly URL must be `https://old-new-things.github.io/wodex/builds/nightly/<id>/wodex.nro`. The ID must match the valid UTC timestamp and the first seven characters of the full lowercase commit SHA. Published NROs embed their own channel, timestamp and commit. The app selects a later nightly by timestamp, independently of semantic version: a newer nightly with a lower version number is eligible, and an older nightly with a higher version number is not. Nightly IDs break equal-timestamp ties for installed nightlies in the same order as the publisher. Switching from Versions requires a strictly newer nightly timestamp. Local development binaries without a publication identity may opt into the latest nightly. Checking again after installation does not offer the same build.

Both feeds require a size from the minimum NRO header through 128 MiB and a valid SHA-256 digest. The URL must match the configured project's exact HTTPS Pages path for the selected channel and build. Unknown schema fields, mismatched channels, malformed metadata and redirects are rejected. Downloads must match both the advertised byte count and SHA-256 and pass NRO/ASET/RomFS validation before becoming installable. The adjacent `.sha256` file supports manual verification; the app uses the digest in the selected feed.

Checking and downloading do not replace the application. Installing targets the actual SD-card NRO path supplied by the homebrew loader in `argv[0]`. A missing or unsupported executable path disables installation and leaves manual copying available. Desktop builds can check both feeds; self-installation is supported on Switch.

## Installation and recovery

The app saves conversations, finishes its UI and network shutdown, and unmounts RomFS before replacing the NRO. It retains the previous executable beside the destination as a `.previous.nro` backup. Conversations and API settings remain in the application's data directory. A normal exit never installs a staged update.

SD-card FAT filesystems cannot atomically replace an existing destination with the same guarantees as a desktop filesystem. Installation therefore has a short rename gap between moving the old executable aside and putting the verified download in its place. Keep power connected during installation. If power is lost in that gap, use a computer or SD-card file manager to rename the previous NRO back to the original application filename, or manually copy a verified `wodex.nro` there. Then launch it from hbmenu. The app cannot repair its own missing launch file before it is launched.

If downloading, digest validation or staging fails, the installed application is left in place. Installation rechecks the staged file before replacement, and a failed install attempts to restore the previous executable. Retain the backup until the new version has been launched successfully. If the update source is unavailable or not configured, the existing app remains usable and installation by copying the NRO continues to work.

TLS and SHA-256 protect transfer integrity relative to the selected feed. The repository's publisher remains trusted to provide executable updates.

## Publish nightly and version builds

The [build workflow](https://github.com/old-new-things/wodex/actions/workflows/build.yml) runs Rust checks, all desktop integration suites, the Switch build and updater tests using the packaged NRO. It then dispatches the [publication workflow](https://github.com/old-new-things/wodex/actions/workflows/publish.yml) on `main`, passing the successful source run and attempt. Publication validates that source before downloading its artifact. The Switch build uses `devkitpro/devkita64:20260219` and Rust `nightly-2026-10-01`.

Successful `main` pushes publish nightlies. A daily run at **00:17 UTC** and manual runs on `main` use the same checks and channel. Nightly paths contain the UTC workflow-attempt timestamp and abbreviated source commit, for example `builds/nightly/20261020T001700Z-0123456/`. Other branches and pull requests upload CI artifacts without changing the download site.

The build identity job obtains the source workflow attempt's start time and commit from GitHub. The Switch compiler embeds them through `WODEX_BUILD_CHANNEL`, `WODEX_BUILD_TIMESTAMP`, and `WODEX_BUILD_COMMIT`. A CI-only `build-info.json` accompanies the artifact; publication requires it to match the independently validated source run and `VERSION`. All three environment variables must be supplied together for a local published-identity build; leaving them unset produces a development build.

`VERSION` is the canonical application version, used by both the desktop version string and Switch NACP metadata. It must contain one `X.Y.Z` version of at most 15 ASCII characters, without leading zeroes in a component. An explicit version is published by pushing a tag containing `v` followed by that exact value. The tagged commit must already be reachable from `main`.

Normal development keeps the version number unchanged and publishes timestamped nightlies. Change the version number when preparing an intentional fixed-version publication. Current development uses the 0.2.1 base version.

After committing and pushing the version's source to `main`, create and push its tag:

```sh
git tag "v$(cat VERSION)"
git push origin "v$(cat VERSION)"
```

For example, `v0.2.0` publishes to `builds/version/0.2.0/`. The version feed selects the highest published semantic version. Publishing an older tag does not downgrade that selection. An existing version cannot be assigned different download bytes, even after its downloadable files have been pruned; retained digest records enforce this. Increment `VERSION` before publishing changed files.

Each build includes `wodex.nro`, its checksum, `VERSION`, the application license, third-party notices, a checksummed `licenses.tar.gz`, and Noto font license/attribution. New builds use WTFPL version 2 for Wodex code; dependencies retain their original licenses, and already published downloads keep their accompanying license files. The font and its notices also remain embedded in the NRO. Generated publication metadata omits developer identities, personal paths and GitHub actor fields; required upstream license credits remain. Public GitHub activity, including workflow runs and pushes, can still identify accounts; this publication method does not provide account anonymity.

## Publication state and recovery

Publication is serialized so each job restores the last accepted Pages deployment snapshot before adding its build. The host Rust publisher verifies the restored catalog and every retained file, generates the complete replacement site, and uploads it for deployment. The publisher is a separate `std` tool; the Switch application remains `no_std` plus `alloc`.

The site retains up to seven newest nightlies by timestamp and ten highest versions. Each publication removes older nightly entries and their download files from the new site snapshot. A 450 MiB download budget may remove additional older builds while preserving the newest build in each channel. Version digest records survive pruning.

Recovery snapshots are stored as attempt-specific Actions artifacts. After deployment, the publication workflow checks the live catalog, both update feeds, page, and newest downloads against its generated files. It retries for up to 11 minutes to allow CDN propagation. Only successful public verification permits cleanup, which keeps the current and preceding snapshots with 90-day retention. Successful daily publications refresh this history.

Restoration accepts only snapshots from this repository's authorized publication workflow with a successful Pages deployment step, plus a fixed allowlist of the initial migration snapshots. If later public verification fails, the accepted snapshot is still retained as publication intent: the next run preserves its version locks and downloads instead of dropping a potentially deployed version. A failed verification marks the workflow unsuccessful and leaves recovery artifacts intact.

Restored snapshots keep their existing downloads, including older versions without embedded build identity. New incoming artifacts require `build-info.json`; replaying a pre-identity build is rejected. Rerun all build jobs when regenerating an artifact so its embedded identity and workflow attempt agree. Already published explicit versions remain immutable.

The publication workflow always runs through `workflow_dispatch` on `main`, including for tag builds. This avoids a [reported Pages issue with repeated commit IDs on tag/release deployments](https://github.com/actions/deploy-pages/issues/383). The source commit and channel in the catalog still describe the actual tested build. A green build workflow means its tests and dispatch succeeded; check the publication workflow for the public download result.

If the latest deployed snapshot has expired, restoration fails. If no trusted snapshot exists but a live catalog does, publication also fails instead of resetting the site and losing its history or version locks. Recover the complete snapshot from a trusted backup and reconcile it with the live catalog before resuming publication. Do not remove the live catalog to bypass this guard. Existing public downloads remain available while publication is stopped.

Daily jobs depend on GitHub's scheduler. Public-repository schedules can be disabled after 60 days without repository activity, so long inactivity can eventually require publication-state recovery. See [GitHub's schedule behavior](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule).
