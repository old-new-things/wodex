#!/usr/bin/env python3
"""Exercise production image loading and drawing against an isolated loopback server."""

from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib


FIXTURES = Path(__file__).resolve().parent / "fixtures"


def oversized_png():
  """A valid IHDR CRC with dimensions rejected before any pixel allocation."""
  header = b"IHDR" + struct.pack(">IIBBBBB", 4097, 1, 8, 6, 0, 0, 0)
  return b"\x89PNG\r\n\x1a\n" + struct.pack(">I", 13) + header + struct.pack(
    ">I", zlib.crc32(header),
  )


class ImageServer(ThreadingHTTPServer):
  daemon_threads = True

  def __init__(self):
    super().__init__(("127.0.0.1", 0), ImageHandler)
    self.requests = Counter()
    self.failures = []
    self.lock = threading.Lock()


class ImageHandler(BaseHTTPRequestHandler):
  protocol_version = "HTTP/1.1"

  def log_message(self, *_arguments):
    pass

  def do_GET(self):
    with self.server.lock:
      self.server.requests[self.path] += 1
      for name in ("Authorization", "Cookie", "Referer", "Proxy-Authorization"):
        if self.headers.get(name):
          self.server.failures.append(f"Image request included {name}.")
      if self.headers.get("Content-Length") not in (None, "0"):
        self.server.failures.append("Image request included a body.")

    png = (FIXTURES / "image-quadrants.png").read_bytes()
    content = {
      "/color.png": png,
      "/color.jpg": (FIXTURES / "image-quadrants.jpg").read_bytes(),
      "/duplicate.png": png,
      "/invalid.png": b"not an image",
      "/truncated.png": png[:44],
      "/oversized.png": oversized_png(),
      "/too-large.png": b"x" * (4 * 1024 * 1024 + 1),
      "/slow.png": png,
    }.get(self.path, b"Unexpected image endpoint")
    status = 200
    if self.path == "/denied.png":
      status, content = 401, b"Secret server error must not appear in the UI"
    elif self.path == "/redirect.png":
      status, content = 302, b"Redirect refused"
    elif self.path not in {
      "/color.png", "/color.jpg", "/duplicate.png", "/invalid.png",
      "/truncated.png", "/oversized.png", "/too-large.png", "/slow.png",
    }:
      status = 404
      with self.server.lock:
        self.server.failures.append(f"Unexpected endpoint {self.path}.")
    self.send_response(status)
    self.send_header("Content-Type", "image/jpeg" if self.path.endswith(".jpg") else "image/png")
    self.send_header("Content-Length", str(len(content)))
    self.send_header("Connection", "close")
    self.close_connection = True
    if status == 302:
      self.send_header("Location", f"http://127.0.0.1:{self.server.server_port}/redirect-target.png")
    self.end_headers()
    try:
      if self.path == "/slow.png":
        for start in range(0, len(content), 4):
          self.wfile.write(content[start:start + 4])
          self.wfile.flush()
          time.sleep(0.02)
      else:
        self.wfile.write(content)
        self.wfile.flush()
    except (BrokenPipeError, ConnectionResetError):
      pass

  def do_POST(self):
    with self.server.lock:
      self.server.failures.append("Image loading issued a POST.")
    self.send_error(405)


def main():
  if len(sys.argv) not in (3, 4):
    raise SystemExit("Usage: mock_images.py IMAGE_TEST_EXECUTABLE RESOURCE_DIR [SCREENSHOT_DIR]")
  server = ImageServer()
  worker = threading.Thread(target=server.serve_forever, daemon=True)
  worker.start()
  try:
    with tempfile.TemporaryDirectory(prefix="wodex-images-") as directory:
      environment = dict(os.environ)
      environment.update({
        "SDL_VIDEODRIVER": "dummy",
        "WODEX_DATA_DIR": directory,
        "WODEX_FONT": "",
        "DEEPSEEK_API_KEY": "must-not-send-image-key",
      })
      command = [
        str(Path(sys.argv[1]).resolve()), f"http://127.0.0.1:{server.server_port}",
        str(Path(sys.argv[2]).resolve()),
      ]
      if len(sys.argv) == 4:
        output = Path(sys.argv[3]).resolve()
        output.mkdir(parents=True, exist_ok=True)
        command.append(str(output))
      result = subprocess.run(command, env=environment, timeout=45, check=False)
  finally:
    server.shutdown()
    server.server_close()
    worker.join(timeout=2)
  expected = Counter({
    "/color.png": 1, "/color.jpg": 1, "/duplicate.png": 1, "/denied.png": 1,
    "/redirect.png": 1, "/invalid.png": 1, "/truncated.png": 1,
    "/oversized.png": 1, "/too-large.png": 1, "/slow.png": 1,
  })
  if server.failures:
    print("Image HTTP validation failed: " + "; ".join(server.failures), file=sys.stderr)
    return 1
  if result.returncode == 0 and server.requests != expected:
    print(f"Unexpected image requests: {dict(server.requests)}", file=sys.stderr)
    return 1
  if result.returncode != 0:
    print(f"Image requests before failure: {dict(server.requests)}", file=sys.stderr)
  return result.returncode


if __name__ == "__main__":
  raise SystemExit(main())
