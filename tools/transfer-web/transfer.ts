// Wodex local transfer client. Credentials stay in this page's memory only.
type Channel = "config" | "sessions";
type TransferStatus = { active: boolean; pending: boolean; message: string };

function element<T extends HTMLElement>(id: string): T {
  const result = document.getElementById(id);
  if (!result) throw new Error("Missing transfer page control.");
  return result as T;
}

const pairForm = element<HTMLFormElement>("pair-form");
const pairCode = element<HTMLInputElement>("pair-code");
const showCode = element<HTMLInputElement>("show-code");
const channels = element("channels");
const connection = element("connection");
const statusText = element("status");
const connectButton = element<HTMLButtonElement>("connect");
let pairingCode = "";
let pollTimer: number | undefined;
let busy = false;
let pending = false;
let generation = 0;
let statusRevision = 0;
const requests = new Set<AbortController>();
const configEditor = new ConfigurationEditor(controls);
let awaitingEditor = false;

function notice(message: string, error = false): void {
  statusText.textContent = message;
  statusText.dataset.error = String(error);
}

function controls(): void {
  document.querySelectorAll<HTMLButtonElement>(".download, .upload").forEach(button => {
    button.disabled = busy || pending || !pairingCode;
  });
  document.querySelectorAll<HTMLInputElement>('input[type="file"]').forEach(input => {
    input.disabled = busy || pending || !pairingCode;
  });
  connectButton.disabled = busy;
  configEditor.controls(Boolean(pairingCode), busy || pending);
}

function disconnect(message: string, error = false): void {
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
  for (const request of requests) request.abort();
  requests.clear();
  channels.hidden = true;
  connection.hidden = true;
  pairForm.hidden = false;
  document.querySelectorAll<HTMLInputElement>('input[type="file"]').forEach(input => { input.value = ""; });
  controls();
  notice(message, error);
}

async function request(path: string, method = "GET", body?: string): Promise<Response> {
  const controller = new AbortController();
  requests.add(controller);
  const timeout = window.setTimeout(() => controller.abort(), 20_000);
  try {
    const headers: Record<string, string> = { Authorization: `Bearer ${pairingCode}` };
    if (body !== undefined) headers["Content-Type"] = "application/json";
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
      if (complete.status === 401) throw new Error("Pairing code rejected. Enter the current code shown in Wodex.");
      if (complete.status === 409) throw new Error("An import is already waiting for confirmation on your Switch.");
      if (complete.status === 413) throw new Error("The file is too large. The limit is 4 MiB.");
      throw new Error(`Wodex could not complete this transfer (HTTP ${complete.status}).`);
    }
    return complete;
  } finally {
    window.clearTimeout(timeout);
    requests.delete(controller);
  }
}

function readStatus(value: unknown): TransferStatus {
  if (typeof value !== "object" || value === null) throw new Error("Invalid response from Wodex.");
  const status = value as Partial<TransferStatus>;
  if (typeof status.active !== "boolean" || typeof status.pending !== "boolean" || typeof status.message !== "string") {
    throw new Error("Invalid response from Wodex.");
  }
  return status as TransferStatus;
}

function errorMessage(error: unknown): string {
  // JSON parser errors can quote response contents, including an API key.
  if (error instanceof SyntaxError) return "Wodex returned an invalid response. Reload and try again.";
  return error instanceof Error && error.name !== "AbortError"
    ? error.message : "Connection interrupted. Keep Wodex open and check your local network.";
}

// Callers hold the busy flag so a load cannot overwrite edits made in flight.
async function loadConfiguration(message = "Showing saved configuration. Edit it below.", preserveDraft = false): Promise<void> {
  const current = generation;
  configEditor.notice("Loading saved configuration…");
  try {
    const value: unknown = await (await request("/api/config")).json();
    if (current !== generation) return;
    const document = readConfiguration(value);
    if (preserveDraft) configEditor.reconcile(document, message);
    else {
      configEditor.load(document);
      configEditor.notice(message);
    }
  } catch (error) {
    if (current === generation) {
      configEditor.requireReload(`${errorMessage(error)} Use Reload saved configuration to retry.`);
    }
  }
}

