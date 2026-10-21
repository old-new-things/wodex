import assert from "node:assert/strict";
import { test } from "node:test";
import { CONFIG, Harness, PAIR_CODE, pair, status } from "./harness";

function posts(app: Harness): number {
  return app.calls.filter(call => call.method === "POST").length;
}
function configReads(app: Harness): number {
  return app.calls.filter(call => call.method === "GET" && call.path === "/api/config").length;
}
function notices(app: Harness): string {
  return app.element("status").textContent + " " + app.element("config-status").textContent;
}
async function sendForm(app: Harness): Promise<unknown> {
  const submitted = app.element("config-form").dispatch("submit");
  const upload = await app.nextCall("/api/config", "POST");
  assert.equal(upload.headers.get("Authorization"), "Bearer " + PAIR_CODE);
  assert.equal(upload.headers.get("Content-Type"), "application/json");
  const document: unknown = JSON.parse(upload.body!);
  upload.reply(status(true, "Confirm on your Switch."), 202);
  await submitted;
  await app.flush();
  return document;
}
async function pollStatus(app: Harness, pending: boolean, message = "Ready"): Promise<void> {
  await app.runPoll();
  (await app.nextCall("/api/status")).reply(status(pending, message));
  await app.flush();
}

test("pairing loads saved configuration into the actual HTML form without posting", async () => {
  const app = new Harness();
  await pair(app);
  assert.equal(app.element("pair-form").hidden, true);
  assert.equal(app.element("config-api-key").value, CONFIG.settings.api_key);
  assert.equal(app.element("config-api-key").type, "password");
  assert.equal(app.element("config-model").value, CONFIG.settings.model);
  assert.equal(app.element("config-system-prompt").value, CONFIG.settings.system_prompt);
  assert.equal(app.element("config-base-url").value, CONFIG.settings.base_url);
  assert.equal(app.element("config-thinking").checked, true);
  assert.equal(app.element("config-provider").value, "deepseek");
  assert.equal(app.element("config-token-limit-field").value, "max_tokens");
  assert.equal(app.element("config-thinking-field").hidden, false);
  assert.equal(app.element("config-token-limit-choice").hidden, true);
  assert.equal(app.element("config-max-tokens").value, String(CONFIG.settings.max_tokens));
  assert.equal(app.element("config-update-repository").value, CONFIG.settings.update_repository);
  assert.equal(app.element("config-update-channel").value, CONFIG.settings.update_channel);
  assert.equal(app.element("config-fields").disabled, false);
  assert.equal(posts(app), 0);
  assert.equal(configReads(app), 1);
  assert.equal(app.element("config-save").disabled, true);
  assert.ok(!notices(app).includes(CONFIG.settings.api_key));
});

test("changing provider clears the draft credential and resets only provider-specific settings", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("show-api-key", true);
  await app.edit("config-system-prompt", "Keep these instructions.");
  app.element("config-provider").value = "openai_compatible";
  await app.element("config-provider").dispatch("input");
  await app.element("config-provider").dispatch("change");
  await app.flush();
  assert.equal(posts(app), 0);
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-api-key").type, "password");
  assert.equal(app.element("show-api-key").checked, false);
  assert.equal(app.element("config-base-url").value, "https://api.openai.com/v1");
  assert.equal(app.element("config-model").value, "");
  assert.equal(app.element("config-thinking").checked, false);
  assert.equal(app.element("config-thinking").disabled, true);
  assert.equal(app.element("config-thinking-field").hidden, true);
  assert.equal(app.element("config-token-limit-choice").hidden, false);
  assert.equal(app.element("config-token-limit-field").disabled, false);
  assert.equal(app.element("config-system-prompt").value, "Keep these instructions.");
  assert.equal(app.element("config-max-tokens").value, String(CONFIG.settings.max_tokens));
  assert.match(app.element("config-status").textContent, /key was cleared/i);
  const document = await sendForm(app) as { settings: Record<string, unknown> };
  assert.equal(document.settings.provider, "openai_compatible");
  assert.equal(document.settings.api_key, "");
  assert.equal(document.settings.model, "");
  assert.equal(document.settings.thinking, false);
  assert.ok(!("token_limit_field" in document.settings));
  assert.equal(app.element("config-fields").disabled, true, "Changing a provider still needs Switch confirmation.");
});

