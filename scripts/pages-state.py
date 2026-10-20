#!/usr/bin/env python3
"""Restore only previously deployed Pages snapshots; never publish GitHub user data."""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import zipfile

STATE_NAME = re.compile(r"wodex-pages-state-([1-9][0-9]*)-([1-9][0-9]*)")
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024
MAX_FILE_BYTES = 128 * 1024 * 1024
MAX_ENTRIES = 2000


def api(repository, endpoint, method="GET"):
  output = subprocess.check_output([
    "gh", "api", "--method", method, f"repos/{repository}/{endpoint}",
  ])
  return json.loads(output) if output else None


def artifacts(repository):
  snapshots = []
  for page in range(1, 21):
    result = api(repository, f"actions/artifacts?per_page=100&page={page}")
    entries = result["artifacts"]
    snapshots.extend(item for item in entries if STATE_NAME.fullmatch(item["name"]))
    if len(entries) < 100:
      return sorted(snapshots, key=lambda item: item["id"], reverse=True)
  raise RuntimeError("Artifact history exceeds the bounded search; preserve the current site.")


def trusted_run(run, repository, workflow_id):
  ref = run.get("head_branch", "")
  return (
    run.get("workflow_id") == workflow_id
    and (run.get("head_repository") or {}).get("full_name") == repository
    and run.get("event") in {"push", "schedule", "workflow_dispatch"}
    and (ref == "main" or (ref.startswith("v") and VERSION.fullmatch(ref[1:])))
  )


def deployment_succeeded(jobs):
  return any(
    job.get("name") == "Publish build channels"
    and any(
      step.get("name") == "Deploy download page" and step.get("conclusion") == "success"
      for step in job.get("steps", [])
    )
    for job in jobs
  )


def select_snapshot(repository, candidates, workflow_id, run_id, attempt):
  for item in candidates:
    match = STATE_NAME.fullmatch(item["name"])
    source_run, source_attempt = map(int, match.groups())
    if (source_run, source_attempt) == (run_id, attempt):
      continue
    if item.get("workflow_run", {}).get("id") != source_run:
      continue
    source = api(repository, f"actions/runs/{source_run}/attempts/{source_attempt}")
    if not trusted_run(source, repository, workflow_id):
      continue
    endpoint = f"actions/runs/{source_run}/attempts/{source_attempt}/jobs?per_page=100"
    for poll in range(7):
      jobs = api(repository, endpoint)["jobs"]
      if deployment_succeeded(jobs):
        if item.get("expired"):
          raise RuntimeError("The latest deployed snapshot expired; restore it before publishing.")
        return item
      pending = any(
        job.get("name") == "Publish build channels" and job.get("status") != "completed"
        for job in jobs
      )
      if not pending:
        break
      if poll == 6:
        raise RuntimeError("The preceding deployment is unresolved; preserve the current site.")
      time.sleep(5)
  return None


def extract_snapshot(archive_path, destination):
  if destination.exists():
    raise RuntimeError("Previous snapshot destination already exists.")
  with zipfile.ZipFile(archive_path) as archive:
    entries = archive.infolist()
    if len(entries) > MAX_ENTRIES or sum(item.file_size for item in entries) > MAX_SNAPSHOT_BYTES:
      raise RuntimeError("The snapshot exceeds the extraction bounds.")
    names = set()
    for item in entries:
      path = PurePosixPath(item.filename)
      mode = item.external_attr >> 16
      if (
        not item.filename or "\\" in item.filename or "\x00" in item.filename
        or path.is_absolute() or ".." in path.parts
        or path.as_posix() != item.filename.rstrip("/")
        or item.filename in names or stat.S_ISLNK(mode)
        or (stat.S_IFMT(mode) not in {0, stat.S_IFREG, stat.S_IFDIR})
        or item.file_size > MAX_FILE_BYTES
      ):
        raise RuntimeError("The snapshot contains an unsafe archive entry.")
      names.add(item.filename)
    if "catalog.json" not in names:
      raise RuntimeError("The snapshot has no build catalog.")
    destination.mkdir(parents=True)
    archive.extractall(destination)


