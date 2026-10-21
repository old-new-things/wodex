# PC import and export

Current nightlies support two independent JSON transfer channels: **Configuration** and **Sessions**. Transfer runs directly between Wodex and your PC browser over your local network. It uses no external server or cloud service. The base app version remains **0.2.1**; this feature does not change the existing fixed version download or create a GitHub Release.

## Connect and transfer

1. Connect the Switch and PC to the same local network. Finish any response, app update or HOME Menu installation.
2. Open **Settings → PC transfer → Start PC transfer** in Wodex.
3. Open the displayed `http://` address, including its port, in your PC browser. Enter the complete 32-character pairing code shown on the Switch. Spaces and hyphens are ignored.
4. Your saved configuration loads into **Edit on your PC**. Change the fields and choose **Send changes to Switch**. Review the replacement on the Switch, then choose **Confirm replacement** or **Reject import**.
5. For file backups, choose **Download configuration** or **Download sessions** to save `wodex-config.json` or `wodex-sessions.json`. To import a backup, choose the matching JSON file and select **Send to Switch**; the Switch asks for the same confirmation.
6. Select **Stop transfer**, go back from the transfer page, or exit Wodex when finished. Each start shows the current address and a fresh pairing code; the listener also stops after 15 minutes.

Keep Wodex on the transfer page while connected. On-device chatting and editing, image downloads and update operations stay paused during transfer. Closing or disconnecting the browser clears its pairing state but does not stop the Switch listener; stop it in Wodex as well. Desktop development builds support the same flow; offline demo mode does not start a server.

Transfers use **HTTP without TLS**. Use a trusted local network: the pairing code controls access but does not encrypt traffic. The configuration form loads the **saved provider API key**, and configuration downloads include it. Desktop `DEEPSEEK_API_KEY` and `OPENAI_API_KEY` overrides are never shown in the form, exported or written into the file. After import, Wodex resolves only the selected provider's environment key. Keep downloaded configuration and session files private.

If the page cannot connect, check that the full address is current, both devices can reach one another, and the network does not isolate Wi-Fi clients. Restart transfer after changing networks. Wodex does not configure router port forwarding.

## Edit configuration in the browser

The form provides **Provider**, **Saved API key**, **API endpoint**, **Model** and multiline **Assistant instructions**. The key is masked by default; **Show API key** reveals it. Leaving the key field empty clears the saved key when you send and confirm the change. Choosing a different provider clears and masks the draft key, resets its endpoint/model/options, and keeps the instructions and response limit. Only one provider configuration is stored; export a backup before switching to retain it.

Expand **Advanced settings** to edit DeepSeek thinking mode or the OpenAI-compatible token-limit field, response token limit, update repository and update channel. OpenAI-compatible mode supports `max_tokens` and `max_completion_tokens`; choose the field accepted by the provider/model. Its model starts empty and must be set before chatting. The base URL must include the API path; Wodex appends `/chat/completions`. The browser checks field limits before sending; Wodex performs the authoritative validation before offering confirmation. An empty update repository disables the update source.

Edits stay in the current page until you choose **Send changes to Switch**, and saved configuration changes only after device confirmation. Sessions remain unchanged. **Reload saved configuration** replaces the form with the device's saved values and asks before discarding unsent edits. JSON download and upload remain available alongside the form.

After you confirm or reject a submitted form on the Switch, the editor reloads the settings that are actually saved. If a different import changes configuration while you have an unsent draft, Wodex keeps the draft visible and asks you to reload before sending it. Session imports leave that draft editable. Disconnecting clears the form and its API key; drafts are not stored in browser storage.

## Files and replacement behavior

Configuration contains the provider, saved API key, endpoint, model, provider options, response limit, assistant instructions and update settings. Importing it replaces those settings while preserving every conversation. Imported provider settings determine where future messages and the API key are sent; importing itself makes no provider request.

