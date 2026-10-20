import importlib.util
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location(
  "pages_state", Path(__file__).resolve().parents[1] / "scripts/pages-state.py",
)
STATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STATE)


def producer(event="push", branch="main"):
  return {
    "workflow_id": 42,
    "head_repository": {"full_name": "example/wodex"},
    "head_branch": branch,
    "event": event,
    "head_sha": "0123456789abcdef0123456789abcdef01234567",
    "run_attempt": 1,
    "run_started_at": "2026-10-20T12:34:56Z",
  }


def publisher():
  return {
    **producer(event="workflow_dispatch"),
    "workflow_id": 43,
    "head_sha": "a" * 40,
  }


def build_jobs():
  return [
    {"name": name, "status": "completed", "conclusion": "success"}
    for name in sorted(STATE.BUILD_JOBS)
  ]


def deployed_job(conclusion="success", verification="success"):
  return {
    "name": "Publish build channels",
    "status": "completed",
    "steps": [
      {"name": "Deploy download page", "conclusion": conclusion},
      {"name": "Verify public downloads", "conclusion": verification},
    ],
  }


class SourceTests(unittest.TestCase):
  def checked(self, run=None, latest=None, jobs=None):
    run = run if run is not None else producer()
    latest = latest if latest is not None else run
    jobs = jobs if jobs is not None else build_jobs()
    def fake_api(repository, endpoint):
      self.assertEqual(repository, "example/wodex")
      return {
        "actions/workflows/build.yml": {"id": 42},
        "actions/runs/9": latest,
        "actions/runs/9/attempts/1": run,
        "actions/runs/9/attempts/1/jobs?per_page=100": {"jobs": jobs},
      }[endpoint]
    with patch.object(STATE, "api", side_effect=fake_api):
      return STATE.checked_source("example/wodex", 9, 1)

  def test_source_uses_build_commit_ref_and_attempt_start_time(self):
    result = self.checked(run={**producer(), "status": "in_progress"})
    self.assertEqual(result, {
      "commit": producer()["head_sha"],
      "ref": "refs/heads/main",
      "channel": "nightly",
      "tag": "",
      "timestamp": "2026-10-20T12:34:56Z",
    })
    tagged = self.checked(run=producer(branch="v0.2.0"))
    self.assertEqual(tagged["ref"], "refs/tags/v0.2.0")
    self.assertEqual(tagged["channel"], "version")
    self.assertEqual(tagged["tag"], "v0.2.0")

  def test_source_rejects_forks_untrusted_workflows_events_and_refs(self):
    for run in [
      {**producer(), "head_repository": {"full_name": "other/wodex"}},
      {**producer(), "workflow_id": 43},
      producer(event="pull_request"),
      producer(event="workflow_run"),
      producer(branch="feature"),
      producer(branch="v01.2.0"),
      producer(branch="v0.2.0-rc1"),
      {**producer(), "head_sha": "bad\ninjected=value"},
    ]:
      with self.subTest(run=run), self.assertRaisesRegex(RuntimeError, "trusted build"):
        self.checked(run=run)

  def test_source_must_be_latest_attempt_and_match_its_run(self):
    for latest in [
      {**producer(), "run_attempt": 2},
      {**producer(), "head_sha": "b" * 40},
      producer(branch="v0.2.0"),
    ]:
      with self.subTest(latest=latest), self.assertRaisesRegex(RuntimeError, "latest attempt"):
        self.checked(latest=latest)

  def test_all_three_jobs_must_have_one_completed_success(self):
    failures = [build_jobs()[1:], build_jobs() + [build_jobs()[0]]]
    for field, value in [("status", "in_progress"), ("conclusion", "failure"), ("conclusion", "skipped")]:
      jobs = build_jobs()
      jobs[0][field] = value
      failures.append(jobs)
    for jobs in failures:
      with self.subTest(jobs=jobs), self.assertRaisesRegex(RuntimeError, "completed successfully"):
        self.checked(jobs=jobs)

  def test_source_timestamp_requires_timezone(self):
    for timestamp in ["2026-10-20T12:34:56", "invalid", None]:
      with self.subTest(timestamp=timestamp), self.assertRaisesRegex(RuntimeError, "timestamp"):
        self.checked(run={**producer(), "run_started_at": timestamp})