test("a saved compatible provider loads its credential, endpoint and token field without being reset", async () => {
  const app = new Harness();
  const saved = {
    ...CONFIG,
    settings: {
      ...CONFIG.settings,
      provider: "openai_compatible",
      token_limit_field: "max_completion_tokens",
      api_key: "compatible-saved-key",
      base_url: "https://provider.invalid/api/v1",
      model: "provider-model",
      thinking: false
    }
  };
  await pair(app, saved);
  assert.equal(app.element("config-provider").value, "openai_compatible");
  assert.equal(app.element("config-api-key").value, saved.settings.api_key);
  assert.equal(app.element("config-api-key").type, "password");
  assert.equal(app.element("config-base-url").value, saved.settings.base_url);
  assert.equal(app.element("config-model").value, saved.settings.model);
  assert.equal(app.element("config-token-limit-field").value, "max_completion_tokens");
  assert.equal(app.element("config-thinking-field").hidden, true);
  assert.equal(app.element("config-save").disabled, true);
  // Duplicate input/change events for one selection must not clear credentials.
  await app.edit("config-provider", "openai_compatible");
  assert.equal(app.element("config-api-key").value, saved.settings.api_key);
  await app.edit("config-max-tokens", "2048");
  assert.deepEqual(await sendForm(app), {
    ...saved,
    settings: { ...saved.settings, max_tokens: 2048 }
  });
});

test("switching back to DeepSeek clears the compatible key and emits a legacy-compatible document", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-provider", "openai_compatible");
  await app.edit("config-api-key", "compatible-draft-key");
  await app.edit("config-model", "compatible-model");
  await app.edit("config-token-limit-field", "max_completion_tokens");
  await app.edit("config-provider", "deepseek");
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-base-url").value, "https://api.deepseek.com");
  assert.equal(app.element("config-model").value, "deepseek-flash");
  assert.equal(app.element("config-token-limit-field").value, "max_tokens");
  assert.equal(app.element("config-token-limit-field").disabled, true);
  assert.equal(app.element("config-thinking").disabled, false);
  const document = await sendForm(app) as { settings: Record<string, unknown> };
  assert.ok(!("provider" in document.settings));
  assert.ok(!("token_limit_field" in document.settings));
  assert.equal(document.settings.api_key, "");
  assert.equal(document.settings.thinking, false);
});

test("compatible provider edits preserve custom endpoint, model and the selected token parameter", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-provider", "openai_compatible");
  await app.edit("config-api-key", "new-compatible-key");
  await app.edit("config-base-url", "https://compatible.invalid/custom/v1/");
  await app.edit("config-model", "provider/model-name");
  await app.edit("config-token-limit-field", "max_completion_tokens");
  const document = await sendForm(app) as { settings: Record<string, unknown> };
  assert.equal(document.settings.provider, "openai_compatible");
  assert.equal(document.settings.token_limit_field, "max_completion_tokens");
  assert.equal(document.settings.api_key, "new-compatible-key");
  assert.equal(document.settings.base_url, "https://compatible.invalid/custom/v1/");
  assert.equal(document.settings.model, "provider/model-name");
  assert.equal(document.settings.thinking, false);
});

test("unknown provider options and incompatible settings fail closed without exposing credentials", async () => {
  for (const patch of [
    { provider: "unknown" },
    { provider: null },
    { token_limit_field: "unknown" },
    { token_limit_field: null },
    { token_limit_field: "max_completion_tokens" },
    { provider: "openai_compatible", thinking: true }
  ]) {
    const app = new Harness();
    await pair(app, { ...CONFIG, settings: { ...CONFIG.settings, ...patch } });
    assert.equal(app.element("config-fields").disabled, true);
    assert.equal(app.element("config-api-key").value, "");
    assert.equal(app.element("config-save").disabled, true);
    assert.equal(posts(app), 0);
    assert.ok(!notices(app).includes(CONFIG.settings.api_key));
  }
});