Sessions contain every conversation, message, reasoning text and the selected conversation. Importing sessions replaces all existing sessions while preserving configuration. It does not merge chats. A response marked `streaming` in a file becomes `interrupted` when restored.

These are portable UTF-8 JSON files. You can edit an export in a PC text editor and import it again. Keep the top-level `schema_version` and `channel` fields, and choose the matching channel in the browser. Transfer files use their own schema; do not upload the complete internal `state.json` save.

A configuration document has this shape:

```json
{
  "schema_version": 1,
  "channel": "config",
  "settings": {
    "api_key": "",
    "base_url": "https://api.deepseek.com",
    "model": "deepseek-flash",
    "thinking": false,
    "max_tokens": 4096,
    "system_prompt": "Give clear, accurate answers.",
    "update_repository": "old-new-things/wodex",
    "update_channel": "version"
  }
}
```

The empty API key above is an example, not a redaction performed by export. Exported files contain the actual saved key. `update_channel` accepts `version` or `nightly`.

The optional `provider` field accepts `deepseek` (default) or `openai_compatible`. The optional `token_limit_field` accepts `max_tokens` (default) or `max_completion_tokens`. Defaults are omitted in exports, keeping existing DeepSeek backups compatible. OpenAI-compatible configuration includes `"provider": "openai_compatible"` and requires `thinking: false`; DeepSeek requires the default token-limit field. Older Wodex builds cannot read the new nondefault fields, so export a DeepSeek configuration and your sessions before downgrading. The existing unreadable-save recovery behavior preserves unsupported saves.

A minimal sessions document contains one empty conversation:

```json
{
  "schema_version": 1,
  "channel": "sessions",
  "conversations": [
    {
      "id": "chat-1",
      "title": "New conversation",
      "messages": []
    }
  ],
  "active_id": "chat-1"
}
```

Each message contains `role` (`user` or `assistant`), `content`, `reasoning_content` and `status` (`complete`, `streaming`, `interrupted` or `error`). Optional `finish_reason` and `usage` fields preserve completion metadata. User messages must be complete and contain no reasoning; assistant messages must follow a user message. Conversation IDs must be unique positive `chat-N` values without leading zeroes, and `active_id` must name an included conversation. Export an existing conversation to see its complete message structure.

Unknown fields, invalid JSON, unsupported schema versions, wrong channels and invalid settings are rejected before saving. Limits apply both to the uploaded file and the resulting combined save:

| Limit | Maximum |
| --- | --- |
| Transfer document and combined saved JSON | 4 MiB each |
| Conversations | 32, with at least one required |
| Messages per conversation | 128 |
| User message | 64 KiB |
| Assistant answer plus reasoning | 512 KiB per message |
| Total message and reasoning text | 2 MiB |
| Saved API key | 4096 bytes, without spaces or control characters |
| Assistant instructions | 16 KiB |
| Response token limit | 1–131072 |

Provider endpoints follow the normal settings rules: HTTPS without credentials, query or fragment; plain HTTP is accepted only for loopback development endpoints. This rule is separate from the local HTTP transfer listener.

## Recovery and verification

Before applying an import, Wodex saves a snapshot of the previous configuration and sessions to a unique `state.json.recovery-N` file beside `state.json`. On Switch this directory is `sdmc:/switch/wodex/`. These recovery files use the full internal save format and include the saved API key. To restore one manually, exit Wodex, keep copies of the current files, and copy the selected recovery file to `state.json`.

The app prepares and validates a candidate before changing live data. If saving reports an error after replacement, Wodex reads the file back: a matching imported save is adopted with a durability warning; an unchanged previous save leaves the import unapplied. If storage cannot be verified, transfer stops and further editing and saving are blocked until restart, preserving the recovery copy.

Host tests exercise the real local socket path, pairing, independent downloads, device confirmation, rejected imports, storage failures and shutdown. Switch cross-compilation is checked, but PC-to-Switch LAN transfer on physical hardware still needs verification.
