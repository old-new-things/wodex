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
BUILD_JOBS = {
  "Rust and desktop tests",
  "Switch no_std build",
  "Packaged NRO update tests",
}
# One-time migration bridge for the two snapshots produced before publication
# moved to its own main-branch workflow. Future legacy runs must not restore
# stale state that omits snapshots created by the new publication workflow.
LEGACY_SNAPSHOTS = {(39396939856, 1), (39397383528, 1)}
LEGACY_COMMIT = "5847d6ac77b74090c3150a9bdbb1c096bfebcdb3"


def api(repository, endpoint, method="GET"):
  output = subprocess.check_output([
    "gh", "api", "--method", method, f"repos/{repository}/{endpoint}",
  ])
  return json.loads(output) if output else None


def artifact_created_at(item):
  value = item.get("created_at")
  try:
    if not isinstance(value, str) or not re.fullmatch(
      r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]+)?Z", value,
    ):
      raise ValueError("Missing UTC artifact timestamp.")
    created = datetime.fromisoformat(value.removesuffix("Z") + "+00:00")
  except ValueError as error:
    raise RuntimeError("Artifact creation timestamp is invalid; preserve publication state.") from error
  return created


def artifacts(repository):
  snapshots = []
  for page in range(1, 21):
    result = api(repository, f"actions/artifacts?per_page=100&page={page}")
    entries = result["artifacts"]
    snapshots.extend(item for item in entries if STATE_NAME.fullmatch(item["name"]))
    if len(entries) < 100:
      # Artifact IDs may come from different storage shards and are not a clock.
      return sorted(
        snapshots, key=lambda item: (artifact_created_at(item), item["id"]), reverse=True,
      )
  raise RuntimeError("Artifact history exceeds the bounded search; preserve the current site.")


def trusted_run(run, repository, workflow_id):
  ref = run.get("head_branch", "")
  return (
    run.get("workflow_id") == workflow_id
    and (run.get("head_repository") or {}).get("full_name") == repository
    and run.get("event") in {"push", "schedule", "workflow_dispatch"}
    and (ref == "main" or (
      ref.startswith("v") and len(ref) <= 16 and VERSION.fullmatch(ref[1:])
    ))
  )


def trusted_publisher(run, repository, workflow_id):
  return (
    run.get("workflow_id") == workflow_id
    and (run.get("head_repository") or {}).get("full_name") == repository
    and run.get("event") == "workflow_dispatch"
    and run.get("head_branch") == "main"
  )


def deployment_succeeded(jobs):
  # Once Pages accepts a deployment, preserve its version locks even if a later
  # delivery check fails. A subsequent publish repairs delivery without erasing
  # the accepted publication's history. Live verification still gates pruning.
  return any(
    job.get("name") == "Publish build channels"
    and any(
      step.get("name") == "Deploy download page" and step.get("conclusion") == "success"
      for step in job.get("steps", [])
    )
    for job in jobs
  )


def select_snapshot(repository, candidates, publish_workflow_id, build_workflow_id, run_id, attempt):
  for item in candidates:
    match = STATE_NAME.fullmatch(item["name"])
    source_run, source_attempt = map(int, match.groups())
    if (source_run, source_attempt) == (run_id, attempt):
      continue
    if item.get("workflow_run", {}).get("id") != source_run:
      continue
    source = api(repository, f"actions/runs/{source_run}/attempts/{source_attempt}")
    publisher = trusted_publisher(source, repository, publish_workflow_id)
    legacy = (
      (source_run, source_attempt) in LEGACY_SNAPSHOTS
      and source.get("head_sha") == LEGACY_COMMIT
      and trusted_run(source, repository, build_workflow_id)
    )
    if not publisher and not legacy:
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


def checked_source(repository, run_id, attempt):
  workflow_id = api(repository, "actions/workflows/build.yml")["id"]
  latest = api(repository, f"actions/runs/{run_id}")
  run = api(repository, f"actions/runs/{run_id}/attempts/{attempt}")
  if (
    not trusted_run(latest, repository, workflow_id)
    or not trusted_run(run, repository, workflow_id)
    or latest.get("run_attempt") != attempt
    or run.get("run_attempt") != attempt
    or latest.get("head_sha") != run.get("head_sha")
    or latest.get("head_branch") != run.get("head_branch")
    or not re.fullmatch(r"[0-9a-f]{40}", run.get("head_sha", ""))
  ):
    raise RuntimeError("The source must be the latest attempt of a trusted build workflow.")
  jobs = api(
    repository, f"actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100",
  )["jobs"]
  for name in BUILD_JOBS:
    matches = [job for job in jobs if job.get("name") == name]
    if len(matches) != 1 or any(
      job.get("status") != "completed" or job.get("conclusion") != "success"
      for job in matches
    ):
      raise RuntimeError("All source build and validation jobs must have completed successfully.")
  branch = run["head_branch"]
  channel, tag = ("nightly", "") if branch == "main" else ("version", branch)
  ref = "refs/heads/main" if channel == "nightly" else f"refs/tags/{tag}"
  try:
    raw_timestamp = run["run_started_at"]
    if not isinstance(raw_timestamp, str):
      raise ValueError("Missing timestamp.")
    started = datetime.fromisoformat(raw_timestamp.replace("Z", "+00:00"))
    if started.tzinfo is None:
      raise ValueError("Missing timezone.")
    timestamp = started.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
  except (KeyError, TypeError, ValueError) as error:
    raise RuntimeError("Source build timestamp is invalid.") from error
  return {
    "commit": run["head_sha"],
    "ref": ref,
    "channel": channel,
    "tag": tag,
    "timestamp": timestamp,
  }


