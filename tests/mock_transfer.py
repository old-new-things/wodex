#!/usr/bin/env python3
"""PC browser protocol against the real Rust app and native loopback sockets."""
import copy
import http.client
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse


def main():
  checks = 0

  def check(condition, message):
    nonlocal checks
    if not condition:
      raise AssertionError(message)
    checks += 1

  with tempfile.TemporaryDirectory(prefix="wodex-transfer-") as temporary:
    directory = Path(temporary)
    environment = dict(os.environ, SDL_VIDEODRIVER="dummy", WODEX_DATA_DIR=temporary,
      WODEX_FONT="", DEEPSEEK_API_KEY="environment-only-test-key",
      OPENAI_API_KEY="openai-environment-only-test-key")
    process = subprocess.Popen([sys.argv[1], sys.argv[2]], stdin=subprocess.PIPE,
      stdout=subprocess.PIPE, text=True, env=environment)

    def receive():
      if not select.select([process.stdout], [], [], 10)[0]:
        raise AssertionError("Application control timed out.")
      line = process.stdout.readline()
      if not line:
        raise AssertionError(f"Application ended early: {process.poll()}")
      return json.loads(line)

    def command(value):
      process.stdin.write(value + "\n")
      process.stdin.flush()
      return receive()

    def saved():
      return json.loads((directory / "state.json").read_text())

    try:
      check(receive() == {"ready": True}, "Application did not initialize.")
      original = saved()
      listener = command("start")
      address = urllib.parse.urlsplit(listener["url"])
      code = listener["code"]
      check(len(code) == 32, "Pairing code has wrong entropy length.")

      def request(path, method="GET", body=None, authenticated=True, headers=None, expected=200):
        connection = http.client.HTTPConnection(address.hostname, address.port, timeout=5)
        values = {"Authorization": "Bearer " + code} if authenticated else {}
        if body is not None:
          values["Content-Type"] = "application/json"
        values.update(headers or {})
        connection.request(method, path, body=body, headers=values)
        response = connection.getresponse()
        data = response.read()
        check(response.status == expected, f"{method} {path}: {response.status}, expected {expected}")
        check(response.getheader("Cache-Control") == "no-store", "Transfer response can be cached.")
        check(response.getheader("Access-Control-Allow-Origin") is None, "Transfer enabled CORS.")
        check(response.getheader("X-Content-Type-Options") == "nosniff", "Missing nosniff.")
        connection.close()
        return data

      page = request("/", authenticated=False)
      check(b"Edit on your PC" in page, "Missing local browser configuration editor.")
      check(b'id="config-api-key" name="api_key" type="password"' in page,
        "Saved-key editor is not masked by default.")
      check(b'id="config-model"' in page and b'id="config-system-prompt"' in page,
        "Missing model or assistant-instructions input.")
      check(b'id="config-provider" name="provider"' in page
        and b'<option value="deepseek">' in page and b'<option value="openai_compatible">' in page,
        "Missing DeepSeek/OpenAI-compatible provider selector.")
      check(b'id="config-base-url" name="base_url"' in page,
        "Missing provider endpoint input.")
      check(b'id="config-token-limit-field" name="token_limit_field"' in page
        and b'<option value="max_tokens">' in page and b'<option value="max_completion_tokens">' in page,
        "Missing compatible-provider token limit selector.")
      check(code.encode() not in page and b"saved-transfer-test-key" not in page, "Public page leaks credentials.")
      request("/transfer.js", authenticated=False)
      request("/transfer.css", authenticated=False)
      for endpoint in ("/api/config", "/api/sessions", "/api/status"):
        request(endpoint, authenticated=False, expected=401)
      request("/api/config", headers={"Authorization": "Bearer " + "0" * 32}, expected=401)
      request("/api/config", headers={"Origin": "https://other.invalid"}, expected=403)
      request("/api/config", headers={"Sec-Fetch-Site": "cross-site"}, expected=403)
      request("/api/config", headers={"Host": "other.invalid"}, expected=403)
      request("/state.json", expected=404)
      request("/../state.json", expected=404)
      request("/api/config", "POST", "{}", headers={"Content-Type": "text/plain"}, expected=415)
      request("/api/config", "POST", None, headers={"Content-Length": str(4 * 1024 * 1024 + 1)}, expected=413)
      request("/api/config", "POST", None, headers={"Content-Length": "10", "Transfer-Encoding": "chunked"}, expected=400)

      config = json.loads(request("/api/config"))
      sessions = json.loads(request("/api/sessions"))
      check(config["channel"] == "config" and config["schema_version"] == 1, "Wrong config schema.")
      check(config["settings"]["api_key"] == "saved-transfer-test-key", "Config does not include the saved key.")
      check("environment-only-test-key" not in json.dumps(config), "Export leaked the environment override.")
      check("openai-environment-only-test-key" not in json.dumps(config),
        "DeepSeek export leaked the OpenAI environment override.")
      check("settings" not in sessions and "conversations" not in config, "Channels were mixed.")
      check(sessions["conversations"] == original["conversations"], "Sessions differ on export.")

      changed = copy.deepcopy(config)
      changed["settings"].update(api_key="imported-config-test-key", model="pc-import-model",
        system_prompt="PC 配置", base_url="https://example.invalid/v1")
      request("/api/config", "POST", json.dumps(changed).encode(), expected=202)
      check(json.loads(request("/api/status"))["pending"], "Import did not become pending.")
      check(saved() == original, "Upload changed data before confirmation.")
      request("/api/sessions", "POST", json.dumps(sessions).encode(), expected=409)
      command("frame")
      command("reject")
      check(not json.loads(request("/api/status"))["pending"], "Rejected import stayed pending.")
      check(saved() == original, "Rejected import changed data.")

      request("/api/config", "POST", json.dumps(changed, ensure_ascii=False).encode(), expected=202)
      command("frame")
      command("confirm")
      after_config = saved()
      check(after_config["settings"] == changed["settings"], "Confirmed configuration was not saved.")
      check(after_config["conversations"] == original["conversations"], "Config import changed sessions.")
      check(json.loads(request("/api/config")) == changed, "Exports did not refresh after config import.")
      check(any(p.name.startswith("state.json") and p.name != "state.json" for p in directory.iterdir()),
        "Import did not preserve a recovery copy.")

      compatible = copy.deepcopy(changed)
      compatible["settings"].update(provider="openai_compatible", token_limit_field="max_completion_tokens",
        api_key="saved-compatible-test-key", base_url="https://compatible.invalid/v1",
        model="compatible-test-model", thinking=False)
      request("/api/config", "POST", json.dumps(compatible, ensure_ascii=False).encode(), expected=202)
      check(json.loads(request("/api/status"))["pending"], "Compatible-provider import did not become pending.")
      check(saved() == after_config, "Compatible-provider upload changed data before confirmation.")
      command("frame")
      command("confirm")
      after_config = saved()
      check(after_config["settings"] == compatible["settings"],
        "Confirmed compatible-provider settings were not saved.")
      check(after_config["conversations"] == original["conversations"],
        "Compatible-provider import changed conversations.")
      check(json.loads(request("/api/sessions")) == sessions,
        "Compatible-provider import changed exported sessions or their selection.")
      compatible_export = json.loads(request("/api/config"))
      check(compatible_export == compatible, "Compatible provider and token field did not round-trip through HTTP.")
      check(compatible_export["settings"]["api_key"] == "saved-compatible-test-key",
        "Compatible-provider export does not include its saved key.")
      check("environment-only-test-key" not in json.dumps(compatible_export),
        "Compatible-provider export leaked a provider environment override.")
      check("openai-environment-only-test-key" not in json.dumps(after_config),
        "Confirming compatible-provider settings persisted the environment override.")

      updated_sessions = copy.deepcopy(sessions)
      updated_sessions["active_id"] = "chat-7"
      updated_sessions["conversations"] = [{
        "id": "chat-7", "title": "Imported 中文 session", "messages": [
          {"role": "user", "content": "你好", "reasoning_content": "", "status": "complete"},
          {"role": "assistant", "content": "**Welcome**", "reasoning_content": "思考",
           "status": "streaming"},
        ]
      }]
      request("/api/sessions", "POST", json.dumps(updated_sessions, ensure_ascii=False).encode(), expected=202)
      command("frame")
      command("confirm")
      after_sessions = saved()
      check(after_sessions["settings"] == after_config["settings"], "Session import changed config.")
      check(after_sessions["active_id"] == "chat-7", "Imported active session was lost.")
      check(after_sessions["conversations"][0]["messages"][1]["status"] == "interrupted", "Imported stream stayed live.")
      check(after_sessions["next_id"] > 7, "Next conversation ID can collide.")
      check(json.loads(request("/api/sessions"))["active_id"] == "chat-7", "Session export was stale.")
      request("/api/config", "POST", json.dumps(updated_sessions).encode(), expected=202)
      command("frame")
      check(not json.loads(request("/api/status"))["pending"], "Wrong-channel upload was not rejected.")
      check(saved() == after_sessions, "Invalid import changed saved data.")

      # Closing a transfer must interrupt a client that never sends its body.
      stalled = socket.create_connection((address.hostname, address.port), timeout=2)
      stalled.sendall((f"POST /api/config HTTP/1.1\r\nHost: {address.netloc}\r\n"
        f"Authorization: Bearer {code}\r\nContent-Type: application/json\r\n"
        "Content-Length: 1000\r\n\r\n{").encode())
      started = time.monotonic()
      command("stop")
      check(time.monotonic() - started < 2, "Stopping waited for the stalled upload.")
      stalled.close()
      try:
        socket.create_connection((address.hostname, address.port), timeout=1).close()
      except OSError:
        checks += 1
      else:
        raise AssertionError("Stopped server still accepts connections.")
      restarted = command("start")
      check(restarted["code"] != code, "Restart reused its pairing code.")
      address = urllib.parse.urlsplit(restarted["url"])
      request("/api/config", expected=401)  # Old code cannot access the new listener.
      code = restarted["code"]
      request("/api/status")
      command("quit")
      check(process.wait(timeout=3) == 0, "Application did not shut down cleanly.")
      check(saved() == after_sessions, "Closing persisted an environment key or changed imported data.")
    finally:
      if process.poll() is None:
        process.kill()
        process.wait(timeout=3)
  print(f"PC transfer integration: {checks} checks passed.")


if __name__ == "__main__":
  main()