test("equivalent explicit defaults do not invalidate a dirty draft after a sessions import", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-provider", "openai_compatible");
  await app.edit("config-model", "unsent-compatible-model");
  await pollStatus(app, true, "Confirm a sessions import.");
  await pollStatus(app, false, "Sessions imported.");
  (await app.nextCall("/api/config")).reply({
    ...CONFIG,
    settings: { ...CONFIG.settings, provider: "deepseek", token_limit_field: "max_tokens" }
  });
  await app.flush();
  assert.equal(app.element("config-provider").value, "openai_compatible");
  assert.equal(app.element("config-model").value, "unsent-compatible-model");
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-fields").disabled, false);
  assert.equal(app.element("config-save").disabled, false);
  assert.equal(posts(app), 0);
});

test("typing creates only a draft; submit sends the complete config without sessions", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-api-key", "replacement-test-key");
  await app.edit("config-model", "pc-editor-model");
  await app.edit("config-system-prompt", "Edited 中文 prompt\nKeep formatting.");
  assert.equal(posts(app), 0);
  assert.equal(app.element("config-save").disabled, false);
  const document = await sendForm(app);
  assert.deepEqual(document, {
    ...CONFIG,
    settings: {
      ...CONFIG.settings,
      api_key: "replacement-test-key",
      model: "pc-editor-model",
      system_prompt: "Edited 中文 prompt\nKeep formatting."
    }
  });
  assert.equal(posts(app), 1);
  assert.equal(app.element("config-fields").disabled, true);
  assert.equal(app.element("config-save").disabled, true);
  assert.ok(!notices(app).includes("replacement-test-key"));
});

for (const confirmed of [true, false]) {
  test("local form " + (confirmed ? "confirmation" : "rejection") + " reloads the saved configuration", async () => {
    const app = new Harness();
    await pair(app);
    await app.edit("config-model", "draft-model");
    await sendForm(app);
    const saved = {
      ...CONFIG,
      settings: { ...CONFIG.settings, model: confirmed ? "draft-model" : CONFIG.settings.model }
    };
    const message = confirmed ? "Configuration imported on console." : "Import rejected on console.";
    await pollStatus(app, false, message);
    (await app.nextCall("/api/config")).reply(saved);
    await app.flush();
    assert.equal(app.element("config-model").value, saved.settings.model);
    assert.equal(app.element("config-fields").disabled, false);
    assert.equal(app.element("config-save").disabled, true);
    assert.equal(posts(app), 1);
    assert.ok(notices(app).includes(message));
  });
}

test("explicit reload asks before discarding a dirty draft", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-system-prompt", "Unsaved draft");
  app.confirmationAnswers.push(false);
  await app.element("config-reload").dispatch("click");
  await app.flush();
  assert.equal(configReads(app), 1);
  assert.equal(app.element("config-system-prompt").value, "Unsaved draft");
  assert.equal(app.confirmations.length, 1);
  app.confirmationAnswers.push(true);
  const reload = app.element("config-reload").dispatch("click");
  (await app.nextCall("/api/config")).reply(CONFIG);
  await reload;
  await app.flush();
  assert.equal(app.element("config-system-prompt").value, CONFIG.settings.system_prompt);
  assert.equal(app.element("config-save").disabled, true);
  assert.equal(posts(app), 0);
});

test("an import from another client refreshes a clean form", async () => {
  const app = new Harness();
  await pair(app);
  await pollStatus(app, true, "Confirm another import.");
  await pollStatus(app, false, "Another import completed.");
  const updated = { ...CONFIG, settings: { ...CONFIG.settings, model: "another-client-model" } };
  (await app.nextCall("/api/config")).reply(updated);
  await app.flush();
  assert.equal(app.element("config-model").value, "another-client-model");
  assert.equal(app.element("config-save").disabled, true);
});

test("an external import preserves a dirty draft and requires reload before sending", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-model", "unsent-local-draft");
  await pollStatus(app, true, "Confirm another import.");
  await pollStatus(app, false, "Another import completed.");
  (await app.nextCall("/api/config")).reply({
    ...CONFIG, settings: { ...CONFIG.settings, model: "saved-by-another-client" }
  });
  await app.flush();
  assert.equal(app.element("config-model").value, "unsent-local-draft");
  assert.equal(configReads(app), 2);
  assert.equal(app.element("config-save").disabled, true);
  assert.match(notices(app), /reload/i);
  assert.equal(posts(app), 0);
});