class SnapshotTests(unittest.TestCase):
  def test_artifact_order_uses_creation_time_even_when_ids_decrease(self):
    entries = [
      {"id": 999, "name": "wodex-pages-state-8-1", "created_at": "2026-10-19T10:00:00Z"},
      {"id": 100, "name": "wodex-pages-state-9-1", "created_at": "2026-10-20T10:00:00Z"},
      {"id": 101, "name": "wodex-pages-state-10-1", "created_at": "2026-10-20T10:00:00Z"},
    ]
    with patch.object(STATE, "api", return_value={"artifacts": entries}):
      self.assertEqual([item["id"] for item in STATE.artifacts("example/wodex")], [101, 100, 999])

  def test_invalid_artifact_creation_time_fails_closed(self):
    for value in [None, "bad", "2026-10-20Z", "2026-10-20T10:00:00", "2026-02-30T10:00:00Z"]:
      entry = {"id": 1, "name": "wodex-pages-state-8-1", "created_at": value}
      with (
        self.subTest(value=value),
        patch.object(STATE, "api", return_value={"artifacts": [entry]}),
        self.assertRaisesRegex(RuntimeError, "timestamp"),
      ):
        STATE.artifacts("example/wodex")

  def test_prune_uses_current_creation_time_and_preserves_newer_smaller_ids(self):
    entries = [
      {"id": 50, "created_at": "2026-10-21T10:00:00Z"},
      {"id": 100, "created_at": "2026-10-20T10:00:00Z"},
      {"id": 90, "created_at": "2026-10-20T10:00:00Z"},
      {"id": 200, "created_at": "2026-10-19T10:00:00Z"},
      {"id": 999, "created_at": "2026-10-18T10:00:00Z"},
    ]
    args = SimpleNamespace(repository="example/wodex", current_id=100, previous_id=200)
    with patch.object(STATE, "artifacts", return_value=entries), patch.object(STATE, "api") as api:
      STATE.prune(args)
      api.assert_called_once_with("example/wodex", "actions/artifacts/999", method="DELETE")

  def test_prune_requires_current_snapshot_before_deleting_anything(self):
    args = SimpleNamespace(repository="example/wodex", current_id=100, previous_id=200)
    with (
      patch.object(STATE, "artifacts", return_value=[]),
      patch.object(STATE, "api") as api,
      self.assertRaisesRegex(RuntimeError, "not found"),
    ):
      STATE.prune(args)
    api.assert_not_called()

  def test_only_trusted_publication_sources_are_accepted(self):
    self.assertTrue(STATE.trusted_run(producer(), "example/wodex", 42))
    self.assertTrue(STATE.trusted_run(producer(branch="v0.2.0"), "example/wodex", 42))
    for source in [
      producer(event="pull_request"), producer(branch="feature"),
      producer(branch="v01.2.3"), producer(branch="v1.0.0-rc1"),
      {**producer(), "workflow_id": 43},
      {**producer(), "head_repository": {"full_name": "other/wodex"}},
    ]:
      self.assertFalse(STATE.trusted_run(source, "example/wodex", 42))

  def test_restore_uses_deployment_success_even_if_later_cleanup_failed(self):
    job = {**deployed_job(), "conclusion": "failure"}
    self.assertTrue(STATE.deployment_succeeded([job]))
    self.assertFalse(STATE.deployment_succeeded([deployed_job("failure")]))

  def test_accepted_deployment_survives_failed_public_verification(self):
    self.assertTrue(STATE.deployment_succeeded([deployed_job(verification="failure")]))
    self.assertFalse(STATE.deployment_succeeded([deployed_job("failure", "success")]))

  def test_only_main_branch_dispatches_are_trusted_publishers(self):
    self.assertTrue(STATE.trusted_publisher(publisher(), "example/wodex", 43))
    for run in [
      {**publisher(), "event": "push"},
      {**publisher(), "head_branch": "v0.2.0"},
      {**publisher(), "workflow_id": 42},
      {**publisher(), "head_repository": {"full_name": "other/wodex"}},
    ]:
      self.assertFalse(STATE.trusted_publisher(run, "example/wodex", 43))

  def test_failed_newer_snapshot_does_not_replace_last_deployed_state(self):
    candidates = [
      {"id": 102, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}},
      {"id": 101, "name": "wodex-pages-state-8-1", "workflow_run": {"id": 8}},
    ]
    def fake_api(repository, endpoint):
      if endpoint.endswith("/jobs?per_page=100"):
        return {"jobs": [deployed_job("failure" if "/9/" in endpoint else "success")]}
      return publisher()
    with patch.object(STATE, "api", side_effect=fake_api):
      self.assertEqual(STATE.select_snapshot("example/wodex", candidates, 43, 42, 10, 1)["id"], 101)

  def test_rerun_can_restore_previous_attempt_of_same_run(self):
    candidate = {"id": 101, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}}
    with patch.object(STATE, "api", side_effect=[publisher(), {"jobs": [deployed_job()]}]):
      self.assertEqual(STATE.select_snapshot("example/wodex", [candidate], 43, 42, 9, 2), candidate)

  def test_legacy_snapshots_are_limited_to_original_pairs_and_commit(self):
    legacy_id = 39397383528
    candidate = {
      "id": 101, "name": f"wodex-pages-state-{legacy_id}-1",
      "workflow_run": {"id": legacy_id},
    }
    old = {**producer(branch="v0.2.0"), "head_sha": STATE.LEGACY_COMMIT}
    with patch.object(STATE, "api", side_effect=[old, {"jobs": [deployed_job()]}]):
      self.assertEqual(STATE.select_snapshot("example/wodex", [candidate], 43, 42, 10, 1), candidate)
    for run, attempt in [(producer(branch="v0.2.0"), 1), (old, 2)]:
      item = {**candidate, "name": f"wodex-pages-state-{legacy_id}-{attempt}"}
      with patch.object(STATE, "api", return_value=run):
        self.assertIsNone(STATE.select_snapshot("example/wodex", [item], 43, 42, 10, 1))

  def test_unresolved_preceding_deployment_prevents_rollback_to_older_state(self):
    candidate = {"id": 101, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}}
    job = {**deployed_job(None), "status": "in_progress"}
    with (
      patch.object(STATE, "api", side_effect=[publisher()] + [{"jobs": [job]}] * 7),
      patch.object(STATE.time, "sleep"),
      self.assertRaisesRegex(RuntimeError, "unresolved"),
    ):
      STATE.select_snapshot("example/wodex", [candidate], 43, 42, 10, 1)

  def test_expired_last_deployed_state_fails_closed(self):
    candidate = {
      "id": 101, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}, "expired": True,
    }
    with patch.object(STATE, "api", side_effect=[publisher(), {"jobs": [deployed_job()]}]):
      with self.assertRaisesRegex(RuntimeError, "expired"):
        STATE.select_snapshot("example/wodex", [candidate], 43, 42, 10, 1)

  def extract(self, entries):
    with tempfile.TemporaryDirectory() as directory:
      root = Path(directory)
      archive = root / "snapshot.zip"
      with zipfile.ZipFile(archive, "w") as output:
        output.writestr("catalog.json", "{}")
        for name, value in entries:
          output.writestr(name, value)
      STATE.extract_snapshot(archive, root / "previous")
      self.assertEqual((root / "previous/catalog.json").read_text(), "{}")

  def test_valid_snapshot_extracts(self):
    self.extract([("builds/version/0.2.0/wodex.nro", b"NRO")])

  def test_archive_escape_absolute_path_and_symlink_are_rejected(self):
    link = zipfile.ZipInfo("builds/link")
    link.create_system = 3
    link.external_attr = (stat.S_IFLNK | 0o777) << 16
    for name in ["../escape", "/absolute", "builds\\escape", link, "./catalog.json"]:
      with self.subTest(name=name):
        with self.assertRaisesRegex(RuntimeError, "unsafe"):
          self.extract([(name, b"target")])

  def test_archive_limits_are_enforced_before_extraction(self):
    with patch.object(STATE, "MAX_SNAPSHOT_BYTES", 4):
      with self.assertRaisesRegex(RuntimeError, "bounds"):
        self.extract([("file", b"too large")])