async function poll(): Promise<void> {
  const current = generation;
  const revision = statusRevision;
  try {
    if (!pairingCode || busy) return;
    const status = readStatus(await (await request("/api/status")).json());
    if (current !== generation || revision !== statusRevision) return;
    if (!status.active) {
      disconnect("Transfer stopped. Start a new transfer in Wodex to reconnect.");
      return;
    }
    const changed = pending !== status.pending;
    pending = status.pending;
    if (pending || changed) notice(status.message || "Confirm or reject the import on your Switch.");
    if (pending) {
      configEditor.notice("An import is waiting. Confirm or reject it on your Switch before editing.");
    } else if (changed) {
      const preserveDraft = !awaitingEditor;
      awaitingEditor = false;
      busy = true;
      statusRevision += 1;
      controls();
      await loadConfiguration(`${status.message || "Import finished."} Showing saved configuration.`, preserveDraft);
      if (current !== generation) return;
      busy = false;
    }
    controls();
  } catch (error) {
    if (current === generation && revision === statusRevision) disconnect(errorMessage(error), true);
  } finally {
    if (current === generation && pairingCode) pollTimer = window.setTimeout(poll, 1500);
  }
}

pairForm.addEventListener("submit", async event => {
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
    if (current !== generation) return;
    if (!status.active) throw new Error("Start PC transfer in Wodex before connecting.");
    pending = status.pending;
    pairForm.hidden = true;
    channels.hidden = false;
    connection.hidden = false;
    notice(pending ? status.message : "Connected. Edit configuration or transfer a backup below.");
    await loadConfiguration(pending ? "An import is waiting for confirmation on your Switch." : undefined);
    if (current !== generation) return;
    pollTimer = window.setTimeout(poll, 1500);
  } catch (error) {
    if (current === generation) disconnect(errorMessage(error), true);
  } finally {
    if (current === generation) { busy = false; controls(); }
  }
});

element("disconnect").addEventListener("click", () => disconnect("Disconnected from this browser. Stop transfer in Wodex when finished."));
showCode.addEventListener("change", () => { pairCode.type = showCode.checked ? "text" : "password"; });

configEditor.reload.addEventListener("click", async () => {
  if (busy || pending || !pairingCode) return;
  if (configEditor.dirty() && !window.confirm("Discard your unsent changes and reload the saved configuration from Wodex?")) return;
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

configEditor.form.addEventListener("submit", async event => {
  event.preventDefault();
  if (busy || pending || !pairingCode || !configEditor.dirty()) return;
  let body: string;
  try {
    body = JSON.stringify(configEditor.document());
  } catch (error) {
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
    if (current !== generation) return;
    if (response.status !== 202) throw new Error("Wodex did not acknowledge the configuration change.");
    awaitingEditor = true;
    pending = true;
    statusRevision += 1;
    const message = "Changes sent. Confirm replacement of provider profiles and preferences on your Switch to save them, or reject to keep the previous settings.";
    configEditor.notice(message);
    notice(message);
  } catch (error) {
    if (current === generation) {
      configEditor.requireReload(`${errorMessage(error)} Check your Switch, then reload saved configuration before trying again.`);
    }
  } finally {
    if (current === generation) { busy = false; controls(); }
  }
});

async function download(channel: Channel): Promise<void> {
  const current = generation;
  statusRevision += 1;
  busy = true;
  controls();
  notice(`Downloading ${channel === "config" ? "configuration" : "sessions"}…`);
  try {
    const response = await request(`/api/${channel}`);
    const blob = await response.blob();
    if (current !== generation) return;
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
  } catch (error) {
    if (current === generation) notice(errorMessage(error), true);
  } finally {
    if (current === generation) { busy = false; controls(); }
  }
}

async function upload(channel: Channel): Promise<void> {
  const input = element<HTMLInputElement>(`${channel}-file`);
  const file = input.files?.[0];
  if (!file) { notice("Choose a JSON file to import first.", true); return; }
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
    const data: unknown = JSON.parse(body);
    if (typeof data !== "object" || data === null ||
        (data as { channel?: unknown }).channel !== channel ||
        (data as { schema_version?: unknown }).schema_version !== 1) {
      throw new Error(`Choose a Wodex ${channel} export with schema_version 1.`);
    }
    if (current !== generation) return;
    notice("Sending import to your Switch…");
    await request(`/api/${channel}`, "POST", body);
    if (current !== generation) return;
    pending = true;
    statusRevision += 1;
    input.value = "";
    notice("File received. Review and confirm the replacement on your Switch.");
  } catch (error) {
    if (current === generation) notice(error instanceof SyntaxError ? "The file is not valid JSON." : errorMessage(error), true);
  } finally {
    if (current === generation) { busy = false; controls(); }
  }
}

document.querySelectorAll<HTMLButtonElement>(".download, .upload").forEach(button => {
  button.addEventListener("click", () => {
    const channel = button.dataset.channel;
    if (busy || pending || !pairingCode || (channel !== "config" && channel !== "sessions")) return;
    void (button.classList.contains("download") ? download(channel) : upload(channel));
  });
});
window.addEventListener("pagehide", () => disconnect("Disconnected."));
configEditor.clear();
controls();
