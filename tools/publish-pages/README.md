# Pages snapshot publisher

This host-only Rust tool uses `std`; the Switch application remains `no_std`.
It publishes verified, already-tested artifacts into a static GitHub Pages
snapshot. It never calls the GitHub Releases API.

```sh
cargo run --locked -p wodex-publish-pages -- \
  --artifact-dir /artifact-directory \
  --output-dir /new-site-directory \
  --channel nightly \
  --timestamp 2026-10-20T12:34:56Z \
  --commit 0123456789abcdef0123456789abcdef01234567 \
  --base-url https://old-new-things.github.io/wodex
```

Pass `--previous-dir /previous-site-directory` when restoring the most recent
successful publication snapshot. The output directory must not exist, and its
parent must exist. Paths may be relative or absolute and must not use symbolic
links or `..`. For explicit versions use `--channel version --tag vX.Y.Z`;
the tag must match the artifact's `VERSION`. Timestamps use the source workflow
attempt's UTC start time; commit IDs use the full source commit hash.

Required input files:

- `wodex.nro`, `wodex.nro.sha256`, and `VERSION`
- `LICENSE` and `THIRD_PARTY.md`
- `licenses.tar.gz` and `licenses.tar.gz.sha256`
- `NotoSansCJK-OFL.txt` and `NotoSansCJK-README.md`

Only these files are copied, with sizes and SHA-256 digests verified again in the
finished snapshot. The executable must contain a bounded NRO header; the license
archive must have a gzip signature. The workflow is responsible for validating
the full NRO and license contents before handing artifacts to this tool.

`catalog.json` uses `schema_version: 1` with a `builds` array. Each build contains
`channel`, `id`, `version`, `timestamp`, `commit`, `size`, `sha256`,
`licenses_sha256`, and an `assets` map containing each file's `size` and `sha256`.
Paths are derived from validated identities; catalog-provided paths are never
accepted. `version_locks` preserves all explicit version asset digests, including
versions whose downloadable files have expired, to prevent later replacement.

Nightly IDs are `YYYYMMDDTHHMMSSZ-<first-seven-commit-characters>`. Downloads live
under `builds/nightly/<id>/` and `builds/version/<X.Y.Z>/`. The publisher retains
the seven newest nightlies and ten highest explicit versions. If necessary, it
prunes additional older entries to keep total download payloads within 450 MiB,
preserving the newest build in each channel. Integrity failure in a restored
snapshot aborts publication without modifying that snapshot.

`updates/stable.json` contains `schema_version: 1`, `channel: "version"`, and
`build: null` until a version is published. Otherwise `build` contains `version`,
absolute HTTPS `url`, `size`, `sha256`, and plain-text `notes` for the highest
semantic version. Nightlies never enter the stable update feed.

The static HTML and CSS contain no scripts, external requests, analytics,
publishing actor, environment details, or local filesystem paths.
