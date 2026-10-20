#!/usr/bin/env python3
"""Exercise GitHub update checks and downloads without external network access."""

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
    self.redirected_downloads = 0


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
      if len(parts) == 6 and parts[:3] == ["api.github.com", "repos", "fixture"] and parts[4:] == ["releases", "latest"]:
        scenario = parts[3]
        if scenario == "http-error":
          self.reply(503, b"private-update-body")
          return
        if scenario == "bad-json":
          self.reply(200, b"private-update-body invalid JSON")
          return
        if scenario == "oversized-metadata":
          self.reply(200, b"x" * (2 * 1024 * 1024 + 1))
          return
        payload = asset(scenario)
        size = len(payload) + (1 if scenario == "short" else -1 if scenario == "long" else 0)
        sha256 = "0" * 64 if scenario == "hash" else hashlib.sha256(payload).hexdigest()
        release = {
          "tag_name": "v0.3.0", "draft": False, "prerelease": False,
          "body": "Verified mock update\n包含字体",
          "assets": [{
            "name": "wodex.nro", "state": "uploaded", "size": size,
            "browser_download_url": "https://github.com/fixture/" + ("good" if scenario == "wrong-repository" else scenario) + "/releases/download/v0.3.0/wodex.nro",
            "digest": "sha256:" + sha256,
          }],
        }
        self.reply(200, json.dumps(release, ensure_ascii=False).encode("utf-8"))
      elif len(parts) == 7 and parts[:2] == ["github.com", "fixture"] and parts[3:] == ["releases", "download", "v0.3.0", "wodex.nro"]:
        scenario = parts[2]
        redirects = {
          "bad-redirect": "https://untrusted.example/private-update-body",
          "http-redirect": "http://github.com/fixture/good/releases/download/v0.3.0/wodex.nro",
          "credentials-redirect": "https://user:password@github.com/fixture/good/releases/download/v0.3.0/wodex.nro",
          "loop": "https://github.com/fixture/loop/releases/download/v0.3.0/wodex.nro",
          "good": "https://release-assets.githubusercontent.com/assets/good?signature=mock-query",
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
          self.reply(200, asset(scenario), "application/octet-stream")
      elif parsed.path == "/release-assets.githubusercontent.com/assets/good":
        assert parsed.query == "signature=mock-query", "The allowed redirect lost its signed query"
        with self.server.lock:
          self.server.redirected_downloads += 1
        self.reply(200, asset("good"), "application/octet-stream")
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
  if result.returncode == 0 and server.redirected_downloads != 2:
    print("The expected allowed release-asset redirects were not followed.", file=sys.stderr)
    return 1
  return result.returncode


if __name__ == "__main__":
  raise SystemExit(main())
