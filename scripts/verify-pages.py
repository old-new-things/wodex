#!/usr/bin/env python3
"""Verify that public GitHub Pages serves the exact generated download site."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import queue
import re
import ssl
import stat
import sys
import threading
import time
from typing import NamedTuple
import urllib.request

CHUNK_BYTES = 64 * 1024
MAX_NRO_BYTES = 128 * 1024 * 1024
METADATA_LIMITS = {
  "catalog.json": 4 * 1024 * 1024,
  "updates/stable.json": 1024 * 1024,
  "index.html": 1024 * 1024,
}


class VerificationError(RuntimeError):
  pass


class PublicationTimeout(VerificationError):
  pass


class ExpectedFile(NamedTuple):
  path: str
  size: int
  sha256: str


class NoRedirect(urllib.request.HTTPRedirectHandler):
  def redirect_request(self, request, response, code, message, headers, new_url):
    raise VerificationError("The published site redirected a verification request.")


def public_opener():
  # No proxy credentials, cookie jar, or authentication handlers are installed.
  return urllib.request.build_opener(
    urllib.request.ProxyHandler({}),
    urllib.request.HTTPSHandler(context=ssl.create_default_context()),
    NoRedirect(),
  ).open


def validate_site_url(site_url):
  match = re.fullmatch(
    r"https://([a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?)\.github\.io/([A-Za-z0-9_.-]{1,100})",
    site_url,
  )
  if not match or "--" in match[1] or ".." in match[2] or match[2] == ".":
    raise VerificationError("The site URL must be a direct HTTPS project GitHub Pages URL.")


def local_file(site_dir, relative, limit):
  path = site_dir
  for part in relative.split("/"):
    path = path / part
    if path.is_symlink():
      raise VerificationError("The generated site contains a symbolic link.")
  try:
    info = path.stat()
    if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= limit:
      raise VerificationError(f"The generated {relative} has an invalid size or file type.")
    digest = hashlib.sha256()
    received = 0
    with path.open("rb") as source:
      while chunk := source.read(CHUNK_BYTES):
        received += len(chunk)
        if received > limit:
          raise VerificationError(f"The generated {relative} exceeds its size limit.")
        digest.update(chunk)
    if received != info.st_size:
      raise VerificationError(f"The generated {relative} changed during verification.")
    return ExpectedFile(relative, received, digest.hexdigest())
  except OSError as error:
    raise VerificationError(f"Cannot read generated file {relative}.") from error


def verification_plan(site_dir):
  site_dir = Path(site_dir)
  if site_dir.is_symlink() or not site_dir.is_dir():
    raise VerificationError("The generated site directory is missing or unsafe.")
  metadata = [
    local_file(site_dir, name, limit)
    for name, limit in METADATA_LIMITS.items()
  ]
  try:
    with (site_dir / "catalog.json").open("rb") as source:
      catalog_bytes = source.read(METADATA_LIMITS["catalog.json"] + 1)
    if len(catalog_bytes) > METADATA_LIMITS["catalog.json"]:
      raise VerificationError("The generated build catalog exceeds its size limit.")
    catalog = json.loads(catalog_bytes)
  except (OSError, ValueError) as error:
    raise VerificationError("The generated build catalog is invalid.") from error
  if (
    not isinstance(catalog, dict) or type(catalog.get("schema_version")) is not int
    or catalog["schema_version"] != 1
    or not isinstance(catalog.get("builds"), list) or not 1 <= len(catalog["builds"]) <= 17
  ):
    raise VerificationError("The generated build catalog has an unsupported schema.")
  selected = set()
  downloads = []
  # The publisher orders each channel newest first (semantic order for versions).
  for build in catalog["builds"]:
    if (
      not isinstance(build, dict) or not isinstance(build.get("channel"), str)
      or build["channel"] not in {"nightly", "version"}
    ):
      raise VerificationError("The generated catalog contains an invalid build channel.")
    channel = build["channel"]
    if channel in selected:
      continue
    selected.add(channel)
    identity = build.get("id")
    if (
      not isinstance(identity, str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,100}", identity)
      or ".." in identity or identity == "."
    ):
      raise VerificationError("The generated catalog contains an unsafe build path.")
    assets = build.get("assets")
    if not isinstance(assets, dict):
      raise VerificationError("The generated catalog is missing asset integrity metadata.")
    for name, limit in [("wodex.nro.sha256", 256), ("wodex.nro", MAX_NRO_BYTES)]:
      asset = assets.get(name)
      if (
        not isinstance(asset, dict) or type(asset.get("size")) is not int
        or not 0 < asset["size"] <= limit or not isinstance(asset.get("sha256"), str)
        or not re.fullmatch(r"[0-9a-f]{64}", asset["sha256"])
      ):
        raise VerificationError("The generated catalog contains invalid asset integrity metadata.")
      expected = local_file(site_dir, f"builds/{channel}/{identity}/{name}", limit)
      if (expected.size, expected.sha256) != (asset["size"], asset["sha256"]):
        raise VerificationError(f"The generated {expected.path} does not match its catalog.")
      downloads.append(expected)
  return metadata, downloads


def remaining(deadline, now):
  available = deadline - now
  if available <= 0:
    raise PublicationTimeout("Public Pages verification exceeded its time budget.")
  return available


def verify_file(site_url, expected, nonce, opener, deadline, clock, request_timeout):
  timeout = min(request_timeout, remaining(deadline, clock()))
  request_deadline = min(deadline, clock() + timeout)
  cancelled = threading.Event()
  result = queue.Queue(maxsize=1)
  url = f"{site_url}/{expected.path}?publication_check={nonce}"
  request = urllib.request.Request(url, headers={
    "User-Agent": "Wodexpublication-check",
    "Accept-Encoding": "identity",
    "Cache-Control": "no-cache",
    "Pragma": "no-cache",
  })

  def check_request_time():
    if cancelled.is_set() or clock() >= request_deadline:
      raise VerificationError(f"Fetching {expected.path} exceeded the request timeout.")

  def fetch():
    try:
      check_request_time()
      with opener(request, timeout=timeout) as response:
        if response.geturl() != url:
          raise VerificationError("The published site redirected a verification request.")
        if response.status != 200:
          raise VerificationError(f"The published {expected.path} did not return HTTP 200.")
        if response.headers.get("Content-Encoding", "identity") != "identity":
          raise VerificationError(f"The published {expected.path} used unexpected content encoding.")
        length = response.headers.get("Content-Length")
        if length is not None and (not length.isdecimal() or int(length) != expected.size):
          raise VerificationError(f"The published {expected.path} has a different size.")
        digest = hashlib.sha256()
        received = 0
        while True:
          check_request_time()
          chunk = response.read(min(CHUNK_BYTES, expected.size - received + 1))
          check_request_time()
          if not chunk:
            break
          received += len(chunk)
          if received > expected.size:
            raise VerificationError(f"The published {expected.path} is too large.")
          digest.update(chunk)
        if received != expected.size or digest.hexdigest() != expected.sha256:
          raise VerificationError(f"The published {expected.path} does not match the generated file.")
    except Exception as error:
      result.put(error)
    else:
      result.put(None)

  # Bound the whole request, including DNS and a slow streaming body, instead of
  # allowing the socket's inactivity timeout to restart indefinitely per read.
  worker = threading.Thread(target=fetch, daemon=True)
  worker.start()
  try:
    outcome = result.get(timeout=max(0, request_deadline - clock()))
  except queue.Empty:
    cancelled.set()
    remaining(deadline, clock())
    raise VerificationError(f"Fetching {expected.path} exceeded the request timeout.") from None
  remaining(deadline, clock())
  if isinstance(outcome, VerificationError):
    raise outcome
  if outcome is not None:
    raise VerificationError(f"Cannot fetch published file {expected.path}.") from outcome


def verify_site(
  site_url, site_dir, *, opener=None, clock=time.monotonic, sleep=time.sleep,
  budget_seconds=660, retry_seconds=25, request_timeout=20, report=print,
):
  validate_site_url(site_url)
  for value in (budget_seconds, retry_seconds, request_timeout):
    if not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
      raise VerificationError("Verification time limits must be finite positive values.")
  deadline = clock() + budget_seconds
  metadata, downloads = verification_plan(site_dir)
  opener = public_opener() if opener is None else opener
  attempt = 0
  while True:
    remaining(deadline, clock())
    attempt += 1
    nonce = f"{attempt}-{int(clock() * 1000)}"
    try:
      # Do not download any payload while the CDN still serves old metadata.
      for expected in metadata + downloads:
        verify_file(site_url, expected, nonce, opener, deadline, clock, request_timeout)
      report(f"Verified the public page, catalog, version feed, and {len(downloads) // 2} channel downloads.")
      return
    except PublicationTimeout:
      raise
    except VerificationError as error:
      available = remaining(deadline, clock())
      report(f"Pages content is not current yet (attempt {attempt}): {error}")
      sleep(min(retry_seconds, available))


def main():
  parser = argparse.ArgumentParser(description=__doc__)
  parser.add_argument("--site-url", required=True)
  parser.add_argument("--site-dir", required=True, type=Path)
  args = parser.parse_args()
  try:
    verify_site(args.site_url, args.site_dir)
  except VerificationError as error:
    print(f"Pages verification failed: {error}", file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
