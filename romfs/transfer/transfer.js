"use strict";
// The editor handles saved configuration. The transfer controller owns
// authentication, requests and Switch-side confirmation.
const CONFIGURATION_PROVIDERS = [
    "deepseek", "openai", "opencode_go", "llama_cpp", "zai", "zai_coding_plan",
    "zhipuai", "zhipuai_coding_plan", "openai_compatible"
];
function isConfigurationProvider(value) {
    return typeof value === "string" && CONFIGURATION_PROVIDERS.some(provider => provider === value);
}
function defaultProviderProfile(provider) {
    const base_urls = {
        deepseek: "https://api.deepseek.com",
        openai: "https://api.openai.com/v1",
        opencode_go: "https://opencode.ai/zen/go/v1",
        llama_cpp: "http://127.0.0.1:8080/v1",
        zai: "https://api.z.ai/api/paas/v4",
        zai_coding_plan: "https://api.z.ai/api/coding/paas/v4",
        zhipuai: "https://open.bigmodel.cn/api/paas/v4",
        zhipuai_coding_plan: "https://open.bigmodel.cn/api/coding/paas/v4",
        openai_compatible: "https://api.openai.com/v1"
    };
    return {
        token_limit_field: provider === "openai" ? "max_completion_tokens" : "max_tokens",
        api_key: "",
        base_url: base_urls[provider],
        model: provider === "deepseek" ? "deepseek-flash" : "",
        thinking: false,
        max_tokens: 4096,
        system_prompt: "You are a helpful assistant. Give clear, accurate answers.",
        allow_insecure_http: false
    };
}
const PROVIDER_PROFILE_KEYS = Object.keys(defaultProviderProfile("deepseek"));
function recordValue(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
function readProviderProfile(value, extra_keys = []) {
    if (!recordValue(value) ||
        typeof value.api_key !== "string" || typeof value.base_url !== "string" ||
        typeof value.model !== "string" || typeof value.thinking !== "boolean" ||
        typeof value.max_tokens !== "number" || typeof value.system_prompt !== "string") {
        throw new Error("Unsupported provider profile from Wodex. Reload using a matching app version.");
    }
    const keys = [...PROVIDER_PROFILE_KEYS, ...extra_keys];
    if (Object.keys(value).some(key => !keys.includes(key))) {
        throw new Error("Unsupported provider profile fields from Wodex. Reload using a matching app version.");
    }
    return {
        token_limit_field: (value.token_limit_field === undefined ? "max_tokens" : value.token_limit_field),
        api_key: value.api_key,
        base_url: value.base_url,
        model: value.model,
        thinking: value.thinking,
        max_tokens: value.max_tokens,
        system_prompt: value.system_prompt,
        allow_insecure_http: (value.allow_insecure_http === undefined ? false : value.allow_insecure_http)
    };
}
function readConfiguration(value) {
    if (!recordValue(value) || value.schema_version !== 1 || value.channel !== "config" || !recordValue(value.settings)) {
        throw new Error("Unsupported configuration from Wodex. Reload using a matching app version.");
    }
    const saved = value.settings;
    const provider = saved.provider === undefined ? "deepseek" : saved.provider;
    if (!isConfigurationProvider(provider) || typeof saved.update_repository !== "string" ||
        (saved.update_channel !== "version" && saved.update_channel !== "nightly")) {
        throw new Error("Unsupported configuration from Wodex. Reload using a matching app version.");
    }
    const provider_profiles = {};
    if (saved.provider_profiles !== undefined) {
        if (!recordValue(saved.provider_profiles))
            throw new Error("Invalid saved provider profiles from Wodex.");
        for (const [id, profile] of Object.entries(saved.provider_profiles)) {
            if (!isConfigurationProvider(id) || id === provider)
                throw new Error("Invalid saved provider profiles from Wodex.");
            provider_profiles[id] = readProviderProfile(profile);
        }
    }
    const settings = {
        ...readProviderProfile(saved, ["provider", "provider_profiles", "update_repository", "update_channel"]),
        provider,
        provider_profiles,
        update_repository: saved.update_repository,
        update_channel: saved.update_channel
    };
    validateConfiguration(settings);
    return { schema_version: 1, channel: "config", settings };
}
function privateLanHost(host) {
    if (host.toLowerCase().endsWith(".local"))
        return true;
    const parts = host.split(".");
    if (parts.length !== 4 || parts.some(part => !/^(0|[1-9][0-9]{0,2})$/.test(part) || Number(part) > 255))
        return false;
    const octets = parts.map(Number);
    return octets[0] === 10 || (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) ||
        (octets[0] === 192 && octets[1] === 168);
}
function validateProviderProfile(provider, profile) {
    const bytes = (text) => new TextEncoder().encode(text).length;
    if (profile.token_limit_field !== "max_tokens" && profile.token_limit_field !== "max_completion_tokens") {
        throw new Error("Choose max_tokens or max_completion_tokens for the token limit field.");
    }
    if (provider === "deepseek" && profile.token_limit_field !== "max_tokens") {
        throw new Error("DeepSeek uses max_tokens for its response token limit.");
    }
    if (typeof profile.thinking !== "boolean" || (provider !== "deepseek" && profile.thinking)) {
        throw new Error("Thinking mode is available only with the DeepSeek provider.");
    }
    if (typeof profile.allow_insecure_http !== "boolean" || (profile.allow_insecure_http && provider !== "llama_cpp")) {
        throw new Error("Local HTTP can be enabled only for llama.cpp.");
    }
    if (bytes(profile.api_key) > 4096 || /[^\x21-\x7e]/.test(profile.api_key)) {
        throw new Error("The API key must use printable ASCII characters without spaces and be at most 4096 bytes.");
    }
    if (!(provider !== "deepseek" && profile.model === "") && !/^[A-Za-z0-9_./:-]{1,128}$/.test(profile.model)) {
        throw new Error("Enter a model name of at most 128 ASCII letters, numbers, or _ . / : - characters.");
    }
    if (!Number.isInteger(profile.max_tokens) || profile.max_tokens < 1 || profile.max_tokens > 131072) {
        throw new Error("The response token limit must be a whole number from 1 to 131072.");
    }
    if (bytes(profile.system_prompt) > 16384 || profile.system_prompt.includes("\0")) {
        throw new Error("The system prompt must be at most 16 KiB of UTF-8 text and cannot contain NUL characters.");
    }
    // Match Rust without URL() silently resolving dot segments or escapes.
    const url = profile.base_url.replace(/\/+$/, "");
    const scheme = /^(https?):\/\/(.*)$/.exec(url);
    const remainder = scheme?.[2] || "";
    const authority = remainder.split("/")[0];
    const host = /^(\[::1\]|[A-Za-z0-9.-]+)(?::([0-9]+))?$/.exec(authority);
    const loopback = host && ["localhost", "127.0.0.1", "[::1]"].includes(host[1]);
    const local_http = host && provider === "llama_cpp" && profile.allow_insecure_http && privateLanHost(host[1]);
    if (url.length > 2048 || /[^\x21-\x7e]|[?#@\\"<>%]/.test(url) || !scheme || !host ||
        host[1].startsWith(".") || host[1].endsWith(".") || host[1].includes("..") ||
        (host[2] !== undefined && (Number(host[2]) < 1 || Number(host[2]) > 65535)) ||
        (scheme[1] === "http" && !loopback && !local_http) || remainder.includes("//") ||
        remainder.split("/").slice(1).some(part => part === "." || part === "..")) {
        throw new Error("Use an HTTPS endpoint without credentials, query, or fragment. Local HTTP requires the llama.cpp option; localhost is also allowed.");
    }
}
function validateConfiguration(settings) {
    if (!isConfigurationProvider(settings.provider))
        throw new Error("Choose a supported provider.");
    validateProviderProfile(settings.provider, settings);
    for (const [id, profile] of Object.entries(settings.provider_profiles)) {
        if (!isConfigurationProvider(id) || id === settings.provider || !profile)
            throw new Error("Invalid saved provider profiles.");
        validateProviderProfile(id, profile);
    }
    const repository = settings.update_repository;
    if (repository) {
        const parts = repository.split("/");
        const owner = parts[0];
        const name = parts[1] || "";
        if (parts.length !== 2 || !/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$/.test(owner) ||
            owner.includes("--") || !/^[A-Za-z0-9_.-]{1,100}$/.test(name) || name === "." || name.includes("..")) {
            throw new Error("Enter an update source as owner/repo, or leave it empty to disable update checks.");
        }
    }
    if (settings.update_channel !== "version" && settings.update_channel !== "nightly") {
        throw new Error("Choose Versions or Nightly for updates.");
    }
}
function copyProfiles(profiles) {
    const copied = {};
    for (const provider of CONFIGURATION_PROVIDERS) {
        const profile = profiles[provider];
        if (profile)
            copied[provider] = { ...profile };
    }
    return copied;
}
function sameProfile(first, second) {
    if (!first || !second)
        return first === second;
    return PROVIDER_PROFILE_KEYS.every(key => first[key] === second[key]);
}
function sameConfiguration(first, second) {
    return first.provider === second.provider && first.update_repository === second.update_repository &&
        first.update_channel === second.update_channel && sameProfile(first, second) &&
        CONFIGURATION_PROVIDERS.every(provider => sameProfile(first.provider_profiles[provider], second.provider_profiles[provider]));
}
function profileWire(profile) {
    return {
        api_key: profile.api_key,
        base_url: profile.base_url,
        model: profile.model,
        thinking: profile.thinking,
        max_tokens: profile.max_tokens,
        system_prompt: profile.system_prompt,
        ...(profile.token_limit_field === "max_tokens" ? {} : { token_limit_field: profile.token_limit_field }),
        ...(profile.allow_insecure_http ? { allow_insecure_http: true } : {})
    };
}
class ConfigurationEditor {
    constructor(onChange) {
        this.form = element("config-form");
        this.reload = element("config-reload");
        this.fields = element("config-fields");
        this.save = element("config-save");
        this.status = element("config-status");
        this.provider = element("config-provider");
        this.apiKey = element("config-api-key");
        this.keyHint = element("api-key-hint");
        this.showKey = element("show-api-key");
        this.model = element("config-model");
        this.prompt = element("config-system-prompt");
        this.endpoint = element("config-base-url");
        this.thinking = element("config-thinking");
        this.thinkingField = element("config-thinking-field");
        this.tokenLimitField = element("config-token-limit-field");
        this.tokenLimitChoice = element("config-token-limit-choice");
        this.insecureHttp = element("config-allow-insecure-http");
        this.insecureHttpField = element("config-insecure-http-field");
        this.maxTokens = element("config-max-tokens");
        this.repository = element("config-update-repository");
        this.channel = element("config-update-channel");
        this.baseline = null;
        this.profiles = {};
        this.stale = false;
        this.selectedProvider = "deepseek";
        const changed = (event) => {
            // A select emits input then change. Switch once and keep its notice.
            if (event.target === this.provider && event.type === "input")
                return;
            const switched = event.target === this.provider && this.switchProvider();
            if (this.baseline && !this.stale) {
                this.notice(switched ? "Provider changed. Its own settings are shown; other provider drafts are kept. Review and send changes to save them on your Switch." :
                    this.dirty() ? "Unsent changes. Send them to your Switch when ready." : "Showing saved configuration.");
            }
            onChange();
        };
        this.form.addEventListener("input", changed);
        this.form.addEventListener("change", changed);
        this.showKey.addEventListener("change", () => {
            this.apiKey.type = this.showKey.checked ? "text" : "password";
        });
    }
    notice(message, error = false) {
        this.status.textContent = message;
        this.status.dataset.error = String(error);
    }
    clear() {
        this.baseline = null;
        this.profiles = {};
        this.stale = false;
        for (const input of [this.apiKey, this.model, this.prompt, this.endpoint, this.maxTokens, this.repository])
            input.value = "";
        this.channel.value = "version";
        this.provider.value = this.selectedProvider = "deepseek";
        this.tokenLimitField.value = "max_tokens";
        this.thinking.checked = this.insecureHttp.checked = false;
        this.providerControls();
        this.maskKey();
        this.notice("Connect to load your saved configuration.");
    }
    load(document) {
        const settings = document.settings;
        this.baseline = { ...document, settings: { ...settings, provider_profiles: copyProfiles(settings.provider_profiles) } };
        this.profiles = copyProfiles(settings.provider_profiles);
        this.stale = false;
        this.provider.value = this.selectedProvider = settings.provider;
        this.loadProfile(settings);
        this.repository.value = settings.update_repository;
        this.channel.value = settings.update_channel;
        this.providerControls();
        this.maskKey();
    }
    loadProfile(profile) {
        this.tokenLimitField.value = profile.token_limit_field;
        this.apiKey.value = profile.api_key;
        this.model.value = profile.model;
        this.prompt.value = profile.system_prompt;
        this.endpoint.value = profile.base_url;
        this.thinking.checked = profile.thinking;
        this.maxTokens.value = String(profile.max_tokens);
        this.insecureHttp.checked = profile.allow_insecure_http;
    }
    switchProvider() {
        const provider = this.provider.value;
        if (!isConfigurationProvider(provider) || provider === this.selectedProvider)
            return false;
        this.profiles[this.selectedProvider] = this.profileValues();
        const profile = this.profiles[provider] || defaultProviderProfile(provider);
        delete this.profiles[provider];
        this.selectedProvider = provider;
        this.loadProfile(profile);
        this.providerControls();
        this.maskKey();
        return true;
    }
    providerControls() {
        const deepseek = this.provider.value === "deepseek";
        const llama = this.provider.value === "llama_cpp";
        this.thinkingField.hidden = this.thinking.disabled = !deepseek;
        this.tokenLimitChoice.hidden = this.tokenLimitField.disabled = deepseek;
        this.insecureHttpField.hidden = this.insecureHttp.disabled = !llama;
        this.model.required = deepseek;
        this.keyHint.textContent = llama
            ? "Optional if your llama.cpp server does not require a key. Environment keys are never shown here."
            : "Leave empty to clear this provider’s saved key. Environment keys are never shown here.";
    }
    reconcile(document, message) {
        if (!this.dirty()) {
            this.load(document);
            this.notice(message);
            return;
        }
        if (!sameConfiguration(this.baseline.settings, document.settings)) {
            this.requireReload("Saved configuration changed while you had unsent edits. Your draft is kept here; reload saved configuration before sending it.");
        }
        else {
            this.stale = false;
            this.notice("Saved configuration is unchanged. Your unsent edits are kept here.");
        }
    }
    maskKey() {
        this.showKey.checked = false;
        this.apiKey.type = "password";
    }
    profileValues() {
        return {
            token_limit_field: this.tokenLimitField.value,
            api_key: this.apiKey.value,
            base_url: this.endpoint.value,
            model: this.model.value,
            thinking: this.thinking.checked,
            max_tokens: /^[0-9]+$/.test(this.maxTokens.value) ? Number(this.maxTokens.value) : NaN,
            system_prompt: this.prompt.value,
            allow_insecure_http: this.insecureHttp.checked
        };
    }
    values() {
        return {
            ...this.profileValues(),
            provider: this.provider.value,
            provider_profiles: copyProfiles(this.profiles),
            update_repository: this.repository.value,
            update_channel: this.channel.value
        };
    }
    dirty() {
        return this.baseline !== null && !sameConfiguration(this.values(), this.baseline.settings);
    }
    document() {
        if (!this.baseline || this.stale)
            throw new Error("Reload saved configuration before sending changes.");
        const settings = this.values();
        validateConfiguration(settings);
        const provider_profiles = {};
        for (const provider of CONFIGURATION_PROVIDERS) {
            const profile = settings.provider_profiles[provider];
            if (profile)
                provider_profiles[provider] = profileWire(profile);
        }
        return {
            schema_version: 1,
            channel: "config",
            settings: {
                ...profileWire(settings),
                update_repository: settings.update_repository,
                update_channel: settings.update_channel,
                ...(settings.provider === "deepseek" ? {} : { provider: settings.provider }),
                ...(Object.keys(provider_profiles).length ? { provider_profiles } : {})
            }
        };
    }
    requireReload(message) {
        this.stale = true;
        this.maskKey();
        this.notice(message, true);
    }
    controls(connected, blocked) {
        this.fields.disabled = blocked || !connected || !this.baseline || this.stale;
        this.save.disabled = this.fields.disabled || !this.dirty();
        this.reload.disabled = blocked || !connected;
    }
}
function element(id) {
    const result = document.getElementById(id);
    if (!result)
        throw new Error("Missing transfer page control.");
    return result;
}
const pairForm = element("pair-form");
const pairCode = element("pair-code");
const showCode = element("show-code");
const channels = element("channels");
const connection = element("connection");
const statusText = element("status");
const connectButton = element("connect");
let pairingCode = "";
let pollTimer;
let busy = false;
let pending = false;
let generation = 0;
let statusRevision = 0;
const requests = new Set();
const configEditor = new ConfigurationEditor(controls);
let awaitingEditor = false;
function notice(message, error = false) {
    statusText.textContent = message;
    statusText.dataset.error = String(error);
}
function controls() {
    document.querySelectorAll(".download, .upload").forEach(button => {
        button.disabled = busy || pending || !pairingCode;
    });
    document.querySelectorAll('input[type="file"]').forEach(input => {
        input.disabled = busy || pending || !pairingCode;
    });
    connectButton.disabled = busy;
    configEditor.controls(Boolean(pairingCode), busy || pending);
}
function disconnect(message, error = false) {
    generation += 1;
    statusRevision += 1;
    pairingCode = "";
    pairCode.value = "";
    pairCode.type = "password";
    showCode.checked = false;
    pending = false;
    busy = false;
    awaitingEditor = false;
    configEditor.clear();
    window.clearTimeout(pollTimer);
    for (const request of requests)
        request.abort();
    requests.clear();
    channels.hidden = true;
    connection.hidden = true;
    pairForm.hidden = false;
    document.querySelectorAll('input[type="file"]').forEach(input => { input.value = ""; });
    controls();
    notice(message, error);
}
async function request(path, method = "GET", body) {
    const controller = new AbortController();
    requests.add(controller);
    const timeout = window.setTimeout(() => controller.abort(), 20000);
    try {
        const headers = { Authorization: `Bearer ${pairingCode}` };
        if (body !== undefined)
            headers["Content-Type"] = "application/json";
        const response = await fetch(path, {
            method, headers, body, signal: controller.signal,
            cache: "no-store", credentials: "omit", redirect: "error"
        });
        // Consume the response before clearing the timeout, including downloads.
        const bytes = await response.arrayBuffer();
        const complete = new Response(bytes, {
            status: response.status, statusText: response.statusText, headers: response.headers
        });
        if (!complete.ok) {
            if (complete.status === 401)
                throw new Error("Pairing code rejected. Enter the current code shown in Wodex.");
            if (complete.status === 409)
                throw new Error("An import is already waiting for confirmation on your Switch.");
            if (complete.status === 413)
                throw new Error("The file is too large. The limit is 4 MiB.");
            throw new Error(`Wodex could not complete this transfer (HTTP ${complete.status}).`);
        }
        return complete;
    }
    finally {
        window.clearTimeout(timeout);
        requests.delete(controller);
    }
}
function readStatus(value) {
    if (typeof value !== "object" || value === null)
        throw new Error("Invalid response from Wodex.");
    const status = value;
    if (typeof status.active !== "boolean" || typeof status.pending !== "boolean" || typeof status.message !== "string") {
        throw new Error("Invalid response from Wodex.");
    }
    return status;
}
function errorMessage(error) {
    // JSON parser errors can quote response contents, including an API key.
    if (error instanceof SyntaxError)
        return "Wodex returned an invalid response. Reload and try again.";
    return error instanceof Error && error.name !== "AbortError"
        ? error.message : "Connection interrupted. Keep Wodex open and check your local network.";
}
// Callers hold the busy flag so a load cannot overwrite edits made in flight.
async function loadConfiguration(message = "Showing saved configuration. Edit it below.", preserveDraft = false) {
    const current = generation;
    configEditor.notice("Loading saved configuration…");
    try {
        const value = await (await request("/api/config")).json();
        if (current !== generation)
            return;
        const document = readConfiguration(value);
        if (preserveDraft)
            configEditor.reconcile(document, message);
        else {
            configEditor.load(document);
            configEditor.notice(message);
        }
    }
    catch (error) {
        if (current === generation) {
            configEditor.requireReload(`${errorMessage(error)} Use Reload saved configuration to retry.`);
        }
    }
}
async function poll() {
    const current = generation;
    const revision = statusRevision;
    try {
        if (!pairingCode || busy)
            return;
        const status = readStatus(await (await request("/api/status")).json());
        if (current !== generation || revision !== statusRevision)
            return;
        if (!status.active) {
            disconnect("Transfer stopped. Start a new transfer in Wodex to reconnect.");
            return;
        }
        const changed = pending !== status.pending;
        pending = status.pending;
        if (pending || changed)
            notice(status.message || "Confirm or reject the import on your Switch.");
        if (pending) {
            configEditor.notice("An import is waiting. Confirm or reject it on your Switch before editing.");
        }
        else if (changed) {
            const preserveDraft = !awaitingEditor;
            awaitingEditor = false;
            busy = true;
            statusRevision += 1;
            controls();
            await loadConfiguration(`${status.message || "Import finished."} Showing saved configuration.`, preserveDraft);
            if (current !== generation)
                return;
            busy = false;
        }
        controls();
    }
    catch (error) {
        if (current === generation && revision === statusRevision)
            disconnect(errorMessage(error), true);
    }
    finally {
        if (current === generation && pairingCode)
            pollTimer = window.setTimeout(poll, 1500);
    }
}
pairForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const code = pairCode.value.replace(/[\s-]/g, "").toLowerCase();
    if (!/^[0-9a-f]{32}$/.test(code)) {
        notice("Enter the full 32-character pairing code shown in Wodex.", true);
        return;
    }
    disconnect("Connecting…");
    pairingCode = code;
    busy = true;
    controls();
    const current = generation;
    try {
        const status = readStatus(await (await request("/api/status")).json());
        if (current !== generation)
            return;
        if (!status.active)
            throw new Error("Start PC transfer in Wodex before connecting.");
        pending = status.pending;
        pairForm.hidden = true;
        channels.hidden = false;
        connection.hidden = false;
        notice(pending ? status.message : "Connected. Edit configuration or transfer a backup below.");
        await loadConfiguration(pending ? "An import is waiting for confirmation on your Switch." : undefined);
        if (current !== generation)
            return;
        pollTimer = window.setTimeout(poll, 1500);
    }
    catch (error) {
        if (current === generation)
            disconnect(errorMessage(error), true);
    }
    finally {
        if (current === generation) {
            busy = false;
            controls();
        }
    }
});
element("disconnect").addEventListener("click", () => disconnect("Disconnected from this browser. Stop transfer in Wodex when finished."));
showCode.addEventListener("change", () => { pairCode.type = showCode.checked ? "text" : "password"; });
configEditor.reload.addEventListener("click", async () => {
    if (busy || pending || !pairingCode)
        return;
    if (configEditor.dirty() && !window.confirm("Discard your unsent changes and reload the saved configuration from Wodex?"))
        return;
    const current = generation;
    statusRevision += 1;
    busy = true;
    controls();
    await loadConfiguration();
    if (current === generation) {
        awaitingEditor = false;
        busy = false;
        controls();
    }
});
configEditor.form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy || pending || !pairingCode || !configEditor.dirty())
        return;
    let body;
    try {
        body = JSON.stringify(configEditor.document());
    }
    catch (error) {
        configEditor.notice(errorMessage(error), true);
        return;
    }
    const current = generation;
    statusRevision += 1;
    busy = true;
    awaitingEditor = false;
    controls();
    configEditor.notice("Sending configuration to your Switch…");
    try {
        const response = await request("/api/config", "POST", body);
        if (current !== generation)
            return;
        if (response.status !== 202)
            throw new Error("Wodex did not acknowledge the configuration change.");
        awaitingEditor = true;
        pending = true;
        statusRevision += 1;
        const message = "Changes sent. Confirm replacement of provider profiles and preferences on your Switch to save them, or reject to keep the previous settings.";
        configEditor.notice(message);
        notice(message);
    }
    catch (error) {
        if (current === generation) {
            configEditor.requireReload(`${errorMessage(error)} Check your Switch, then reload saved configuration before trying again.`);
        }
    }
    finally {
        if (current === generation) {
            busy = false;
            controls();
        }
    }
});
async function download(channel) {
    const current = generation;
    statusRevision += 1;
    busy = true;
    controls();
    notice(`Downloading ${channel === "config" ? "configuration" : "sessions"}…`);
    try {
        const response = await request(`/api/${channel}`);
        const blob = await response.blob();
        if (current !== generation)
            return;
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = `wodex-${channel}.json`;
        document.body.append(link);
        link.click();
        link.remove();
        window.setTimeout(() => URL.revokeObjectURL(url), 1000);
        notice(channel === "config"
            ? "Configuration downloaded, including all provider profiles and saved API keys. Keep this file private."
            : "Sessions downloaded. Keep this chat backup private.");
    }
    catch (error) {
        if (current === generation)
            notice(errorMessage(error), true);
    }
    finally {
        if (current === generation) {
            busy = false;
            controls();
        }
    }
}
async function upload(channel) {
    const input = element(`${channel}-file`);
    const file = input.files?.[0];
    if (!file) {
        notice("Choose a JSON file to import first.", true);
        return;
    }
    if (file.size === 0 || file.size > 4 * 1024 * 1024) {
        notice("Choose a nonempty JSON file no larger than 4 MiB.", true);
        return;
    }
    const current = generation;
    statusRevision += 1;
    busy = true;
    controls();
    try {
        const body = await file.text();
        const data = JSON.parse(body);
        if (typeof data !== "object" || data === null ||
            data.channel !== channel ||
            data.schema_version !== 1) {
            throw new Error(`Choose a Wodex ${channel} export with schema_version 1.`);
        }
        if (current !== generation)
            return;
        notice("Sending import to your Switch…");
        await request(`/api/${channel}`, "POST", body);
        if (current !== generation)
            return;
        pending = true;
        statusRevision += 1;
        input.value = "";
        notice("File received. Review and confirm the replacement on your Switch.");
    }
    catch (error) {
        if (current === generation)
            notice(error instanceof SyntaxError ? "The file is not valid JSON." : errorMessage(error), true);
    }
    finally {
        if (current === generation) {
            busy = false;
            controls();
        }
    }
}
document.querySelectorAll(".download, .upload").forEach(button => {
    button.addEventListener("click", () => {
        const channel = button.dataset.channel;
        if (busy || pending || !pairingCode || (channel !== "config" && channel !== "sessions"))
            return;
        void (button.classList.contains("download") ? download(channel) : upload(channel));
    });
});
window.addEventListener("pagehide", () => disconnect("Disconnected."));
configEditor.clear();
controls();
