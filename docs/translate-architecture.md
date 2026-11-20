# Translate architecture

This document is the engineering deep dive for the on-device translation stack this fork
adds to Serein. For installation and daily operations see [translate-setup.md](translate-setup.md);
for building and releasing the fork see [building.md](building.md).

## System overview

```
┌─────────────────────────────────────────────────────────────┐
│ Serein client (this fork)                                   │
│                                                             │
│  timeline.rs ──render──▶ inline entries (spinner/result)    │
│  lib.rs      ──composer──▶ before-send translation          │
│  local_translate.rs  ◀── one module, zero new deps          │
│   ├─ settings (egui memory, seeded from SQLite)             │
│   ├─ skip_kind() classifier (pure fn, runs before requests) │
│   ├─ bounded pools: INLINE 24 · PENDING 8 · SEND 1          │
│   └─ hand-rolled HTTP/1.1 POST on std::thread               │
└──────────────────────┬──────────────────────────────────────┘
                       │ 127.0.0.1:5050 — localhost only
┌──────────────────────▼──────────────────────────────────────┐
│ LTEngine (separate process, hidden)                          │
│  /translate          { q, source:"auto", target }            │
│  /translate-context  { …, context: ["Name: text", …] }       │
│  gemma3 GGUF on GPU (Vulkan / Metal), no network egress     │
└─────────────────────────────────────────────────────────────┘
```

Nothing in the translation path leaves the machine. The engine binds localhost only,
the client talks raw TCP to it, and there is no telemetry of any kind.

## Components

| Piece | Where | Responsibility |
|---|---|---|
| Feature module | `crates/ui/src/local_translate.rs` | Settings, skip classifier, request pools, HTTP + JSON, settings window, persistence bridge |
| Timeline integration | `crates/ui/src/timeline.rs` | Renders per-message entries: pending spinner, result + target chip, muted skip note, error + fallback link; clears entries when a message is edited |
| Composer integration | `crates/ui/src/lib.rs` | Globe master toggle, before-send flow (`SendOutcome`), preview card, status strings |
| Preference model | `crates/model/src/translate_preferences.rs` | Serializable `TranslatePreferences` shape with `is_valid()` guards |
| Preference storage | `crates/local-store` | One field on the bounded `app_preferences` SQLite row (≤ 16 KB) |
| Preference sync | `apps/desktop/src/app_settings.rs`, `apps/desktop/src/main.rs` | Seeds egui memory once at startup, syncs back asynchronously through the existing save queue |
| Launchers | `scripts/setup-translate.ps1/.sh`, `scripts/start-translate.ps1/.sh` | Build/patch the engine, download models, hidden background start, login autostart |
| Update channel | `apps/desktop/src/updater.rs` | Release selection pinned to this repository, SHA256SUMS verification |

## Request lifecycle

1. **Classify.** `skip_kind(text)` runs before anything else. Credentials win over all
   other rules (privacy), then non-verbal-only text, then bare addresses. A `None`
   result is the only path that builds a request.
2. **Acquire a slot.** Inline requests need one of 24 map slots (finished entries are
   evicted oldest-first when full; an all-pending map refuses work instead of queueing).
   Before-send translations hold the single `SEND_SLOT`; a second attempt returns `Busy`
   and the composer keeps the draft.
3. **POST on a worker thread.** Hand-rolled HTTP/1.1 to `SEREIN_TRANSLATE_HOST`
   (default `127.0.0.1:5050`), JSON escaped in-module. The UI thread never blocks;
   results arrive via `request_repaint()`.
4. **Same-language protocol.** The engine returns the detected language. If it matches
   the target, auto-mode stores an *empty error* which the renderer drops silently, and
   a once-per-run notice explains the first occurrence. Manual clicks always show the
   result — silence must never look like a dead button.
5. **Render.** Results render through the full markdown pipeline with mention and
   attachment handling, followed by a small `→ {target}` chip. Errors show the message
   plus a `^` fallback that opens the external translate flow in a browser.

## Skip classifier

`skip_kind(text) -> Option<SkipKind>` is a pure function; the full rule set is unit-tested.