test("a session import or rejected config import does not invalidate an unsent config draft", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-model", "unsent-local-draft");
  await pollStatus(app, true, "Confirm a sessions import.");
  await pollStatus(app, false, "Sessions imported.");
  (await app.nextCall("/api/config")).reply(CONFIG);
  await app.flush();
  assert.equal(app.element("config-model").value, "unsent-local-draft");
  assert.equal(app.element("config-fields").disabled, false);
  assert.equal(app.element("config-save").disabled, false);
  assert.equal(posts(app), 0);
});

test("invalid token limits never leave the browser", async () => {
  const app = new Harness();
  await pair(app);
  for (const value of ["0", "131073", "1.5", ""]) {
    await app.edit("config-max-tokens", value);
    await app.element("config-form").dispatch("submit");
    await app.flush();
    assert.equal(posts(app), 0, "Invalid max_tokens was sent: " + value);
  }
  await app.edit("config-max-tokens", "131072");
  const document = await sendForm(app) as typeof CONFIG;
  assert.equal(document.settings.max_tokens, 131072);
});

test("an initial config load failure keeps transfer connected and allows retry", async () => {
  const app = new Harness();
  app.element("pair-code").value = PAIR_CODE;
  const submitted = app.element("pair-form").dispatch("submit");
  (await app.nextCall("/api/status")).reply(status());
  (await app.nextCall("/api/config")).reply({ error: "failed" }, 500);
  await submitted;
  await app.flush();
  assert.equal(app.element("channels").hidden, false);
  assert.equal(app.element("config-fields").disabled, true);
  assert.equal(app.element("config-reload").disabled, false);
  const reload = app.element("config-reload").dispatch("click");
  (await app.nextCall("/api/config")).reply(CONFIG);
  await reload;
  await app.flush();
  assert.equal(app.element("config-model").value, CONFIG.settings.model);
  assert.equal(app.element("config-fields").disabled, false);
});

test("failed post-confirmation refresh cannot claim the draft is saved or send it again", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-model", "draft-awaiting-result");
  await sendForm(app);
  await pollStatus(app, false, "Configuration imported.");
  (await app.nextCall("/api/config")).reply({ error: "failed" }, 500);
  await app.flush();
  assert.equal(app.element("config-fields").disabled, true);
  assert.equal(app.element("config-save").disabled, true);
  assert.equal(app.element("config-reload").disabled, false);
  assert.match(notices(app), /reload|load|refresh/i);
  assert.equal(posts(app), 1);
});

test("disconnect clears revealed keys and ignores a late initial config response", async () => {
  const app = new Harness();
  app.element("pair-code").value = PAIR_CODE;
  const submitted = app.element("pair-form").dispatch("submit");
  (await app.nextCall("/api/status")).reply(status());
  const load = await app.nextCall("/api/config");
  await app.element("disconnect").dispatch("click");
  assert.equal(load.signal.aborted, true);
  load.reply(CONFIG); // Simulate a response already in flight despite AbortSignal.
  await submitted;
  await app.flush();
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-api-key").type, "password");
  assert.equal(app.element("pair-form").hidden, false);
  assert.equal(app.element("channels").hidden, true);
  assert.equal(posts(app), 0);

  await pair(app);
  await app.edit("show-api-key", true);
  assert.equal(app.element("config-api-key").type, "text");
  await app.pagehide();
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-api-key").type, "password");
  assert.equal(app.element("show-api-key").checked, false);
});

test("a stale status poll cannot interrupt an editor submission", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-model", "draft-model");
  await app.runPoll();
  const stale = await app.nextCall("/api/status");
  const submitted = app.element("config-form").dispatch("submit");
  const upload = await app.nextCall("/api/config", "POST");
  stale.reply({ error: "old request failed" }, 500);
  await app.flush();
  assert.equal(upload.signal.aborted, false);
  upload.reply(status(true), 202);
  await submitted;
  await app.flush();
  assert.equal(app.element("channels").hidden, false);
  assert.equal(app.element("config-fields").disabled, true);
});

test("malformed configuration responses never put response secrets in error notices", async () => {
  const app = new Harness();
  app.element("pair-code").value = PAIR_CODE;
  const submitted = app.element("pair-form").dispatch("submit");
  (await app.nextCall("/api/status")).reply(status());
  (await app.nextCall("/api/config")).replyRaw('private-response-key is not valid JSON');
  await submitted;
  await app.flush();
  assert.ok(!notices(app).includes("private-response-key"));
  assert.match(notices(app), /invalid response/i);
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-fields").disabled, true);
  assert.equal(app.element("config-reload").disabled, false);
});

