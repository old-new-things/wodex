#!/usr/bin/env python3
"""Render the saved Markdown fixtures through the production desktop app."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
  root = Path(__file__).resolve().parent.parent
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("executable", type=Path, help="Desktop wodex executable")
  parser.add_argument(
    "output", type=Path, nargs="?", default=root / "build/desktop/markdown-previews",
    help="Screenshot directory (default: build/desktop/markdown-previews)",
  )
  arguments = parser.parse_args()
  executable = arguments.executable.resolve(strict=True)
  output = arguments.output.resolve()
  output.mkdir(parents=True, exist_ok=True)
  fixtures = sorted((root / "tests/fixtures").glob("markdown-*.json"))
  if not fixtures:
    parser.error("No Markdown fixtures found.")

  for fixture in fixtures:
    state = json.loads(fixture.read_text(encoding="utf-8"))
    if state["settings"]["api_key"]:
      raise ValueError(f"{fixture.name} must not contain an API key.")
    with tempfile.TemporaryDirectory(prefix="wodex-markdown-") as temporary:
      Path(temporary, "state.json").write_text(
        json.dumps(state, ensure_ascii=False), encoding="utf-8",
      )
      environment = dict(os.environ)
      environment.update({
        "SDL_VIDEODRIVER": "dummy",
        "WODEX_DATA_DIR": temporary,
        "WODEX_FONT": "",
        "DEEPSEEK_API_KEY": "",
      })
      screenshot = output / (fixture.stem + ".bmp")
      subprocess.run(
        [str(executable), "--screenshot", str(screenshot)],
        env=environment, check=True, timeout=30,
      )
      if any(Path(temporary).glob("state.json.recovery-*")):
        raise ValueError(f"The application rejected {fixture.name}.")
      print(screenshot)


if __name__ == "__main__":
  main()
