// The editor handles only saved configuration. The transfer controller owns
// authentication, requests and Switch-side confirmation.
type ConfigurationSettings = {
  provider: "deepseek" | "openai_compatible";
  token_limit_field: "max_tokens" | "max_completion_tokens";
  api_key: string;
  base_url: string;
  model: string;
  thinking: boolean;
  max_tokens: number;
  system_prompt: string;
  update_repository: string;
  update_channel: "version" | "nightly";
};
type ConfigurationDocument = {
  schema_version: 1;
  channel: "config";
  settings: ConfigurationSettings;
};
type ConfigurationWireDocument = Omit<ConfigurationDocument, "settings"> & {
  settings: Omit<ConfigurationSettings, "provider" | "token_limit_field"> &
    Partial<Pick<ConfigurationSettings, "provider" | "token_limit_field">>;
};

function readConfiguration(value: unknown): ConfigurationDocument {
  if (typeof value !== "object" || value === null) throw new Error("Invalid configuration from Wodex.");
  const document = value as Partial<ConfigurationWireDocument>;
  const settings = document.settings;
  if (document.schema_version !== 1 || document.channel !== "config" || !settings ||
      typeof settings.api_key !== "string" || typeof settings.base_url !== "string" ||
      typeof settings.model !== "string" || typeof settings.thinking !== "boolean" ||
      typeof settings.max_tokens !== "number" || typeof settings.system_prompt !== "string" ||
      typeof settings.update_repository !== "string" ||
      (settings.update_channel !== "version" && settings.update_channel !== "nightly")) {
    throw new Error("Unsupported configuration from Wodex. Reload using a matching app version.");
  }
  const normalized: ConfigurationSettings = {
    ...settings,
    provider: settings.provider === undefined ? "deepseek" : settings.provider,
    token_limit_field: settings.token_limit_field === undefined ? "max_tokens" : settings.token_limit_field
  };
  validateConfiguration(normalized);
  return { schema_version: 1, channel: "config", settings: normalized };
}

