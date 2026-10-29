// The editor handles saved configuration. The transfer controller owns
// authentication, requests and Switch-side confirmation.
const CONFIGURATION_PROVIDERS = [
  "deepseek", "openai", "opencode_go", "llama_cpp", "zai", "zai_coding_plan",
  "zhipuai", "zhipuai_coding_plan", "openai_compatible"
] as const;
type ConfigurationProvider = typeof CONFIGURATION_PROVIDERS[number];
type ProviderProfile = {
  token_limit_field: "max_tokens" | "max_completion_tokens";
  api_key: string;
  base_url: string;
  model: string;
  thinking: boolean;
  max_tokens: number;
  system_prompt: string;
  allow_insecure_http: boolean;
};
type ProviderProfiles = Partial<Record<ConfigurationProvider, ProviderProfile>>;
type ConfigurationSettings = ProviderProfile & {
  provider: ConfigurationProvider;
  provider_profiles: ProviderProfiles;
  update_repository: string;
  update_channel: "version" | "nightly";
};
type ConfigurationDocument = {
  schema_version: 1;
  channel: "config";
  settings: ConfigurationSettings;
};
type WireProfile = Omit<ProviderProfile, "token_limit_field" | "allow_insecure_http"> &
  Partial<Pick<ProviderProfile, "token_limit_field" | "allow_insecure_http">>;
type ConfigurationWireDocument = Omit<ConfigurationDocument, "settings"> & {
  settings: WireProfile & Pick<ConfigurationSettings, "update_repository" | "update_channel"> & {
    provider?: ConfigurationProvider;
    provider_profiles?: Partial<Record<ConfigurationProvider, WireProfile>>;
  };
};

function isConfigurationProvider(value: unknown): value is ConfigurationProvider {
  return typeof value === "string" && CONFIGURATION_PROVIDERS.some(provider => provider === value);
}

