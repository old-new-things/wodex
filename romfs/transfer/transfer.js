"use strict";
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
    return error instanceof Error && error.name !== "AbortError"
        ? error.message : "Connection interrupted. Keep Wodex open and check your local network.";
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
        notice(pending ? status.message : "Connected. Choose configuration or sessions below.");
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
            ? "Configuration downloaded, including the saved API key. Keep this file private."
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
controls();
