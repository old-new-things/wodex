import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
  "pages_build", Path(__file__).resolve().parents[1] / "scripts/pages-build.py",
)
BUILD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD)

SHA = "0123456789abcdef0123456789abcdef01234567"


class BuildIdentityTests(unittest.TestCase):
  def identity(self, run=None, **overrides):
    context = {
      "repository": BUILD.REPOSITORY, "run_id": 9, "attempt": 2,
      "commit": SHA, "ref": "refs/heads/main", "event": "push", "version": "0.3.0",
      **overrides,
    }
    producer = {
      "id": 9, "run_attempt": 2, "workflow_id": 42,
      "head_repository": {"full_name": BUILD.REPOSITORY},
      "head_sha": SHA, "head_branch": "main", "event": "push",
      "run_started_at": "2026-10-20T12:34:56Z", **(run or {}),
    }
    def fake_api(repository, endpoint):
      self.assertEqual(repository, BUILD.REPOSITORY)
      return {
        "actions/workflows/build.yml": {"id": 42},
        "actions/runs/9/attempts/2": producer,
      }[endpoint]
    with patch.object(BUILD, "api", side_effect=fake_api):
      return BUILD.identity(**context)

  def test_nightly_uses_actual_attempt_timestamp_and_commit(self):
    self.assertEqual(self.identity(), {
      "schema_version": 1, "version": "0.3.0", "channel": "nightly",
      "timestamp": "2026-10-20T12:34:56Z", "commit": SHA,
    })
    self.assertEqual(
      self.identity(run={"run_started_at": "2026-10-20T20:34:56+08:00"})["timestamp"],
      "2026-10-20T12:34:56Z",
    )

  def test_explicit_version_must_match_tag(self):
    result = self.identity(run={"head_branch": "v0.3.0"}, ref="refs/tags/v0.3.0")
    self.assertEqual(result["channel"], "version")
    for tag in ["v0.2.0", "v00.3.0", "v0.3.0-rc1"]:
      with self.subTest(tag=tag), self.assertRaisesRegex(RuntimeError, "tag"):
        self.identity(ref=f"refs/tags/{tag}")

  def test_other_branches_prs_and_forks_are_development(self):
    for override in [
      {"ref": "refs/heads/feature"}, {"event": "pull_request", "ref": "refs/pull/1/merge"},
      {"repository": "example/fork"}, {"event": "workflow_run"},
    ]:
      with self.subTest(override=override):
        result = self.identity(**override)
        self.assertEqual(result["channel"], "development")
        self.assertEqual(result["timestamp"], "")
        self.assertEqual(result["commit"], "")

  def test_attempt_cannot_claim_other_source_metadata(self):
    for override in [
      {"id": 10}, {"run_attempt": 1}, {"workflow_id": 43}, {"head_sha": "a" * 40},
      {"head_repository": {"full_name": "other/wodex"}}, {"head_branch": "feature"},
      {"event": "pull_request"},
    ]:
      with self.subTest(override=override), self.assertRaisesRegex(RuntimeError, "context"):
        self.identity(run=override)

  def test_invalid_and_naive_timestamps_fail(self):
    for timestamp in [None, "invalid", "2026-10-20T12:34:56", "2026-02-30T12:34:56Z"]:
      with self.subTest(timestamp=timestamp), self.assertRaisesRegex(RuntimeError, "timestamp"):
        self.identity(run={"run_started_at": timestamp})

  def test_invalid_version_and_commit_cannot_inject_outputs(self):
    for version in ["0.3.0\nother=value", "00.3.0", "v0.3.0", "0.3.0\n"]:
      with self.subTest(version=version), self.assertRaises(RuntimeError):
        self.identity(version=version)
    with self.assertRaises(RuntimeError):
      self.identity(commit=SHA + "\nother=value")