test("a delayed response from a previous pairing cannot overwrite a newly connected editor", async () => {
  const app = new Harness();
  app.element("pair-code").value = PAIR_CODE;
  const old_submit = app.element("pair-form").dispatch("submit");
  (await app.nextCall("/api/status")).reply(status());
  const old_load = await app.nextCall("/api/config");
  await app.element("disconnect").dispatch("click");
  const latest = { ...CONFIG, settings: { ...CONFIG.settings, api_key: "current-session-key", model: "current-session-model" } };
  await pair(app, latest);
  old_load.reply(CONFIG);
  await old_submit;
  await app.flush();
  assert.equal(old_load.signal.aborted, true);
  assert.equal(app.element("config-api-key").value, "current-session-key");
  assert.equal(app.element("config-model").value, "current-session-model");
  assert.equal(app.element("channels").hidden, false);
  assert.equal(app.element("config-fields").disabled, false);
});

test("clearing a masked API key sends an empty key instead of retaining the saved secret", async () => {
  const app = new Harness();
  await pair(app);
  assert.equal(app.element("config-api-key").type, "password");
  await app.edit("config-api-key", "");
  const document = await sendForm(app) as typeof CONFIG;
  assert.equal(document.settings.api_key, "");
  assert.equal(document.settings.model, CONFIG.settings.model);
});

test("invalid keys, models and UTF-8 prompt byte lengths are rejected before sending", async () => {
  const cases = [
    { field: "config-api-key", value: "key with spaces", message: /API key/i },
    { field: "config-api-key", value: "key\nwith-control", message: /API key/i },
    { field: "config-api-key", value: "key\u00a0with-space", message: /API key/i },
    { field: "config-model", value: "invalid model name", message: /model/i },
    { field: "config-system-prompt", value: "中".repeat(5462), message: /16 KiB/i }
  ];
  for (const entry of cases) {
    const app = new Harness();
    await pair(app);
    await app.edit(entry.field, entry.value);
    await app.element("config-form").dispatch("submit");
    await app.flush();
    assert.equal(posts(app), 0, entry.field + " allowed invalid input.");
    assert.match(app.element("config-status").textContent, entry.message);
    assert.ok(!notices(app).includes(CONFIG.settings.api_key));
  }
});

test("Chinese and multiline prompts at the UTF-8 byte limit remain unchanged in the payload", async () => {
  const app = new Harness();
  await pair(app);
  const prompt = "中".repeat(5460) + "\nEND";
  assert.equal(Buffer.byteLength(prompt), 16384);
  await app.edit("config-system-prompt", prompt);
  const document = await sendForm(app) as typeof CONFIG;
  assert.equal(document.settings.system_prompt, prompt);
});

test("advanced setting edits round-trip through the complete configuration document", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-base-url", "https://new-endpoint.invalid/v2");
  await app.edit("config-thinking", false);
  await app.edit("config-max-tokens", "32768");
  await app.edit("config-update-repository", "another-owner/project");
  await app.edit("config-update-channel", "version");
  const document = await sendForm(app);
  assert.deepEqual(document, {
    ...CONFIG,
    settings: {
      ...CONFIG.settings,
      base_url: "https://new-endpoint.invalid/v2",
      thinking: false,
      max_tokens: 32768,
      update_repository: "another-owner/project",
      update_channel: "version"
    }
  });
});

test("a competing import after POST 409 cannot discard the editor's rejected upload draft", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-model", "unsent-draft-after-conflict");
  const submitted = app.element("config-form").dispatch("submit");
  (await app.nextCall("/api/config", "POST")).reply(status(true, "Another import is pending."), 409);
  await submitted;
  await app.flush();
  await pollStatus(app, true, "Confirm another client's import.");
  await pollStatus(app, false, "Another client's configuration imported.");
  (await app.nextCall("/api/config")).reply({
    ...CONFIG, settings: { ...CONFIG.settings, model: "other-client-saved-model" }
  });
  await app.flush();
  assert.equal(app.element("config-model").value, "unsent-draft-after-conflict");
  assert.equal(app.element("config-fields").disabled, true);
  assert.equal(app.element("config-save").disabled, true);
  assert.match(app.element("config-status").textContent, /reload/i);
});