| Kind | Rule | Notes |
|---|---|---|
| `Credential` | `sk-` / `ghp_` / `ghu_` / `gho_` / `ghs_` / `github_pat_` (≥30 run), `xox?-` (≥10), `aiza…` (≥30), `AKIA` + 16 upper/digits, `----- … PRIVATE KEY-----`, labelled `password:`/`token:`/`secret:`/… with a ≥8-char value, pure hex ≥40, mixed base64 ≥48 | Checked **first**. Word-boundary guards stop false hits inside ordinary words. Applies even mid-sentence — key material never reaches the model. |
| `LinksEmojiMentionsOnly` | Every whitespace token is a URL (`http(s)://`, `www.`, `discord.gg/`, bare `host.tld`), `<…>` angle token (`<@id>`, `<#id>`, `<:emoji:id>`, `<a:emoji:id>`, `<url>`), `@mention`, `#channel`, or Unicode emoji | Punctuation-trimmed before matching. Any single human word makes the message translatable again. |
| `EmailAddress` | The whole message is one `user@domain.tld` token | An address inside a sentence still translates. |

Skipped manual requests insert a `Skipped(kind)` entry rendered as a muted, dismissible
note. Auto-mode skips silently — a badge on every link message would be noise.

## Settings and persistence

```
startup:  SQLite row ──apply_preferences()──▶ egui memory (validate + clamp)
runtime:  settings window / globe toggle ──▶ egui memory (with_settings)
each frame changed: egui memory ──preferences()──▶ async save queue ──▶ SQLite row
```

- The row is `Option<TranslatePreferences>`: `None` (pre-0.2.0 rows) keeps defaults.
- Load-time validation replaces unknown language codes with the default target and
  clamps `context_messages` to the offered 2/4/6 — a hand-edited row cannot poison the UI.
- All fields carry container-level `#[serde(default)]`, so adding fields never requires
  a schema migration.
- The master switch (`enabled`, default `true`) pauses the feature without touching any
  other setting; the Globe mirrors it and the settings window grays out sub-settings
  while it is off.

## Concurrency limits

| Pool | Cap | Overflow behavior |
|---|---|---|
| Inline entries | 24 | Oldest finished entry evicted; all-pending map refuses new requests |
| Auto-request memo | 4096 | Cleared wholesale (rare; prevents unbounded growth in long sessions) |
| Before-send slot | 1 | `Busy` — composer keeps the draft, user retries |
| Text size | clamped | Very long messages are truncated before the request |

## Update channel

The in-app updater is pinned to this repository's releases. A release is installable
when it is stable (non-draft, non-prerelease), its tag exceeds the running version, and
it carries `serein-{tag}-Windows-X64.zip` plus `SHA256SUMS.txt` whose
`browser_download_url` matches `https://github.com/{repo}/releases/download/{tag}/{name}`
exactly. `SEREIN_UPDATE_REPO` re-points the channel for self-hosted forks; the value is
validated (`owner/name`, `[A-Za-z0-9._-]`, no `.`/`..`) before use. Upstream releases —
including their new nightly cadence — are structurally invisible to this updater.

## Extension surface

Sandboxed plugins translate through the host-mediated `RequestTranslation` action: the
plugin never gains network capability, the host performs the localhost call and
re-invokes the plugin with the result. The extension plugin at
`extensions/plugins/local-translate` keeps its own storage preference, separate from the
client settings described above.

## Internationalization

New UI strings land in `en-US` and `ko` (the fork's maintained locales); the remaining
eleven upstream locales fall back to English until translated. There is no ftl parity
gate — missing keys degrade gracefully.

## Testing and gates

- Unit tests cover the classifier (positive and negative shapes), preference validation
  and round-trips (including SQLite save/reopen), context-depth clamping, settings
  dirty-tracking, and the update-repo contract.
- Gates before every merge: `cargo fmt --check`, `cargo clippy --all-targets
  -- -D warnings`, `cargo test`, `cargo xtask check`. CI runs on manual dispatch
  (`gh workflow run ci.yml --ref main`).

## Performance envelope

Measured on RTX 5060 (8 GB, Vulkan) with gemma3-4b Q4_K_M, all layers on GPU: short
chat messages 259–349 ms, medium technical ~509 ms, long paragraphs ~3.1 s,
context-aware ~1.5–2× base. The skip classifier itself is string scanning over short
texts — microseconds, on the UI thread, before any request exists.