function validateConfiguration(settings: ConfigurationSettings): void {
  const bytes = (text: string): number => new TextEncoder().encode(text).length;
  if (settings.provider !== "deepseek" && settings.provider !== "openai_compatible") {
    throw new Error("Choose DeepSeek or OpenAI-compatible as the provider.");
  }
  if (settings.token_limit_field !== "max_tokens" && settings.token_limit_field !== "max_completion_tokens") {
    throw new Error("Choose max_tokens or max_completion_tokens for the token limit field.");
  }
  if (settings.provider === "deepseek" && settings.token_limit_field !== "max_tokens") {
    throw new Error("DeepSeek uses max_tokens for its response token limit.");
  }
  if (settings.provider === "openai_compatible" && settings.thinking) {
    throw new Error("Thinking mode is available only with the DeepSeek provider.");
  }
  if (bytes(settings.api_key) > 4096 || /[\p{White_Space}\p{Cc}]/u.test(settings.api_key)) {
    throw new Error("The API key must contain no spaces or control characters and be at most 4096 bytes.");
  }
  if (!(settings.provider === "openai_compatible" && settings.model === "") &&
      !/^[A-Za-z0-9_./:-]{1,128}$/.test(settings.model)) {
    throw new Error("Enter a model name of at most 128 ASCII letters, numbers, or _ . / : - characters.");
  }
  if (!Number.isInteger(settings.max_tokens) || settings.max_tokens < 1 || settings.max_tokens > 131072) {
    throw new Error("The response token limit must be a whole number from 1 to 131072.");
  }
  if (bytes(settings.system_prompt) > 16384 || settings.system_prompt.includes("\0")) {
    throw new Error("The system prompt must be at most 16 KiB of UTF-8 text and cannot contain NUL characters.");
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
  // Match Rust's endpoint policy without URL() silently resolving dot segments,
  // percent escapes or uppercase schemes before the user reviews the change.
  const url = settings.base_url.replace(/\/+$/, "");
  const scheme = /^(https?):\/\/(.*)$/.exec(url);
  const remainder = scheme?.[2] || "";
  const authority = remainder.split("/")[0];
  const host = /^(\[::1\]|[A-Za-z0-9.-]+)(?::([0-9]+))?$/.exec(authority);
  if (url.length > 2048 || /[^\x21-\x7e]|[?#@\\"<>%]/.test(url) || !scheme || !host ||
      host[1].startsWith(".") || host[1].endsWith(".") || host[1].includes("..") ||
      (host[2] !== undefined && (Number(host[2]) < 1 || Number(host[2]) > 65535)) ||
      (scheme[1] === "http" && !["localhost", "127.0.0.1", "[::1]"].includes(host[1])) ||
      remainder.includes("//") || remainder.split("/").slice(1).some(part => part === "." || part === "..")) {
    throw new Error("Use an HTTPS endpoint without credentials, query, or fragment. HTTP is allowed only on localhost.");
  }
}

class ConfigurationEditor {
  readonly form = element<HTMLFormElement>("config-form");
  readonly reload = element<HTMLButtonElement>("config-reload");
  private readonly fields = element<HTMLFieldSetElement>("config-fields");
  private readonly save = element<HTMLButtonElement>("config-save");
  private readonly status = element("config-status");
  private readonly provider = element<HTMLSelectElement>("config-provider");
  private readonly apiKey = element<HTMLInputElement>("config-api-key");
  private readonly showKey = element<HTMLInputElement>("show-api-key");
  private readonly model = element<HTMLInputElement>("config-model");
  private readonly prompt = element<HTMLTextAreaElement>("config-system-prompt");
  private readonly endpoint = element<HTMLInputElement>("config-base-url");
  private readonly thinking = element<HTMLInputElement>("config-thinking");
  private readonly thinkingField = element("config-thinking-field");
  private readonly tokenLimitField = element<HTMLSelectElement>("config-token-limit-field");
  private readonly tokenLimitChoice = element("config-token-limit-choice");
  private readonly maxTokens = element<HTMLInputElement>("config-max-tokens");
  private readonly repository = element<HTMLInputElement>("config-update-repository");
  private readonly channel = element<HTMLSelectElement>("config-update-channel");
  private baseline: ConfigurationDocument | null = null;
  private stale = false;
  private selectedProvider: ConfigurationSettings["provider"] = "deepseek";

  constructor(onChange: () => void) {
    const changed = (event: Event): void => {
      // A select fires input followed by change. Apply its reset once, when
      // the selection is committed, and keep the reset notice visible.
      if (event.target === this.provider && event.type === "input") return;
      const switched = event.target === this.provider && this.switchProvider();
      if (this.baseline && !this.stale) {
        this.notice(switched ? "Provider changed. The key was cleared from this draft; endpoint, model and provider options were reset. Review them before sending changes." :
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

  notice(message: string, error = false): void {
    this.status.textContent = message;
    this.status.dataset.error = String(error);
  }

  clear(): void {
    this.baseline = null;
    this.stale = false;
    for (const input of [this.apiKey, this.model, this.prompt, this.endpoint, this.maxTokens, this.repository]) {
      input.value = "";
    }
    this.channel.value = "version";
    this.provider.value = this.selectedProvider = "deepseek";
    this.tokenLimitField.value = "max_tokens";
    this.thinking.checked = false;
    this.providerControls();
    this.maskKey();
    this.notice("Connect to load your saved configuration.");
  }

  load(document: ConfigurationDocument): void {
    this.baseline = document;
    this.stale = false;
    const settings = document.settings;
    this.provider.value = this.selectedProvider = settings.provider;
    this.tokenLimitField.value = settings.token_limit_field;
    this.apiKey.value = settings.api_key;
    this.model.value = settings.model;
    this.prompt.value = settings.system_prompt;
    this.endpoint.value = settings.base_url;
    this.thinking.checked = settings.thinking;
    this.maxTokens.value = String(settings.max_tokens);
    this.repository.value = settings.update_repository;
    this.channel.value = settings.update_channel;
    this.providerControls();
    this.maskKey();
  }

  private switchProvider(): boolean {
    const provider = this.provider.value;
    if ((provider !== "deepseek" && provider !== "openai_compatible") || provider === this.selectedProvider) {
      return false;
    }
    this.selectedProvider = provider;
    this.apiKey.value = "";
    this.endpoint.value = provider === "deepseek" ? "https://api.deepseek.com" : "https://api.openai.com/v1";
    this.model.value = provider === "deepseek" ? "deepseek-flash" : "";
    this.thinking.checked = false;
    this.tokenLimitField.value = "max_tokens";
    this.providerControls();
    this.maskKey();
    return true;
  }

  private providerControls(): void {
    const deepseek = this.provider.value === "deepseek";
    this.thinkingField.hidden = this.thinking.disabled = !deepseek;
    this.tokenLimitChoice.hidden = this.tokenLimitField.disabled = deepseek;
    this.model.required = deepseek;
  }

  reconcile(document: ConfigurationDocument, message: string): void {
    if (!this.dirty()) {
      this.load(document);
      this.notice(message);
      return;
    }
    const previous = this.baseline!.settings;
    const saved = document.settings;
    const keys = new Set([...Object.keys(previous), ...Object.keys(saved)]) as Set<keyof ConfigurationSettings>;
    if ([...keys].some(key => previous[key] !== saved[key])) {
      this.requireReload("Saved configuration changed while you had unsent edits. Your draft is kept here; reload saved configuration before sending it.");
    } else {
      this.stale = false;
      this.notice("Saved configuration is unchanged. Your unsent edits are kept here.");
    }
  }

  private maskKey(): void {
    this.showKey.checked = false;
    this.apiKey.type = "password";
  }

  private values(): ConfigurationSettings {
    return {
      provider: this.provider.value as ConfigurationSettings["provider"],
      token_limit_field: this.tokenLimitField.value as ConfigurationSettings["token_limit_field"],
      api_key: this.apiKey.value,
      base_url: this.endpoint.value,
      model: this.model.value,
      thinking: this.thinking.checked,
      max_tokens: Number(this.maxTokens.value),
      system_prompt: this.prompt.value,
      update_repository: this.repository.value,
      update_channel: this.channel.value as ConfigurationSettings["update_channel"]
    };
  }

  dirty(): boolean {
    if (!this.baseline) return false;
    const values = this.values();
    return (Object.keys(values) as (keyof ConfigurationSettings)[])
      .some(key => values[key] !== this.baseline!.settings[key]);
  }

  document(): ConfigurationWireDocument {
    if (!this.baseline || this.stale) throw new Error("Reload saved configuration before sending changes.");
    const settings = this.values();
    if (!/^[0-9]+$/.test(this.maxTokens.value)) {
      throw new Error("The response token limit must be a whole number from 1 to 131072.");
    }
    validateConfiguration(settings);
    const { provider, token_limit_field, ...shared } = { ...this.baseline.settings, ...settings };
    // Keep default-provider documents readable by older Wodex versions.
    return {
      schema_version: 1,
      channel: "config",
      settings: {
        ...shared,
        ...(provider === "deepseek" ? {} : { provider }),
        ...(token_limit_field === "max_tokens" ? {} : { token_limit_field })
      }
    };
  }

  requireReload(message: string): void {
    this.stale = true;
    this.maskKey();
    this.notice(message, true);
  }

  controls(connected: boolean, blocked: boolean): void {
    this.fields.disabled = blocked || !connected || !this.baseline || this.stale;
    this.save.disabled = this.fields.disabled || !this.dirty();
    this.reload.disabled = blocked || !connected;
  }
}