def assert_empty_site(site_url):
  request = urllib.request.Request(
    f"{site_url}/catalog.json?publication_check={time.time_ns()}",
    headers={"Cache-Control": "no-cache"},
  )
  try:
    with urllib.request.urlopen(request, timeout=30):
      raise RuntimeError("A live catalog exists without a trusted snapshot; restore state first.")
  except urllib.error.HTTPError as error:
    if error.code != 404:
      raise RuntimeError("Cannot verify previous site state; preserve the current site.") from error


def prepare(args):
  run = api(args.repository, f"actions/runs/{args.run_id}/attempts/{args.attempt}")
  workflow_id = api(args.repository, "actions/workflows/build.yml")["id"]
  if not trusted_run(run, args.repository, workflow_id) or run["head_sha"] != args.commit:
    raise RuntimeError("This run is not an authorized build-channel producer.")
  head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
  if head != args.commit:
    raise RuntimeError("The checked-out source does not match the built commit.")
  version_text = (args.artifact_dir / "VERSION").read_text()
  version = version_text.removesuffix("\n")
  if len(version) > 15 or not VERSION.fullmatch(version):
    raise RuntimeError("The artifact VERSION is invalid.")
  if args.ref == "refs/heads/main":
    channel, tag = "nightly", ""
  elif args.ref == f"refs/tags/v{version}":
    subprocess.run(["git", "merge-base", "--is-ancestor", args.commit, "origin/main"], check=True)
    channel, tag = "version", f"v{version}"
  else:
    raise RuntimeError("Only main and exact vX.Y.Z tags may publish builds.")
  timestamp = datetime.fromisoformat(run["run_started_at"].replace("Z", "+00:00"))
  timestamp = timestamp.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
  snapshot = select_snapshot(
    args.repository, artifacts(args.repository), workflow_id, args.run_id, args.attempt,
  )
  if snapshot:
    with tempfile.TemporaryDirectory(prefix="wodex-pages-") as temporary:
      archive = Path(temporary) / "state.zip"
      with archive.open("wb") as output:
        subprocess.run([
          "gh", "api", f"repos/{args.repository}/actions/artifacts/{snapshot['id']}/zip",
        ], stdout=output, check=True)
      extract_snapshot(archive, args.previous_dir)
  else:
    assert_empty_site(args.site_url)
  with args.github_output.open("a") as output:
    for key, value in {
      "channel": channel,
      "tag": tag,
      "timestamp": timestamp,
      "has_previous": "true" if snapshot else "false",
      "previous_artifact_id": str(snapshot["id"]) if snapshot else "",
    }.items():
      output.write(f"{key}={value}\n")
  print(f"Prepared {channel} publication for {version}; previous snapshot: {bool(snapshot)}.")


def prune(args):
  keep = {args.current_id}
  if args.previous_id:
    keep.add(args.previous_id)
  for item in artifacts(args.repository):
    if item["id"] < args.current_id and item["id"] not in keep:
      api(args.repository, f"actions/artifacts/{item['id']}", method="DELETE")
  print("Retained the current and preceding publication snapshots.")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--repository", required=True)
  commands = parser.add_subparsers(dest="command", required=True)
  publish = commands.add_parser("prepare")
  publish.add_argument("--run-id", required=True, type=int)
  publish.add_argument("--attempt", required=True, type=int)
  publish.add_argument("--commit", required=True)
  publish.add_argument("--ref", required=True)
  publish.add_argument("--site-url", required=True)
  publish.add_argument("--artifact-dir", required=True, type=Path)
  publish.add_argument("--previous-dir", required=True, type=Path)
  publish.add_argument("--github-output", required=True, type=Path)
  cleanup = commands.add_parser("prune")
  cleanup.add_argument("--current-id", required=True, type=int)
  cleanup.add_argument("--previous-id", type=int)
  args = parser.parse_args()
  if not re.fullmatch(r"[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+", args.repository):
    parser.error("Invalid repository.")
  if args.command == "prepare":
    owner, repository = args.repository.split("/")
    if args.site_url != f"https://{owner.lower()}.github.io/{repository}":
      parser.error("The site URL must match the repository's project Pages URL.")
    if not re.fullmatch(r"[0-9a-f]{40}", args.commit):
      parser.error("Invalid source commit.")
    prepare(args)
  else:
    prune(args)


if __name__ == "__main__":
  main()
