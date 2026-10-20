import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request

SPEC = importlib.util.spec_from_file_location(
  "verify_pages", Path(__file__).resolve().parents[1] / "scripts/verify-pages.py",
)
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)
SITE = "https://example.github.io/Wodex"


class Clock:
  def __init__(self):
    self.now = 0
    self.sleeps = []

  def __call__(self):
    return self.now

  def sleep(self, delay):
    self.sleeps.append(delay)
    self.now += delay


class Response(io.BytesIO):
  def __init__(self, payload, url, headers=None):
    super().__init__(payload)
    self.url = url
    self.status = 200
    self.headers = headers or {}
    self.read_sizes = []

  def geturl(self):
    return self.url

  def read(self, size=-1):
    self.read_sizes.append(size)
    if size < 0 or size > VERIFY.CHUNK_BYTES:
      raise AssertionError("The verifier tried to buffer an unbounded response.")
    return super().read(size)


class VerificationTests(unittest.TestCase):
  def setUp(self):
    self.temporary = tempfile.TemporaryDirectory()
    self.addCleanup(self.temporary.cleanup)
    self.root = Path(self.temporary.name)
    self.clock = Clock()
    self.requests = []
    self.responses = []
    self.logs = []
    self.payloads = {}
    builds = []
    for channel, identity, marker in [
      ("nightly", "20261020T120000Z-abcdef0", b"nightly"),
      ("version", "0.2.0", b"version"),
    ]:
      binary = marker * 25000
      checksum = hashlib.sha256(binary).hexdigest()
      files = {
        "wodex.nro": binary,
        "wodex.nro.sha256": f"{checksum}  wodex.nro\n".encode(),
      }
      assets = {}
      for name, payload in files.items():
        self.write(f"builds/{channel}/{identity}/{name}", payload)
        assets[name] = {"size": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}
      builds.append({"channel": channel, "id": identity, "assets": assets})
    self.write("catalog.json", json.dumps({"schema_version": 1, "builds": builds}).encode())
    self.write("updates/stable.json", b'{"schema_version":1,"channel":"version","build":null}')
    self.write("updates/nightly.json", b'{"schema_version":1,"channel":"nightly","build":null}')
    self.write("index.html", b'<main><section id="nightly"></section><section id="version"></section></main>')

  def write(self, relative, payload):
    path = self.root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    self.payloads[relative] = payload

  def opener(self, request, timeout):
    self.requests.append((request, timeout))
    relative = urllib.parse.urlsplit(request.full_url).path.removeprefix("/Wodex/")
    response = Response(self.payloads[relative], request.full_url)
    self.responses.append(response)
    return response

  def run_verify(self, **options):
    VERIFY.verify_site(
      SITE, self.root, opener=options.pop("opener", self.opener),
      clock=self.clock, sleep=self.clock.sleep, report=self.logs.append,
      budget_seconds=options.pop("budget_seconds", 60), retry_seconds=25, **options,
    )

  def test_exact_site_checks_both_channels_with_bounded_streaming(self):
    self.run_verify()
    self.assertEqual(len(self.requests), 8)
    self.assertEqual(self.clock.sleeps, [])
    binary_responses = [
      response for response in self.responses
      if "/wodex.nro?" in response.url
    ]
    self.assertEqual(len(binary_responses), 2)
    self.assertTrue(all(len(response.read_sizes) > 2 for response in binary_responses))
    for request, timeout in self.requests:
      self.assertEqual(timeout, 20)
      self.assertIn("publication_check=", request.full_url)
      self.assertIsNone(request.get_header("Authorization"))
      self.assertIsNone(request.get_header("Cookie"))
      self.assertEqual(request.get_header("User-agent"), "Wodexpublication-check")

  def test_old_catalog_retries_before_any_payload_then_succeeds(self):
    def stale_once(request, timeout):
      if not self.requests:
        self.requests.append((request, timeout))
        return Response(b"old catalog", request.full_url)
      return self.opener(request, timeout)
    self.run_verify(opener=stale_once)
    self.assertEqual(self.clock.sleeps, [25])
    self.assertEqual(len(self.requests), 9)
    self.assertIn("/catalog.json?", self.requests[0][0].full_url)
    self.assertIn("/catalog.json?", self.requests[1][0].full_url)

  def test_hash_mismatch_never_reports_success(self):
    def wrong_binary(request, timeout):
      response = self.opener(request, timeout)
      if "/wodex.nro?" in request.full_url:
        return Response(b"x" * len(response.getvalue()), request.full_url)
      return response
    with self.assertRaises(VERIFY.PublicationTimeout):
      self.run_verify(opener=wrong_binary, budget_seconds=30)
    self.assertEqual(self.clock.now, 30)
    self.assertEqual(self.clock.sleeps, [25, 5])
    self.assertFalse(any(line.startswith("Verified") for line in self.logs))
    self.assertTrue(any("does not match" in line for line in self.logs))

  def test_stale_update_feed_or_page_prevents_payload_downloads(self):
    for relative in ["updates/stable.json", "updates/nightly.json", "index.html"]:
      with self.subTest(relative=relative):
        original = self.payloads[relative]
        self.payloads[relative] = b"x" * len(original)
        self.clock = Clock()
        self.requests = []
        try:
          with self.assertRaises(VERIFY.PublicationTimeout):
            self.run_verify(budget_seconds=1)
          self.assertFalse(any("/builds/" in request.full_url for request, _ in self.requests))
        finally:
          self.payloads[relative] = original

  def test_missing_generated_nightly_feed_fails_before_network(self):
    (self.root / "updates/nightly.json").unlink()
    with self.assertRaisesRegex(VERIFY.VerificationError, "Cannot read generated file"):
      self.run_verify()
    self.assertEqual(self.requests, [])

  def test_redirect_handler_rejects_and_unexpected_final_urls_fail(self):
    handler = VERIFY.NoRedirect()
    with self.assertRaisesRegex(VERIFY.VerificationError, "redirected"):
      handler.redirect_request(None, None, 302, "Found", {}, "https://elsewhere.test/")
    def redirected(request, timeout):
      self.requests.append((request, timeout))
      return Response(self.payloads["catalog.json"], "https://elsewhere.test/")
    with self.assertRaises(VERIFY.PublicationTimeout):
      self.run_verify(opener=redirected, budget_seconds=1)
    self.assertEqual(len(self.requests), 1)
    self.assertIn("redirected", self.logs[0])

  def test_invalid_site_urls_fail_before_a_network_request(self):
    for url in [
      "http://example.github.io/Wodex", "https://example.github.io/Wodex?secret=x",
      "https://user:secret@example.github.io/Wodex", "https://example.github.io:443/Wodex",
      "https://example.github.io.evil.test/Wodex", "https://example.github.io/../Wodex",
      "https://example.github.io/Wodex#fragment", "https://example.github.io/Wodex/extra",
      "https://example.github.io/Wodex\\other", "https://example.github.io/.",
    ]:
      with self.subTest(url=url):
        with self.assertRaisesRegex(VERIFY.VerificationError, "direct HTTPS"):
          VERIFY.verify_site(url, self.root, opener=self.opener)
    self.assertEqual(self.requests, [])

  def test_elapsed_request_timeout_uses_remaining_budget_and_stops(self):
    def timed_out(request, timeout):
      self.requests.append((request, timeout))
      self.clock.now += timeout
      raise TimeoutError("Simulated socket timeout")
    with self.assertRaises(VERIFY.PublicationTimeout):
      self.run_verify(opener=timed_out, budget_seconds=7)
    self.assertEqual(self.requests[0][1], 7)
    self.assertEqual(self.clock.sleeps, [])
    self.assertEqual(self.clock.now, 7)

  def test_blocked_opener_cannot_overrun_the_whole_request_budget(self):
    blocked = threading.Event()
    def opener(request, timeout):
      blocked.wait()
      raise TimeoutError("Released test request")
    started = time.monotonic()
    try:
      with self.assertRaises(VERIFY.PublicationTimeout):
        VERIFY.verify_site(
          SITE, self.root, opener=opener, budget_seconds=0.03, report=self.logs.append,
        )
      self.assertLess(time.monotonic() - started, 0.5)
    finally:
      blocked.set()

  def test_http_errors_and_truncated_metadata_are_retryable_and_bounded(self):
    for failure in ["http", "truncated"]:
      with self.subTest(failure=failure):
        self.clock = Clock()
        self.requests = []
        def bad_response(request, timeout):
          self.requests.append((request, timeout))
          if failure == "http":
            raise urllib.error.HTTPError(request.full_url, 404, "Not Found", {}, None)
          return Response(self.payloads["catalog.json"][:-1], request.full_url)
        with self.assertRaises(VERIFY.PublicationTimeout):
          self.run_verify(opener=bad_response, budget_seconds=1)
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(self.clock.now, 1)

  def test_generated_catalog_paths_and_local_checksums_are_validated(self):
    catalog = json.loads(self.payloads["catalog.json"])
    catalog["builds"][0]["id"] = "../escape"
    self.write("catalog.json", json.dumps(catalog).encode())
    with self.assertRaisesRegex(VERIFY.VerificationError, "unsafe build path"):
      self.run_verify()
    catalog["builds"][0]["id"] = "20261020T120000Z-abcdef0"
    self.write("catalog.json", json.dumps(catalog).encode())
    local = self.root / "builds/version/0.2.0/wodex.nro"
    local.write_bytes(b"x" * local.stat().st_size)
    with self.assertRaisesRegex(VERIFY.VerificationError, "does not match its catalog"):
      self.run_verify()
    self.assertEqual(self.requests, [])


if __name__ == "__main__":
  unittest.main()
