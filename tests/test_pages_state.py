import importlib.util
from pathlib import Path
import stat
import tempfile
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
  }


def deployed_job(conclusion="success"):
  return {
    "name": "Publish build channels",
    "status": "completed",
    "steps": [{"name": "Deploy download page", "conclusion": conclusion}],
  }


class SnapshotTests(unittest.TestCase):
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

  def test_failed_newer_snapshot_does_not_replace_last_deployed_state(self):
    candidates = [
      {"id": 102, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}},
      {"id": 101, "name": "wodex-pages-state-8-1", "workflow_run": {"id": 8}},
    ]
    def fake_api(repository, endpoint):
      if endpoint.endswith("/jobs?per_page=100"):
        return {"jobs": [deployed_job("failure" if "/9/" in endpoint else "success")]}
      return producer()
    with patch.object(STATE, "api", side_effect=fake_api):
      self.assertEqual(STATE.select_snapshot("example/wodex", candidates, 42, 10, 1)["id"], 101)

  def test_rerun_can_restore_previous_attempt_of_same_run(self):
    candidate = {"id": 101, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}}
    with patch.object(STATE, "api", side_effect=[producer(), {"jobs": [deployed_job()]}]):
      self.assertEqual(STATE.select_snapshot("example/wodex", [candidate], 42, 9, 2), candidate)

  def test_expired_last_deployed_state_fails_closed(self):
    candidate = {
      "id": 101, "name": "wodex-pages-state-9-1", "workflow_run": {"id": 9}, "expired": True,
    }
    with patch.object(STATE, "api", side_effect=[producer(), {"jobs": [deployed_job()]}]):
      with self.assertRaisesRegex(RuntimeError, "expired"):
        STATE.select_snapshot("example/wodex", [candidate], 42, 10, 1)

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


if __name__ == "__main__":
  unittest.main()