function defaultProviderProfile(provider: ConfigurationProvider): ProviderProfile {
  const base_urls: Record<ConfigurationProvider, string> = {
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

const PROVIDER_PROFILE_KEYS = Object.keys(defaultProviderProfile("deepseek")) as (keyof ProviderProfile)[];

function recordValue(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readProviderProfile(value: unknown, extra_keys: readonly string[] = []): ProviderProfile {
  if (!recordValue(value) ||
      typeof value.api_key !== "string" || typeof value.base_url !== "string" ||
      typeof value.model !== "string" || typeof value.thinking !== "boolean" ||
      typeof value.max_tokens !== "number" || typeof value.system_prompt !== "string") {
    throw new Error("Unsupported provider profile from Wodex. Reload using a matching app version.");
  }
  const keys: readonly string[] = [...PROVIDER_PROFILE_KEYS, ...extra_keys];
  if (Object.keys(value).some(key => !keys.includes(key))) {
    throw new Error("Unsupported provider profile fields from Wodex. Reload using a matching app version.");
  }
  return {
    token_limit_field: (value.token_limit_field === undefined ? "max_tokens" : value.token_limit_field) as ProviderProfile["token_limit_field"],
    api_key: value.api_key,
    base_url: value.base_url,
    model: value.model,
    thinking: value.thinking,
    max_tokens: value.max_tokens,
    system_prompt: value.system_prompt,
    allow_insecure_http: (value.allow_insecure_http === undefined ? false : value.allow_insecure_http) as boolean
  };
}

function readConfiguration(value: unknown): ConfigurationDocument {
  if (!recordValue(value) || value.schema_version !== 1 || value.channel !== "config" || !recordValue(value.settings)) {
    throw new Error("Unsupported configuration from Wodex. Reload using a matching app version.");
  }
  const saved = value.settings;
  const provider = saved.provider === undefined ? "deepseek" : saved.provider;
  if (!isConfigurationProvider(provider) || typeof saved.update_repository !== "string" ||
      (saved.update_channel !== "version" && saved.update_channel !== "nightly")) {
    throw new Error("Unsupported configuration from Wodex. Reload using a matching app version.");
  }
  const provider_profiles: ProviderProfiles = {};
  if (saved.provider_profiles !== undefined) {
    if (!recordValue(saved.provider_profiles)) throw new Error("Invalid saved provider profiles from Wodex.");
    for (const [id, profile] of Object.entries(saved.provider_profiles)) {
      if (!isConfigurationProvider(id) || id === provider) throw new Error("Invalid saved provider profiles from Wodex.");
      provider_profiles[id] = readProviderProfile(profile);
    }
  }
  const settings: ConfigurationSettings = {
    ...readProviderProfile(saved, ["provider", "provider_profiles", "update_repository", "update_channel"]),
    provider,
    provider_profiles,
    update_repository: saved.update_repository,
    update_channel: saved.update_channel
  };
  validateConfiguration(settings);
  return { schema_version: 1, channel: "config", settings };
}

function privateLanHost(host: string): boolean {
  if (host.toLowerCase().endsWith(".local")) return true;
  const parts = host.split(".");
  if (parts.length !== 4 || parts.some(part => !/^(0|[1-9][0-9]{0,2})$/.test(part) || Number(part) > 255)) return false;
  const octets = parts.map(Number);
  return octets[0] === 10 || (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) ||
    (octets[0] === 192 && octets[1] === 168);
}

function validateProviderProfile(provider: ConfigurationProvider, profile: ProviderProfile): void {
  const bytes = (text: string): number => new TextEncoder().encode(text).length;
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

function validateConfiguration(settings: ConfigurationSettings): void {
  if (!isConfigurationProvider(settings.provider)) throw new Error("Choose a supported provider.");
  validateProviderProfile(settings.provider, settings);
  for (const [id, profile] of Object.entries(settings.provider_profiles)) {
    if (!isConfigurationProvider(id) || id === settings.provider || !profile) throw new Error("Invalid saved provider profiles.");
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

function copyProfiles(profiles: ProviderProfiles): ProviderProfiles {
  const copied: ProviderProfiles = {};
  for (const provider of CONFIGURATION_PROVIDERS) {
    const profile = profiles[provider];
    if (profile) copied[provider] = { ...profile };
  }
  return copied;
}

function sameProfile(first: ProviderProfile | undefined, second: ProviderProfile | undefined): boolean {
  if (!first || !second) return first === second;
  return PROVIDER_PROFILE_KEYS.every(key => first[key] === second[key]);
}

function sameConfiguration(first: ConfigurationSettings, second: ConfigurationSettings): boolean {
  return first.provider === second.provider && first.update_repository === second.update_repository &&
    first.update_channel === second.update_channel && sameProfile(first, second) &&
    CONFIGURATION_PROVIDERS.every(provider => sameProfile(first.provider_profiles[provider], second.provider_profiles[provider]));
}

function profileWire(profile: ProviderProfile): WireProfile {
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
  readonly form = element<HTMLFormElement>("config-form");
  readonly reload = element<HTMLButtonElement>("config-reload");
  private readonly fields = element<HTMLFieldSetElement>("config-fields");
  private readonly save = element<HTMLButtonElement>("config-save");
  private readonly status = element("config-status");
  private readonly provider = element<HTMLSelectElement>("config-provider");
  private readonly apiKey = element<HTMLInputElement>("config-api-key");
  private readonly keyHint = element("api-key-hint");
  private readonly showKey = element<HTMLInputElement>("show-api-key");
  private readonly model = element<HTMLInputElement>("config-model");
  private readonly prompt = element<HTMLTextAreaElement>("config-system-prompt");
  private readonly endpoint = element<HTMLInputElement>("config-base-url");
  private readonly thinking = element<HTMLInputElement>("config-thinking");
  private readonly thinkingField = element("config-thinking-field");
  private readonly tokenLimitField = element<HTMLSelectElement>("config-token-limit-field");
  private readonly tokenLimitChoice = element("config-token-limit-choice");
  private readonly insecureHttp = element<HTMLInputElement>("config-allow-insecure-http");
  private readonly insecureHttpField = element("config-insecure-http-field");
  private readonly maxTokens = element<HTMLInputElement>("config-max-tokens");
  private readonly repository = element<HTMLInputElement>("config-update-repository");
  private readonly channel = element<HTMLSelectElement>("config-update-channel");
  private baseline: ConfigurationDocument | null = null;
  private profiles: ProviderProfiles = {};
  private stale = false;
  private selectedProvider: ConfigurationProvider = "deepseek";

  constructor(onChange: () => void) {
    const changed = (event: Event): void => {
      // A select emits input then change. Switch once and keep its notice.
      if (event.target === this.provider && event.type === "input") return;
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

  notice(message: string, error = false): void {
    this.status.textContent = message;
    this.status.dataset.error = String(error);
  }

  clear(): void {
    this.baseline = null;
    this.profiles = {};
    this.stale = false;
    for (const input of [this.apiKey, this.model, this.prompt, this.endpoint, this.maxTokens, this.repository]) input.value = "";
    this.channel.value = "version";
    this.provider.value = this.selectedProvider = "deepseek";
    this.tokenLimitField.value = "max_tokens";
    this.thinking.checked = this.insecureHttp.checked = false;
    this.providerControls();
    this.maskKey();
    this.notice("Connect to load your saved configuration.");
  }

  load(document: ConfigurationDocument): void {
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

  private loadProfile(profile: ProviderProfile): void {
    this.tokenLimitField.value = profile.token_limit_field;
    this.apiKey.value = profile.api_key;
    this.model.value = profile.model;
    this.prompt.value = profile.system_prompt;
    this.endpoint.value = profile.base_url;
    this.thinking.checked = profile.thinking;
    this.maxTokens.value = String(profile.max_tokens);
    this.insecureHttp.checked = profile.allow_insecure_http;
  }

  private switchProvider(): boolean {
    const provider = this.provider.value;
    if (!isConfigurationProvider(provider) || provider === this.selectedProvider) return false;
    this.profiles[this.selectedProvider] = this.profileValues();
    const profile = this.profiles[provider] || defaultProviderProfile(provider);
    delete this.profiles[provider];
    this.selectedProvider = provider;
    this.loadProfile(profile);
    this.providerControls();
    this.maskKey();
    return true;
  }

  private providerControls(): void {
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

  reconcile(document: ConfigurationDocument, message: string): void {
    if (!this.dirty()) {
      this.load(document);
      this.notice(message);
      return;
    }
    if (!sameConfiguration(this.baseline!.settings, document.settings)) {
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

  private profileValues(): ProviderProfile {
    return {
      token_limit_field: this.tokenLimitField.value as ProviderProfile["token_limit_field"],
      api_key: this.apiKey.value,
      base_url: this.endpoint.value,
      model: this.model.value,
      thinking: this.thinking.checked,
      max_tokens: /^[0-9]+$/.test(this.maxTokens.value) ? Number(this.maxTokens.value) : NaN,
      system_prompt: this.prompt.value,
      allow_insecure_http: this.insecureHttp.checked
    };
  }

  private values(): ConfigurationSettings {
    return {
      ...this.profileValues(),
      provider: this.provider.value as ConfigurationProvider,
      provider_profiles: copyProfiles(this.profiles),
      update_repository: this.repository.value,
      update_channel: this.channel.value as ConfigurationSettings["update_channel"]
    };
  }

  dirty(): boolean {
    return this.baseline !== null && !sameConfiguration(this.values(), this.baseline.settings);
  }

  document(): ConfigurationWireDocument {
    if (!this.baseline || this.stale) throw new Error("Reload saved configuration before sending changes.");
    const settings = this.values();
    validateConfiguration(settings);
    const provider_profiles: Partial<Record<ConfigurationProvider, WireProfile>> = {};
    for (const provider of CONFIGURATION_PROVIDERS) {
      const profile = settings.provider_profiles[provider];
      if (profile) provider_profiles[provider] = profileWire(profile);
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
