#!/usr/bin/env python3
"""Derive embedded build identity from the same Actions attempt used by Pages."""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess

VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
REPOSITORY = "old-new-things/wodex"


def api(repository, endpoint):
  return json.loads(subprocess.check_output(["gh", "api", f"repos/{repository}/{endpoint}"]))


def identity(repository, run_id, attempt, commit, ref, event, version):
  if not VERSION.fullmatch(version) or len(version) > 15:
    raise RuntimeError("VERSION must be a canonical X.Y.Z value of at most 15 characters.")
  if not re.fullmatch(r"[0-9a-f]{40}", commit):
    raise RuntimeError("The build commit must be a full lowercase SHA.")
  metadata = {
    "schema_version": 1, "version": version, "channel": "development",
    "timestamp": "", "commit": "",
  }
  if (
    repository != REPOSITORY
    or event not in {"push", "schedule", "workflow_dispatch"}
    or not (ref == "refs/heads/main" or ref.startswith("refs/tags/v"))
  ):
    return metadata
  channel = "nightly" if ref == "refs/heads/main" else "version"
  if channel == "version" and ref != f"refs/tags/v{version}":
    raise RuntimeError("The version tag must match VERSION exactly.")
  workflow = api(repository, "actions/workflows/build.yml")
  run = api(repository, f"actions/runs/{run_id}/attempts/{attempt}")
  if (
    run.get("id") != run_id
    or run.get("run_attempt") != attempt
    or run.get("workflow_id") != workflow["id"]
    or (run.get("head_repository") or {}).get("full_name") != repository
    or run.get("head_sha") != commit
    or run.get("head_branch") != ref.removeprefix("refs/heads/").removeprefix("refs/tags/")
    or run.get("event") != event
  ):
    raise RuntimeError("The build context does not match its Actions workflow attempt.")
  try:
    raw_timestamp = run["run_started_at"]
    if not isinstance(raw_timestamp, str):
      raise ValueError("Missing timestamp.")
    started = datetime.fromisoformat(raw_timestamp.replace("Z", "+00:00"))
    if started.tzinfo is None:
      raise ValueError("Missing timezone.")
    timestamp = started.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if not re.fullmatch(r"20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", timestamp):
      raise ValueError("Unsupported timestamp.")
  except (KeyError, TypeError, ValueError) as error:
    raise RuntimeError("The build attempt timestamp is invalid.") from error
  metadata.update(channel=channel, timestamp=timestamp, commit=commit)
  return metadata


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--repository", required=True)
  parser.add_argument("--run-id", required=True, type=int)
  parser.add_argument("--attempt", required=True, type=int)
  parser.add_argument("--commit", required=True)
  parser.add_argument("--ref", required=True)
  parser.add_argument("--event", required=True)
  parser.add_argument("--version-file", type=Path, default=Path("VERSION"))
  parser.add_argument("--github-output", type=Path, required=True)
  args = parser.parse_args()
  if args.run_id < 1 or args.attempt < 1:
    parser.error("Run ID and attempt must be positive.")
  raw_version = args.version_file.read_text()
  version = raw_version.removesuffix("\n")
  metadata = identity(
    args.repository, args.run_id, args.attempt, args.commit, args.ref, args.event, version,
  )
  outputs = {
    "channel": "" if metadata["channel"] == "development" else metadata["channel"],
    "timestamp": metadata["timestamp"],
    "commit": metadata["commit"],
    "metadata": json.dumps(metadata, separators=(",", ":")),
  }
  with args.github_output.open("a") as output:
    for key, value in outputs.items():
      output.write(f"{key}={value}\n")
  print(f"Prepared {metadata['channel']} build identity.")


if __name__ == "__main__":
  main()
