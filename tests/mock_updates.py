#!/usr/bin/env python3
"""Exercise GitHub Pages update checks and downloads without external network access."""

import hashlib
import http.server
import json
import os
import struct
import subprocess
import sys
import threading
import time
import urllib.parse


def nro(marker=2, size=4096):
  payload = bytearray(size)
  payload[0x10:0x14] = b"NRO0"
  struct.pack_into("<I", payload, 0x18, 0x200)
  struct.pack_into("<IIIIII", payload, 0x20, 0, 0x100, 0x100, 0x80, 0x180, 0x80)
  payload[0x90] = marker
  payload[0x200:0x204] = b"ASET"
  struct.pack_into("<QQ", payload, 0x228, 56, size - 0x200 - 56)
  romfs = 0x200 + 56
  struct.pack_into("<10Q", payload, romfs, 80, 80, 4, 84, 24, 108, 4, 112, 36, 160)
  struct.pack_into("<6I", payload, romfs + 84, 0, 0xffffffff, 0xffffffff, 0, 0xffffffff, 0)
  struct.pack_into("<IIQQII", payload, romfs + 112, 0, 0xffffffff, 0, 1, 0xffffffff, 4)
  payload[romfs + 144:romfs + 148] = b"data"
  payload[romfs + 160] = 0x5a
  return bytes(payload)


def asset(scenario):
  payload = bytearray(nro(size=1024 * 1024 if scenario == "slow" else 4096))
  if scenario == "malformed":
    payload[0x10] = 0
  if scenario == "aset":
    struct.pack_into("<Q", payload, 0x228, len(payload) + 1)
  if scenario == "no-assets":
    payload = payload[:0x200]
  if scenario == "zero-romfs":
    struct.pack_into("<Q", payload, 0x230, 0)
  if scenario == "romfs-header":
    struct.pack_into("<Q", payload, 0x238 + 24, len(payload) + 1)
  if scenario == "romfs-root":
    struct.pack_into("<I", payload, 0x238 + 84 + 20, 0xffffffff)
  if scenario == "romfs-data":
    struct.pack_into("<Q", payload, 0x238 + 72, len(payload) + 1)
  return bytes(payload)


class Server(http.server.ThreadingHTTPServer):
  daemon_threads = True

  def __init__(self):
    super().__init__(("127.0.0.1", 0), Handler)
    self.failures = []
    self.lock = threading.Lock()
    self.direct_downloads = 0
    self.nightly_downloads = 0


