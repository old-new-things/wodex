import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createContext, runInContext } from "node:vm";

type Listener = (event: FakeEvent) => unknown;
type FakeEvent = { type: string; target: Element; currentTarget: Element; preventDefault(): void };

export class Element {
  id = "";
  tagName: string;
  type = "";
  value = "";
  checked = false;
  hidden = false;
  disabled = false;
  textContent = "";
  dataset: Record<string, string> = {};
  attributes: Record<string, string> = {};
  children: Element[] = [];
  parentElement: Element | null = null;
  files: File[] = [];
  className = "";
  href = "";
  download = "";
  listeners = new Map<string, Listener[]>();
  removed = false;
  validityMessage = "";

  constructor(tag_name: string, attributes: Record<string, string> = {}) {
    this.tagName = tag_name.toUpperCase();
    this.attributes = attributes;
    this.id = attributes.id || "";
    this.type = attributes.type || "";
    this.value = attributes.value || "";
    this.checked = "checked" in attributes;
    this.hidden = "hidden" in attributes;
    this.disabled = "disabled" in attributes;
    this.className = attributes.class || "";
    for (const [name, value] of Object.entries(attributes)) {
      if (name.startsWith("data-")) {
        this.dataset[name.slice(5).replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase())] = value;
      }
    }
  }

  get classList(): { contains(value: string): boolean; add(value: string): void; remove(value: string): void } {
    return {
      contains: value => this.className.split(/\s+/).includes(value),
      add: value => { this.className += " " + value; },
      remove: value => { this.className = this.className.split(/\s+/).filter(item => item !== value).join(" "); }
    };
  }
  get valueAsNumber(): number { return Number(this.value); }
  get elements(): Element[] { return this.querySelectorAll("input, textarea, select, button"); }

  addEventListener(name: string, listener: Listener): void {
    this.listeners.set(name, [...(this.listeners.get(name) || []), listener]);
  }

  async dispatch(name: string): Promise<void> {
    const results: unknown[] = [];
    let current: Element | null = this;
    const event: FakeEvent = { type: name, target: this, currentTarget: this, preventDefault() {} };
    while (current) {
      event.currentTarget = current;
      for (const listener of current.listeners.get(name) || []) results.push(listener(event));
      current = current.parentElement;
    }
    await Promise.all(results);
  }

  append(child: Element): void { child.parentElement = this; this.children.push(child); }
  remove(): void { this.removed = true; }
  click(): void { void this.dispatch("click"); }
  focus(): void {}
  setCustomValidity(message: string): void { this.validityMessage = message; }
  getAttribute(name: string): string | null { return this.attributes[name] ?? null; }
  setAttribute(name: string, value: string): void { this.attributes[name] = value; }
  removeAttribute(name: string): void { delete this.attributes[name]; }

  checkValidity(): boolean {
    if (this.disabled) return true;
    if (this.validityMessage) return false;
    if ("required" in this.attributes && !this.value) return false;
    if (this.type === "number") {
      const value = Number(this.value);
      if (!this.value || !Number.isFinite(value)) return false;
      if ("min" in this.attributes && value < Number(this.attributes.min)) return false;
      if ("max" in this.attributes && value > Number(this.attributes.max)) return false;
      if (this.attributes.step !== "any" && !Number.isInteger(value)) return false;
    }
    return this.children.every(child => child.checkValidity());
  }
  reportValidity(): boolean { return this.checkValidity(); }
  querySelectorAll(selector: string): Element[] {
    return this.children.flatMap(child => [
      ...(selector.split(",").some(part => matches(child, part.trim())) ? [child] : []),
      ...child.querySelectorAll(selector)
    ]);
  }
  querySelector(selector: string): Element | null { return this.querySelectorAll(selector)[0] || null; }
}

function matches(element: Element, selector: string): boolean {
  if (selector.startsWith("#")) return element.id === selector.slice(1);
  if (selector.startsWith(".")) return element.classList.contains(selector.slice(1));
  const tagged = /^([a-z]+)(?:\[([\w-]+)="([^"]*)"\])?$/.exec(selector);
  if (!tagged || element.tagName.toLowerCase() !== tagged[1]) return false;
  return !tagged[2] || element.attributes[tagged[2]] === tagged[3];
}

