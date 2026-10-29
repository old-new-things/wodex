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
function providerProfile(patch: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    api_key: "separate-profile-key",
    base_url: "https://provider.invalid/v1",
    model: "profile-model",
    thinking: false,
    max_tokens: 2048,
    system_prompt: "This provider's instructions.",
    ...patch
  };
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

test("changing provider keeps the previous draft and uses an isolated profile for the new provider", async () => {
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
  assert.equal(app.element("config-system-prompt").value, "You are a helpful assistant. Give clear, accurate answers.");
  assert.equal(app.element("config-max-tokens").value, "4096");
  assert.match(app.element("config-status").textContent, /other provider drafts are kept/i);
  const document = await sendForm(app) as { settings: Record<string, unknown> };
  assert.equal(document.settings.provider, "openai_compatible");
  assert.equal(document.settings.api_key, "");
  assert.equal(document.settings.model, "");
  assert.equal(document.settings.thinking, false);
  assert.ok(!("token_limit_field" in document.settings));
  const profiles = document.settings.provider_profiles as Record<string, Record<string, unknown>>;
  assert.equal(profiles.deepseek.api_key, CONFIG.settings.api_key);
  assert.equal(profiles.deepseek.system_prompt, "Keep these instructions.");
  assert.equal(profiles.deepseek.max_tokens, CONFIG.settings.max_tokens);
  assert.ok(!("openai_compatible" in profiles));
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

test("switching back to DeepSeek restores its saved profile and keeps compatible-provider edits", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-provider", "openai_compatible");
  await app.edit("config-api-key", "compatible-draft-key");
  await app.edit("config-model", "compatible-model");
  await app.edit("config-token-limit-field", "max_completion_tokens");
  await app.edit("config-provider", "deepseek");
  assert.equal(app.element("config-api-key").value, CONFIG.settings.api_key);
  assert.equal(app.element("config-base-url").value, CONFIG.settings.base_url);
  assert.equal(app.element("config-model").value, CONFIG.settings.model);
  assert.equal(app.element("config-token-limit-field").value, "max_tokens");
  assert.equal(app.element("config-token-limit-field").disabled, true);
  assert.equal(app.element("config-thinking").disabled, false);
  const document = await sendForm(app) as { settings: Record<string, unknown> };
  assert.ok(!("provider" in document.settings));
  assert.ok(!("token_limit_field" in document.settings));
  assert.equal(document.settings.api_key, CONFIG.settings.api_key);
  assert.equal(document.settings.thinking, true);
  const profiles = document.settings.provider_profiles as Record<string, Record<string, unknown>>;
  assert.equal(profiles.openai_compatible.api_key, "compatible-draft-key");
  assert.equal(profiles.openai_compatible.model, "compatible-model");
  assert.equal(profiles.openai_compatible.token_limit_field, "max_completion_tokens");
  assert.ok(!("deepseek" in profiles));
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
    { field: "config-api-key", value: "key-中文", message: /printable ASCII/i },
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

test("every named provider has its own endpoint and isolated default options", async () => {
  const defaults = [
    ["openai", "https://api.openai.com/v1", "max_completion_tokens"],
    ["opencode_go", "https://opencode.ai/zen/go/v1", "max_tokens"],
    ["llama_cpp", "http://127.0.0.1:8080/v1", "max_tokens"],
    ["zai", "https://api.z.ai/api/paas/v4", "max_tokens"],
    ["zai_coding_plan", "https://api.z.ai/api/coding/paas/v4", "max_tokens"],
    ["zhipuai", "https://open.bigmodel.cn/api/paas/v4", "max_tokens"],
    ["zhipuai_coding_plan", "https://open.bigmodel.cn/api/coding/paas/v4", "max_tokens"],
    ["openai_compatible", "https://api.openai.com/v1", "max_tokens"]
  ];
  for (const [provider, endpoint, token_limit_field] of defaults) {
    const app = new Harness();
    assert.ok(app.element("config-provider").querySelectorAll("option").some(option => option.value === provider));
    await pair(app);
    await app.edit("config-provider", provider);
    assert.equal(app.element("config-api-key").value, "");
    assert.equal(app.element("config-base-url").value, endpoint);
    assert.equal(app.element("config-model").value, "");
    assert.equal(app.element("config-token-limit-field").value, token_limit_field);
    assert.equal(app.element("config-max-tokens").value, "4096");
    assert.equal(app.element("config-thinking").checked, false);
    assert.equal(app.element("config-allow-insecure-http").checked, false);
    assert.equal(app.element("config-insecure-http-field").hidden, provider !== "llama_cpp");
    assert.equal(app.element("config-update-repository").value, CONFIG.settings.update_repository);
    assert.equal(app.element("config-update-channel").value, CONFIG.settings.update_channel);
    assert.equal(posts(app), 0);
  }
});

test("saved profiles restore keys and every edited option without changing update preferences", async () => {
  const openai = providerProfile({ token_limit_field: "max_completion_tokens" });
  const app = new Harness();
  await pair(app, { ...CONFIG, settings: { ...CONFIG.settings, provider_profiles: { openai } } });
  assert.equal(app.element("config-save").disabled, true);
  await app.edit("config-provider", "openai");
  assert.equal(app.element("config-api-key").value, openai.api_key);
  assert.equal(app.element("config-base-url").value, openai.base_url);
  assert.equal(app.element("config-model").value, openai.model);
  assert.equal(app.element("config-max-tokens").value, String(openai.max_tokens));
  assert.equal(app.element("config-system-prompt").value, openai.system_prompt);
  assert.equal(app.element("config-token-limit-field").value, "max_completion_tokens");
  await app.edit("show-api-key", true);
  await app.edit("config-api-key", "edited-openai-key");
  await app.edit("config-base-url", "https://edited.invalid/v2");
  await app.edit("config-model", "edited-model");
  await app.edit("config-max-tokens", "16384");
  await app.edit("config-system-prompt", "Edited provider-specific 中文 instructions.");
  await app.edit("config-token-limit-field", "max_tokens");
  await app.edit("config-provider", "deepseek");
  assert.equal(app.element("config-api-key").value, CONFIG.settings.api_key);
  assert.equal(app.element("config-api-key").type, "password");
  assert.equal(app.element("show-api-key").checked, false);
  assert.equal(app.element("config-thinking").checked, true);
  assert.equal(app.element("config-system-prompt").value, CONFIG.settings.system_prompt);
  await app.edit("config-provider", "openai");
  assert.equal(app.element("config-api-key").value, "edited-openai-key");
  assert.equal(app.element("config-model").value, "edited-model");
  assert.equal(app.element("config-max-tokens").value, "16384");
  assert.equal(app.element("config-base-url").value, "https://edited.invalid/v2");
  assert.equal(app.element("config-system-prompt").value, "Edited provider-specific 中文 instructions.");
  assert.equal(app.element("config-token-limit-field").value, "max_tokens");
  const document = await sendForm(app) as { settings: Record<string, unknown> };
  assert.equal(document.settings.provider, "openai");
  assert.equal(document.settings.api_key, "edited-openai-key");
  assert.equal(document.settings.update_repository, CONFIG.settings.update_repository);
  assert.equal(document.settings.update_channel, CONFIG.settings.update_channel);
  const profiles = document.settings.provider_profiles as Record<string, Record<string, unknown>>;
  assert.deepEqual(Object.keys(profiles), ["deepseek"]);
  assert.equal(profiles.deepseek.api_key, CONFIG.settings.api_key);
  assert.equal(profiles.deepseek.system_prompt, CONFIG.settings.system_prompt);
  assert.equal(profiles.deepseek.thinking, true);
  assert.equal(app.element("config-fields").disabled, true, "Saving all provider profiles still requires Switch confirmation.");
});

test("equivalent saved profile maps do not invalidate a dirty draft", async () => {
  const openai = providerProfile({ token_limit_field: "max_completion_tokens" });
  const zai = providerProfile({ api_key: "zai-profile-key", base_url: "https://api.z.ai/api/paas/v4" });
  const app = new Harness();
  await pair(app, { ...CONFIG, settings: { ...CONFIG.settings, provider_profiles: { openai, zai } } });
  await app.edit("config-model", "unsent-active-model");
  await pollStatus(app, true, "Confirm a sessions import.");
  await pollStatus(app, false, "Sessions imported.");
  (await app.nextCall("/api/config")).reply({
    ...CONFIG,
    settings: {
      ...CONFIG.settings,
      provider: "deepseek",
      allow_insecure_http: false,
      provider_profiles: {
        zai: { ...zai, token_limit_field: "max_tokens", allow_insecure_http: false },
        openai: { ...openai, allow_insecure_http: false }
      }
    }
  });
  await app.flush();
  assert.equal(app.element("config-model").value, "unsent-active-model");
  assert.equal(app.element("config-fields").disabled, false);
  assert.equal(app.element("config-save").disabled, false);
  assert.equal(posts(app), 0);
});

test("an external change to an inactive profile protects the unsent draft from overwriting it", async () => {
  const openai = providerProfile({ token_limit_field: "max_completion_tokens" });
  const app = new Harness();
  await pair(app, { ...CONFIG, settings: { ...CONFIG.settings, provider_profiles: { openai } } });
  await app.edit("config-model", "unsent-active-model");
  await pollStatus(app, true, "Confirm another configuration.");
  await pollStatus(app, false, "Other configuration imported.");
  (await app.nextCall("/api/config")).reply({
    ...CONFIG,
    settings: { ...CONFIG.settings, provider_profiles: { openai: { ...openai, api_key: "another-client-profile-key" } } }
  });
  await app.flush();
  assert.equal(app.element("config-model").value, "unsent-active-model");
  assert.equal(app.element("config-fields").disabled, true);
  assert.equal(app.element("config-save").disabled, true);
  assert.match(app.element("config-status").textContent, /reload/i);
  assert.ok(!notices(app).includes("another-client-profile-key"));
  assert.equal(posts(app), 0);
});

test("malformed or conflicting inactive profiles fail closed without showing keys", async () => {
  const invalid = [
    null,
    [],
    { unknown: providerProfile() },
    { deepseek: providerProfile() },
    { openai: null },
    { openai: providerProfile({ unknown_field: true }) },
    { openai: providerProfile({ api_key: "private key with spaces" }) },
    { openai: providerProfile({ api_key: "private-key-中文" }) },
    { openai: providerProfile({ model: "bad model" }) },
    { openai: providerProfile({ thinking: true }) },
    { openai: providerProfile({ token_limit_field: "unknown" }) },
    { openai: providerProfile({ allow_insecure_http: true }) },
    { openai: providerProfile({ allow_insecure_http: "false" }) },
    { openai: providerProfile({ system_prompt: "中".repeat(5462) }) },
    { llama_cpp: providerProfile({ base_url: "http://192.168.1.2:8080/v1" }) }
  ];
  for (const provider_profiles of invalid) {
    const app = new Harness();
    await pair(app, { ...CONFIG, settings: { ...CONFIG.settings, provider_profiles } });
    assert.equal(app.element("config-fields").disabled, true);
    assert.equal(app.element("config-api-key").value, "");
    assert.equal(posts(app), 0);
    assert.ok(!notices(app).includes("separate-profile-key"));
  }
});

test("llama.cpp permits private LAN HTTP only with an explicit option and keeps its key optional", async () => {
  for (const endpoint of [
    "http://10.0.0.2:8080/v1", "http://172.16.0.1/v1",
    "http://192.168.1.12:8080/v1", "http://server.local:8080/v1"
  ]) {
    const app = new Harness();
    await pair(app);
    await app.edit("config-provider", "llama_cpp");
    await app.edit("config-base-url", endpoint);
    await app.edit("config-model", "local-model");
    await app.element("config-form").dispatch("submit");
    await app.flush();
    assert.equal(posts(app), 0, "Private HTTP was sent before the explicit option.");
    await app.edit("config-allow-insecure-http", true);
    const document = await sendForm(app) as { settings: Record<string, unknown> };
    assert.equal(document.settings.base_url, endpoint);
    assert.equal(document.settings.api_key, "");
    assert.equal(document.settings.allow_insecure_http, true);
    assert.equal(document.settings.provider, "llama_cpp");
  }
});

test("llama.cpp HTTP cannot target public, unspecified, multicast or disguised private addresses", async () => {
  for (const endpoint of [
    "http://172.32.0.1/v1", "http://8.8.8.8/v1", "http://0.0.0.0/v1",
    "http://224.0.0.1/v1", "http://010.1.1.2/v1", "http://10.1.256.2/v1",
    "http://192.168.1.1.evil.invalid/v1", "http://server.local.evil.invalid/v1"
  ]) {
    const app = new Harness();
    await pair(app);
    await app.edit("config-provider", "llama_cpp");
    await app.edit("config-base-url", endpoint);
    await app.edit("config-model", "local-model");
    await app.edit("config-allow-insecure-http", true);
    await app.element("config-form").dispatch("submit");
    await app.flush();
    assert.equal(posts(app), 0, "Invalid HTTP destination was sent: " + endpoint);
  }
});

test("invalid token input in an inactive draft cannot escape validation by switching providers", async () => {
  const app = new Harness();
  await pair(app);
  await app.edit("config-max-tokens", "1e3");
  await app.edit("config-provider", "openai");
  await app.edit("config-model", "openai-model");
  await app.element("config-form").dispatch("submit");
  await app.flush();
  assert.equal(posts(app), 0);
  assert.match(app.element("config-status").textContent, /whole number/i);
});

test("disconnect discards inactive provider credentials as well as the displayed key", async () => {
  const openai = providerProfile({ api_key: "private-inactive-provider-key" });
  const app = new Harness();
  await pair(app, { ...CONFIG, settings: { ...CONFIG.settings, provider_profiles: { openai } } });
  await app.edit("config-provider", "openai");
  assert.equal(app.element("config-api-key").value, openai.api_key);
  await app.element("disconnect").dispatch("click");
  await pair(app);
  await app.edit("config-provider", "openai");
  assert.equal(app.element("config-api-key").value, "");
  assert.equal(app.element("config-base-url").value, "https://api.openai.com/v1");
  assert.ok(!notices(app).includes("private-inactive-provider-key"));
});

test("reloading with confirmation discards inactive drafts and restores saved provider profiles", async () => {
  const openai = providerProfile();
  const saved = { ...CONFIG, settings: { ...CONFIG.settings, provider_profiles: { openai } } };
  const app = new Harness();
  await pair(app, saved);
  await app.edit("config-provider", "openai");
  await app.edit("config-api-key", "unsent-provider-key");
  await app.edit("config-provider", "deepseek");
  app.confirmationAnswers.push(false);
  await app.element("config-reload").dispatch("click");
  await app.edit("config-provider", "openai");
  assert.equal(app.element("config-api-key").value, "unsent-provider-key");
  app.confirmationAnswers.push(true);
  const reload = app.element("config-reload").dispatch("click");
  (await app.nextCall("/api/config")).reply(saved);
  await reload;
  await app.flush();
  assert.equal(app.element("config-provider").value, "deepseek");
  assert.equal(app.element("config-save").disabled, true);
  await app.edit("config-provider", "openai");
  assert.equal(app.element("config-api-key").value, openai.api_key);
  assert.equal(posts(app), 0);
});