class Handler(http.server.BaseHTTPRequestHandler):
  protocol_version = "HTTP/1.1"

  def log_message(self, *_args):
    pass

  def reply(self, status, payload=b"", content_type="application/json", location=None):
    self.send_response(status)
    self.send_header("Content-Type", content_type)
    self.send_header("Content-Length", str(len(payload)))
    self.send_header("Connection", "close")
    if location:
      self.send_header("Location", location)
    self.end_headers()
    self.wfile.write(payload)
    self.close_connection = True

  def do_GET(self):
    try:
      assert self.headers.get("Authorization") is None, "Updater sent an authorization credential"
      assert self.headers.get("Cookie") is None, "Updater sent a cookie"
      parsed = urllib.parse.urlsplit(self.path)
      parts = parsed.path.strip("/").split("/")
      assert not parsed.query and not parsed.fragment, "Update URL contained a query or fragment"
      if len(parts) == 4 and parts[0] == "fixture.github.io" and parts[2] == "updates" and parts[3] in ("stable.json", "nightly.json"):
        scenario = parts[1]
        if scenario == "http-error":
          self.reply(503, b"private-update-body")
          return
        if scenario == "bad-json":
          self.reply(200, b"private-update-body invalid JSON")
          return
        if scenario == "oversized-metadata":
          self.reply(200, b"x" * (1024 * 1024 + 1))
          return
        if scenario == "manifest-redirect":
          self.reply(302, b"private-update-body", location="https://fixture.github.io/good/updates/stable.json")
          return
        payload = asset(scenario)
        size = len(payload) + (1 if scenario == "short" else -1 if scenario == "long" else 0)
        sha256 = "0" * 64 if scenario == "hash" else hashlib.sha256(payload).hexdigest()
        metadata = {
          "schema_version": 1, "channel": "version",
          "build": {
            "version": "0.3.0", "size": size, "notes": "Verified mock update\n包含字体",
            "url": "https://fixture.github.io/" + ("good" if scenario == "wrong-repository" else scenario) + "/builds/version/0.3.0/wodex.nro",
            "sha256": sha256,
          },
        }
        if parts[3] == "nightly.json":
          metadata["channel"] = "nightly"
          metadata["build"].update({
            "id": "20261020T010203Z-abcdef0",
            "timestamp": "2026-10-20T01:02:03Z",
            "commit": "abcdef0123456789abcdef0123456789abcdef01",
            "url": "https://fixture.github.io/" + scenario + "/builds/nightly/20261020T010203Z-abcdef0/wodex.nro",
          })
          if scenario == "nightly-empty":
            metadata["build"] = None
          if scenario == "nightly-bad-id":
            metadata["build"]["id"] = "20261020T010203Z-0000000"
          if scenario == "nightly-bad-date":
            metadata["build"]["timestamp"] = "2026-02-29T01:02:03Z"
          if scenario == "nightly-missing-commit":
            del metadata["build"]["commit"]
          if scenario == "nightly-wrong-path":
            metadata["build"]["url"] = "https://fixture.github.io/" + scenario + "/builds/version/0.3.0/wodex.nro"
          if scenario == "nightly-wrong-channel":
            metadata["channel"] = "version"
        if scenario == "empty":
          metadata["build"] = None
        if scenario == "nightly-feed":
          metadata["channel"] = "nightly"
        if scenario == "unknown-schema":
          metadata["schema_version"] = 2
        if scenario == "missing-checksum":
          del metadata["build"]["sha256"]
        if scenario == "missing-build":
          del metadata["build"]
        if scenario == "wrong-owner":
          metadata["build"]["url"] = "https://elsewhere.github.io/wrong-owner/builds/version/0.3.0/wodex.nro"
        self.reply(200, json.dumps(metadata, ensure_ascii=False).encode("utf-8"))
      elif len(parts) == 6 and parts[0] == "fixture.github.io" and parts[2:] == ["builds", "nightly", "20261020T010203Z-abcdef0", "wodex.nro"]:
        assert parts[1] == "nightly-good", "An unverified nightly was downloaded"
        with self.server.lock:
          self.server.nightly_downloads += 1
        self.reply(200, asset(parts[1]), "application/octet-stream")
      elif len(parts) == 6 and parts[0] == "fixture.github.io" and parts[2:] == ["builds", "version", "0.3.0", "wodex.nro"]:
        scenario = parts[1]
        redirects = {
          "bad-redirect": "https://untrusted.example/private-update-body",
          "http-redirect": "http://fixture.github.io/good/builds/version/0.3.0/wodex.nro",
          "credentials-redirect": "https://user:password@fixture.github.io/good/builds/version/0.3.0/wodex.nro",
          "loop": "https://fixture.github.io/loop/builds/version/0.3.0/wodex.nro",
          "same-project-redirect": "/same-project-redirect/builds/version/0.3.0/other.nro",
          "cross-project-redirect": "https://fixture.github.io/good/builds/version/0.3.0/wodex.nro",
        }
        if scenario in redirects:
          self.reply(302, b"private-update-body", location=redirects[scenario])
        elif scenario == "slow":
          payload = asset(scenario)
          self.send_response(200)
          self.send_header("Content-Type", "application/octet-stream")
          self.send_header("Content-Length", str(len(payload)))
          self.send_header("Connection", "close")
          self.end_headers()
          for offset in range(0, len(payload), 1024):
            self.wfile.write(payload[offset:offset + 1024])
            self.wfile.flush()
            time.sleep(0.05)
          self.close_connection = True
        else:
          if scenario == "good":
            with self.server.lock:
              self.server.direct_downloads += 1
          self.reply(200, asset(scenario), "application/octet-stream")
      else:
        raise AssertionError("A disallowed host, path, or traversal reached the update server")
    except (BrokenPipeError, ConnectionResetError):
      pass
    except Exception as error:
      with self.server.lock:
        self.server.failures.append(str(error))
      try:
        self.reply(400, b"private-update-body")
      except (BrokenPipeError, ConnectionResetError):
        pass


def main():
  if len(sys.argv) not in (2, 3):
    raise SystemExit("Usage: mock_updates.py /path/to/wodex_update_tests [real-artifact.nro]")
  server = Server()
  thread = threading.Thread(target=server.serve_forever, daemon=True)
  thread.start()
  try:
    environment = os.environ.copy()
    environment["DEEPSEEK_API_KEY"] = "mock-chat-key-must-never-be-sent-to-updates"
    result = subprocess.run([sys.argv[1], "http://127.0.0.1:" + str(server.server_port)] + sys.argv[2:],
      env=environment, timeout=60, check=False)
  finally:
    server.shutdown()
    server.server_close()
    thread.join(timeout=2)
  if server.failures:
    print("Update mock validation failed: " + "; ".join(server.failures), file=sys.stderr)
    return 1
  if result.returncode == 0 and server.direct_downloads != 2:
    print("The expected version artifacts were not downloaded directly from the project Pages paths.", file=sys.stderr)
    return 1
  if result.returncode == 0 and server.nightly_downloads != 1:
    print("The nightly artifact was not downloaded from its exact project Pages path.", file=sys.stderr)
    return 1
  return result.returncode


if __name__ == "__main__":
  raise SystemExit(main())