def write_outputs(path, values):
  with path.open("a") as output:
    for key, value in values.items():
      if "\n" in str(value) or "\r" in str(value):
        raise RuntimeError("Workflow outputs must contain only a single line.")
      output.write(f"{key}={value}\n")


def source(args):
  result = checked_source(args.repository, args.run_id, args.attempt)
  write_outputs(args.github_output, result)
  print(f"Validated the {result['channel']} build source and all three prerequisite jobs.")


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
  publish_workflow_id = api(args.repository, "actions/workflows/publish.yml")["id"]
  build_workflow_id = api(args.repository, "actions/workflows/build.yml")["id"]
  if not trusted_publisher(run, args.repository, publish_workflow_id):
    raise RuntimeError("Publication requires the trusted publish workflow dispatched on main.")
  metadata = checked_source(args.repository, args.build_run_id, args.build_attempt)
  if metadata["commit"] != args.commit or metadata["ref"] != args.ref:
    raise RuntimeError("The requested source does not match its validated build workflow.")
  head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
  if head != run["head_sha"]:
    raise RuntimeError("The checked-out publisher does not match its main-branch workflow commit.")
  subprocess.run(["git", "merge-base", "--is-ancestor", args.commit, "origin/main"], check=True)
  version_text = (args.artifact_dir / "VERSION").read_text()
  version = version_text.removesuffix("\n")
  if len(version) > 15 or not VERSION.fullmatch(version):
    raise RuntimeError("The artifact VERSION is invalid.")
  if metadata["channel"] == "version" and metadata["tag"] != f"v{version}":
    raise RuntimeError("The explicit version tag must match the artifact VERSION.")
  snapshot = select_snapshot(
    args.repository, artifacts(args.repository), publish_workflow_id, build_workflow_id,
    args.run_id, args.attempt,
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
  write_outputs(args.github_output, {
    **metadata,
    "has_previous": "true" if snapshot else "false",
    "previous_artifact_id": str(snapshot["id"]) if snapshot else "",
  })
  print(f"Prepared {metadata['channel']} publication for {version}; previous snapshot: {bool(snapshot)}.")


def prune(args):
  snapshots = artifacts(args.repository)
  current = next((item for item in snapshots if item["id"] == args.current_id), None)
  if current is None:
    raise RuntimeError("Current publication snapshot was not found; preserve recovery history.")
  cutoff = artifact_created_at(current)
  keep = {args.current_id}
  if args.previous_id:
    keep.add(args.previous_id)
  for item in snapshots:
    if artifact_created_at(item) < cutoff and item["id"] not in keep:
      api(args.repository, f"actions/artifacts/{item['id']}", method="DELETE")
  print("Retained the current and preceding publication snapshots.")


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--repository", required=True)
  commands = parser.add_subparsers(dest="command", required=True)
  build_source = commands.add_parser("source")
  build_source.add_argument("--run-id", required=True, type=int)
  build_source.add_argument("--attempt", required=True, type=int)
  build_source.add_argument("--github-output", required=True, type=Path)
  publish = commands.add_parser("prepare")
  publish.add_argument("--run-id", required=True, type=int)
  publish.add_argument("--attempt", required=True, type=int)
  publish.add_argument("--build-run-id", required=True, type=int)
  publish.add_argument("--build-attempt", required=True, type=int)
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
  for key in ("run_id", "attempt", "build_run_id", "build_attempt", "current_id", "previous_id"):
    value = getattr(args, key, None)
    if value is not None and value < 1:
      parser.error("Workflow and artifact identifiers must be positive.")
  if args.command == "prepare":
    owner, repository = args.repository.split("/")
    if args.site_url != f"https://{owner.lower()}.github.io/{repository}":
      parser.error("The site URL must match the repository's project Pages URL.")
    if not re.fullmatch(r"[0-9a-f]{40}", args.commit):
      parser.error("Invalid source commit.")
    prepare(args)
  elif args.command == "source":
    source(args)
  else:
    prune(args)


if __name__ == "__main__":
  main()
