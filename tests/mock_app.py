#!/usr/bin/env python3
"""Exercise Rust application input/rendering against the real native transport."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

from mock_deepseek import MockServer


def main():
  if len(sys.argv) != 3:
    raise SystemExit("Usage: mock_app.py /path/to/wodex_app_tests RESOURCE_DIR")
  server = MockServer()
  thread = threading.Thread(target=server.serve_forever, daemon=True)
  thread.start()
  try:
    with tempfile.TemporaryDirectory(prefix="wodex-app-") as directory:
      environment = dict(os.environ)
      environment.update({
        "SDL_VIDEODRIVER": "dummy",
        "WODEX_DATA_DIR": directory,
        "WODEX_FONT": "",
        "DEEPSEEK_API_KEY": "mock-local-test-key",
      })
      base_url = "http://127.0.0.1:" + str(server.server_port)
      result = subprocess.run(
        [str(Path(sys.argv[1]).resolve()), base_url, str(Path(sys.argv[2]).resolve())],
        env=environment, timeout=35, check=False,
      )
  finally:
    server.shutdown()
    server.server_close()
    thread.join(timeout=2)
  if server.failures:
    print("Mock app API validation failed: " + "; ".join(server.failures), file=sys.stderr)
    return 1
  if result.returncode == 0 and dict(server.requests) != {"/normal/chat/completions": 2}:
    print("Application sent an unexpected request count or endpoint.", file=sys.stderr)
    return 1
  return result.returncode


if __name__ == "__main__":
  raise SystemExit(main())