class PrepareTests(unittest.TestCase):
  def prepare(self, root, *, checkout=None, run=None, commit=None, ref=None, metadata=None):
    artifact = root / "artifact"
    artifact.mkdir(exist_ok=True)
    (artifact / "VERSION").write_text("0.2.0\n")
    source = metadata or {
      "commit": producer()["head_sha"], "ref": "refs/tags/v0.2.0",
      "channel": "version", "tag": "v0.2.0", "timestamp": "2026-10-20T12:34:56Z",
    }
    args = SimpleNamespace(
      repository="example/wodex", run_id=10, attempt=1, build_run_id=9, build_attempt=1,
      commit=commit or source["commit"], ref=ref or source["ref"],
      artifact_dir=artifact, previous_dir=root / "previous", github_output=root / "output",
      site_url="https://example.github.io/wodex",
    )
    responses = {
      "actions/runs/10/attempts/1": run or publisher(),
      "actions/workflows/publish.yml": {"id": 43},
      "actions/workflows/build.yml": {"id": 42},
    }
    with (
      patch.object(STATE, "api", side_effect=lambda repo, endpoint: responses[endpoint]),
      patch.object(STATE, "checked_source", return_value=source) as validated,
      patch.object(STATE, "artifacts", return_value=[]),
      patch.object(STATE, "assert_empty_site") as empty,
      patch.object(STATE.subprocess, "check_output", return_value=(checkout or publisher()["head_sha"]) + "\n"),
      patch.object(STATE.subprocess, "run") as ancestor,
    ):
      STATE.prepare(args)
      validated.assert_called_once_with("example/wodex", 9, 1)
      ancestor.assert_called_once_with([
        "git", "merge-base", "--is-ancestor", source["commit"], "origin/main",
      ], check=True)
      empty.assert_called_once_with(args.site_url)
    return args.github_output.read_text()

  def test_publisher_checkout_is_main_while_artifact_commit_can_be_older(self):
    with tempfile.TemporaryDirectory() as directory:
      output = self.prepare(Path(directory))
      self.assertIn("commit=" + producer()["head_sha"], output)
      self.assertIn("channel=version\n", output)
      self.assertIn("has_previous=false\n", output)

  def test_prepare_rejects_untrusted_current_run_source_mismatch_and_wrong_checkout(self):
    cases = [
      {"run": producer()},
      {"commit": "b" * 40},
      {"ref": "refs/heads/main"},
      {"checkout": producer()["head_sha"]},
      {"metadata": {
        "commit": producer()["head_sha"], "ref": "refs/tags/v0.3.0",
        "channel": "version", "tag": "v0.3.0", "timestamp": "2026-10-20T12:34:56Z",
      }},
    ]
    for values in cases:
      with self.subTest(values=values), tempfile.TemporaryDirectory() as directory:
        with self.assertRaises(RuntimeError):
          self.prepare(Path(directory), **values)


if __name__ == "__main__":
  unittest.main()