function readDocument(html: string): Element {
  const document = new Element("document");
  const stack = [document];
  const void_tags = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"]);
  for (const token of html.matchAll(/<(\/?)([a-zA-Z][\w-]*)\b([^>]*)>/g)) {
    const [, closing, tag_name, content] = token;
    if (closing) {
      while (stack.length > 1 && stack.at(-1)?.tagName.toLowerCase() !== tag_name.toLowerCase()) stack.pop();
      if (stack.length > 1) stack.pop();
      continue;
    }
    const attributes: Record<string, string> = {};
    for (const attribute of content.matchAll(/([\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'))?/g)) {
      attributes[attribute[1]] = attribute[2] ?? attribute[3] ?? "";
    }
    const element = new Element(tag_name, attributes);
    stack.at(-1)?.append(element);
    if (!void_tags.has(tag_name.toLowerCase())) stack.push(element);
  }
  return document;
}

export type FetchCall = {
  path: string;
  method: string;
  headers: Headers;
  body: string | undefined;
  signal: AbortSignal;
  answered: boolean;
  reply(value: unknown, status?: number): void;
  replyRaw(value: string, status?: number): void;
  fail(error: Error): void;
};

type Timer = { callback: () => unknown; delay: number };

export class Harness {
  readonly document: Element;
  readonly calls: FetchCall[] = [];
  readonly timers = new Map<number, Timer>();
  readonly confirmations: string[] = [];
  readonly confirmationAnswers: boolean[] = [];
  readonly downloads: Element[] = [];
  readonly windowListeners = new Map<string, (() => unknown)[]>();
  private nextTimer = 1;

  constructor() {
    const repository = resolve(__dirname, "../../");
    this.document = readDocument(readFileSync(resolve(repository, "romfs/transfer/index.html"), "utf8"));
    const body = this.document.querySelector("body");
    assert.ok(body, "Committed browser HTML has a body.");
    const original_append = body.append.bind(body);
    body.append = element => { original_append(element); this.downloads.push(element); };
    const fake_document = {
      getElementById: (id: string) => this.document.querySelector("#" + id),
      querySelectorAll: (selector: string) => this.document.querySelectorAll(selector),
      querySelector: (selector: string) => this.document.querySelector(selector),
      createElement: (name: string) => new Element(name),
      body
    };
    const confirm = (message: string): boolean => {
      this.confirmations.push(message);
      assert.ok(this.confirmationAnswers.length, "Unexpected browser confirmation: " + message);
      return this.confirmationAnswers.shift()!;
    };
    const window = {
      setTimeout: (callback: () => unknown, delay: number): number => {
        const id = this.nextTimer++;
        this.timers.set(id, { callback, delay });
        return id;
      },
      clearTimeout: (id: number): void => { this.timers.delete(id); },
      addEventListener: (name: string, callback: () => unknown): void => {
        this.windowListeners.set(name, [...(this.windowListeners.get(name) || []), callback]);
      },
      confirm
    };
    const fetch = (path: string, init: RequestInit): Promise<Response> => {
      assert.match(path, /^\/api\/(config|sessions|status)$/, "Only local transfer endpoints are requested.");
      assert.equal(init.cache, "no-store");
      assert.equal(init.credentials, "omit");
      assert.equal(init.redirect, "error");
      return new Promise((resolve_response, reject_response) => {
        this.calls.push({
          path,
          method: init.method || "GET",
          headers: new Headers(init.headers),
          body: init.body as string | undefined,
          signal: init.signal!,
          answered: false,
          reply: (value, status = 200) => resolve_response(new Response(JSON.stringify(value), {
            status, headers: { "Content-Type": "application/json" }
          })),
          replyRaw: (value, status = 200) => resolve_response(new Response(value, {
            status, headers: { "Content-Type": "application/json" }
          })),
          fail: reject_response
        });
      });
    };
    class ObjectUrl extends URL {
      static createObjectURL(): string { return "blob:local-test-download"; }
      static revokeObjectURL(): void {}
    }
    const context = createContext({
      document: fake_document, window, fetch, confirm,
      URL: ObjectUrl, Response, Headers, Blob, AbortController, DOMException, TextEncoder, TextDecoder,
      // Fetch/Response and script parsing share the window's error constructors
      // in a browser. Keep the VM's JSON and errors in that same test realm.
      JSON, Error, SyntaxError,
      HTMLElement: Element, HTMLInputElement: Element, HTMLFormElement: Element,
      console
    });
    runInContext(readFileSync(resolve(repository, "romfs/transfer/transfer.js"), "utf8"), context, {
      filename: "transfer.js"
    });
  }

  element(id: string): Element {
    const element = this.document.querySelector("#" + id);
    assert.ok(element, "Missing committed HTML element #" + id);
    return element;
  }
  async edit(id: string, value: string | boolean): Promise<void> {
    const element = this.element(id);
    if (typeof value === "boolean") element.checked = value;
    else element.value = value;
    await element.dispatch(typeof value === "boolean" || element.tagName === "SELECT" ? "change" : "input");
    await this.flush();
  }
  async flush(): Promise<void> {
    for (let index = 0; index < 3; index += 1) await new Promise<void>(resolve => setImmediate(resolve));
  }
  async nextCall(path: string, method = "GET"): Promise<FetchCall> {
    for (let index = 0; index < 12; index += 1) {
      const call = this.calls.find(call => !call.answered && call.path === path && call.method === method);
      if (call) { call.answered = true; return call; }
      await this.flush();
    }
    assert.fail("Expected " + method + " " + path + "; saw " + this.calls.map(call => call.method + " " + call.path).join(", "));
  }
  async runPoll(): Promise<void> {
    const entry = [...this.timers].find(([, timer]) => timer.delay === 1500);
    assert.ok(entry, "A status poll is scheduled.");
    this.timers.delete(entry[0]);
    void entry[1].callback();
    await this.flush();
  }
  async pagehide(): Promise<void> {
    for (const listener of this.windowListeners.get("pagehide") || []) listener();
    await this.flush();
  }
}

export const PAIR_CODE = "0123456789abcdef0123456789abcdef";
export const CONFIG = {
  schema_version: 1,
  channel: "config",
  settings: {
    api_key: "saved-test-api-key",
    model: "deepseek-custom",
    system_prompt: "Keep 中文 and **Markdown** intact.",
    base_url: "https://example.invalid/v1",
    thinking: true,
    max_tokens: 8192,
    update_repository: "old-new-things/wodex",
    update_channel: "nightly"
  }
};

export function status(pending = false, message = "Ready"): object {
  return { active: true, pending, message };
}

export async function pair(app: Harness, config: unknown = CONFIG): Promise<void> {
  app.element("pair-code").value = PAIR_CODE;
  const submitted = app.element("pair-form").dispatch("submit");
  const probe = await app.nextCall("/api/status");
  assert.equal(probe.headers.get("Authorization"), "Bearer " + PAIR_CODE);
  probe.reply(status());
  const load = await app.nextCall("/api/config");
  assert.equal(load.headers.get("Authorization"), "Bearer " + PAIR_CODE);
  load.reply(config);
  await submitted;
  await app.flush();
}
