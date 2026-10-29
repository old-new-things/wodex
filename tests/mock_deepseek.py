#!/usr/bin/env python3
"""Run native integration tests against an isolated, loopback-only API mock."""

import collections
import http.server
import json
import subprocess
import sys
import threading
import time


def event(payload, name=None):
  prefix = "event: " + name + "\r\n" if name else ""
  return (prefix + "data: " + json.dumps(payload, ensure_ascii=False) + "\r\n\r\n").encode("utf-8")


def chat_payload():
  return b": mock keepalive\r\n\r\n" + b"".join([
    event({"choices": [{"delta": {"role": "assistant"}}]}),
    event({"choices": [{"delta": {"reasoning_content": "思考中…"}}]}),
    event({"choices": [{"delta": {"content": "你好，"}}]}),
    event({"choices": [{"delta": {"content": "Switch! 🌿"}}]}),
    event({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
    event({"choices": [], "usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}}),
    b"data: [DONE]\r\n\r\n",
  ])


class MockServer(http.server.ThreadingHTTPServer):
  daemon_threads = True

  def __init__(self):
    super().__init__(("127.0.0.1", 0), MockHandler)
    self.failures = []
    self.requests = collections.Counter()
    self.lock = threading.Lock()


class MockHandler(http.server.BaseHTTPRequestHandler):
  protocol_version = "HTTP/1.1"

  def log_message(self, *_args):
    pass

  def reply(self, status, payload, content_type="text/event-stream", headers=None):
    self.send_response(status)
    self.send_header("Content-Type", content_type)
    self.send_header("Content-Length", str(len(payload)))
    self.send_header("Connection", "close")
    for name, value in (headers or {}).items():
      self.send_header(name, value)
    self.end_headers()
    self.wfile.write(payload)
    self.close_connection = True

  def fragmented_reply(self, payload):
    self.send_response(200)
    self.send_header("Content-Type", "text/event-stream")
    self.send_header("Content-Length", str(len(payload)))
    self.send_header("Connection", "close")
    self.end_headers()
    # Split UTF-8, JSON strings, SSE event names and delimiters at real HTTP callbacks.
    for offset in range(0, len(payload), 3):
      self.wfile.write(payload[offset:offset + 3])
      self.wfile.flush()
      time.sleep(0.0005)
    self.close_connection = True

  def named_provider(self, body):
    _, _, provider, protocol, *operation = self.path.split("/")
    assert self.headers.get("Content-Type") == "application/json", "Incorrect provider content type"
    assert self.headers.get("Accept") == "text/event-stream", "Incorrect provider stream accept header"
    assert body["stream"] is True, "Named provider streaming not enabled"
    if provider == "llama_cpp":
      assert self.headers.get("Authorization") is None, "Keyless llama.cpp received authorization"
      assert self.headers.get("x-api-key") is None, "Keyless llama.cpp received an API key"
    elif protocol == "messages":
      assert self.headers.get("Authorization") is None, "Messages received Bearer authentication"
      assert self.headers.get("x-api-key") == "provider-test-key-" + provider, "Incorrect Messages key"
      assert self.headers.get("anthropic-version") == "2023-06-01", "Missing Messages API version"
    else:
      assert self.headers.get("Authorization") == "Bearer provider-test-key-" + provider, "Incorrect named provider key"
      assert self.headers.get("x-api-key") is None, "Bearer provider also received x-api-key"
    prompt = "Respond clearly. 包含中文。"
    if protocol == "responses":
      assert provider == "opencode_go" and operation == ["responses"], "Incorrect Responses provider path"
      assert body["model"] == "gpt-4o", "Incorrect Responses model"
      assert body["instructions"] == prompt, "Responses instructions were not converted"
      assert body["input"][0]["content"][0]["text"] == "Hello, Switch!", "Incorrect Responses input"
      assert body["max_output_tokens"] == 37, "Responses token limit was not converted"
      assert "messages" not in body and "max_tokens" not in body, "Chat fields reached Responses"
      payload = b"".join([
        event({"type": "response.reasoning_summary_text.delta", "delta": "思考中…"}, "response.reasoning_summary_text.delta"),
        event({"type": "response.output_text.delta", "delta": "你好，Switch! 🌿"}, "response.output_text.delta"),
        event({"type": "response.completed", "response": {"usage": {"input_tokens": 5, "output_tokens": 10, "total_tokens": 15}}}, "response.completed"),
      ])
    elif protocol == "messages":
      assert provider == "opencode_go" and operation == ["messages"], "Incorrect Messages provider path"
      assert body["model"] == "minimax-m2.5", "Incorrect Messages model"
      assert body["system"] == prompt, "Messages system prompt was not converted"
      assert body["messages"][0]["content"][0]["text"] == "Hello, Switch!", "Incorrect Messages input"
      assert body["max_tokens"] == 37, "Messages token limit was not converted"
      assert "instructions" not in body and "input" not in body, "Responses fields reached Messages"
      payload = b"".join([
        event({"type": "message_start", "message": {"usage": {"input_tokens": 5, "output_tokens": 0}}}, "message_start"),
        event({"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "思考中…"}}, "content_block_delta"),
        event({"type": "content_block_delta", "delta": {"type": "text_delta", "text": "你好，Switch! 🌿"}}, "content_block_delta"),
        event({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 10}}, "message_delta"),
        event({"type": "message_stop"}, "message_stop"),
      ])
    else:
      assert protocol == "chat_completions" and operation == ["chat", "completions"], "Incorrect Chat provider path"
      assert body["model"] == ("mock-model" if provider in ("openai", "llama_cpp") else "glm-4.7"), "Incorrect named model"
      assert body["messages"][0] == {"role": "system", "content": prompt}, "Named system prompt changed"
      assert body["messages"][-1]["content"] == "Hello, Switch!", "Named history changed"
      token_field = "max_completion_tokens" if provider == "openai" else "max_tokens"
      assert body[token_field] == 37, "Incorrect named provider token limit"
      assert "stream_options" not in body and "reasoning_effort" not in body, "DeepSeek options reached named provider"
      if provider in ("zai", "zai_coding_plan", "zhipuai", "zhipuai_coding_plan"):
        assert body["thinking"] == {"type": "enabled", "clear_thinking": False}, "Tokn Z.ai body policy changed"
      else:
        assert "thinking" not in body, "Unexpected named provider thinking option"
      payload = chat_payload()
    with self.server.lock:
      self.server.requests[self.path] += 1
    self.fragmented_reply(payload)

  def do_POST(self):
    try:
      length = int(self.headers.get("Content-Length", "0"))
      assert 0 < length < 4 * 1024 * 1024, "Invalid mock request size"
      body = json.loads(self.rfile.read(length))
      if self.path.startswith("/providers/"):
        self.named_provider(body)
        return
      assert body["model"] == "mock-model", "Incorrect model in request"
      assert body["stream"] is True, "Streaming not enabled"
      compatible = self.path.startswith("/compatible/")
      if compatible:
        assert self.headers.get("Authorization") == "Bearer generic-local-test-key", "Wrong provider credential"
        token_field = self.path.split("/")[2]
        assert token_field in ("max_tokens", "max_completion_tokens"), "Unexpected limit field"
        assert body[token_field] == 4096, "Incorrect compatible token limit"
        assert set(body) == {"model", "messages", "stream", token_field}, "Unsupported compatible fields"
      else:
        assert self.headers.get("Authorization") == "Bearer mock-local-test-key", "Invalid mock authorization"
        assert body["stream_options"]["include_usage"] is True, "Usage not requested"
        assert body["thinking"]["type"] == "enabled", "Thinking not enabled"
      assert body["messages"][-1]["content"] == "Hello, Switch!", "Incorrect chat message"
      with self.server.lock:
        self.server.requests[self.path] += 1

      if self.path == "/normal/chat/completions" or compatible:
        self.fragmented_reply(chat_payload())
      elif self.path == "/unauthorized/chat/completions":
        self.reply(401, b'{"error":"private-error-body mock-local-test-key"}', "application/json")
      elif self.path == "/redirect/chat/completions":
        self.reply(302, b"private-error-body", headers={"Location": "/normal/chat/completions"})
      elif self.path == "/truncated/chat/completions":
        self.reply(200, event({"choices": [{"delta": {"content": "Partial answer"}}]}))
      elif self.path == "/cancel/chat/completions":
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(event({"choices": [{"delta": {"content": "Starting"}}]}))
        self.wfile.flush()
        for _ in range(100):
          time.sleep(0.05)
          self.wfile.write(b": keepalive\n\n")
          self.wfile.flush()
        self.close_connection = True
      elif self.path == "/oversized/chat/completions":
        self.reply(200, b"x" * (9 * 1024 * 1024))
      else:
        raise AssertionError("Unexpected mock API path")
    except (BrokenPipeError, ConnectionResetError):
      # Expected when the client cancels or enforces the queue bound.
      pass
    except Exception as error:
      with self.server.lock:
        self.server.failures.append(str(error))
      try:
        self.reply(400, b'{"error":"mock request validation failed"}', "application/json")
      except (BrokenPipeError, ConnectionResetError):
        pass


def main():
  if len(sys.argv) != 2:
    raise SystemExit("Usage: mock_deepseek.py /path/to/wodex_native_tests")
  server = MockServer()
  thread = threading.Thread(target=server.serve_forever, daemon=True)
  thread.start()
  try:
    base_url = "http://127.0.0.1:" + str(server.server_port)
    result = subprocess.run([sys.argv[1], base_url], timeout=45, check=False)
  finally:
    server.shutdown()
    server.server_close()
    thread.join(timeout=2)
  if server.failures:
    print("Mock API validation failed: " + "; ".join(server.failures), file=sys.stderr)
    return 1
  expected = {
    "/normal/chat/completions", "/unauthorized/chat/completions", "/redirect/chat/completions",
    "/truncated/chat/completions", "/cancel/chat/completions", "/oversized/chat/completions",
    "/compatible/max_tokens/chat/completions", "/compatible/max_completion_tokens/chat/completions",
  }
  expected.update(
    "/providers/" + provider + "/chat_completions/chat/completions"
    for provider in ("openai", "opencode_go", "llama_cpp", "zai", "zai_coding_plan", "zhipuai", "zhipuai_coding_plan")
  )
  expected.update({
    "/providers/opencode_go/responses/responses", "/providers/opencode_go/messages/messages",
  })
  if result.returncode == 0 and (set(server.requests) != expected or any(count != 1 for count in server.requests.values())):
    print("Unexpected mock request count; redirects or rejected requests may have escaped validation.", file=sys.stderr)
    return 1
  return result.returncode


if __name__ == "__main__":
  raise SystemExit(main())
