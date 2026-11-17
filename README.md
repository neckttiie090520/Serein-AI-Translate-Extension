<p align="center">
  <img src="docs/assets/banner.svg" alt="Serein Translate banner" width="100%">
</p>

# Serein Translate

[![CI](https://github.com/neckttiie090520/Serein-AI-Translate-Extension/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/neckttiie090520/Serein-AI-Translate-Extension/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#credits--license)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)](docs/platform-support.md)

**On-device LLM translation for the Serein Discord client.** Inline translations under any message, auto-translate for incoming traffic, preview-confirmed translation before you hit send — and an optional conversation-context mode that resolves pronouns and names from the last messages in the channel. All inference runs on your own GPU through a local [LTEngine](https://github.com/LibreTranslate/LibreTranslate) server. **Zero cloud APIs, zero API keys, zero telemetry.**

> Forked from [ViceVerse-cz/Serein](https://github.com/ViceVerse-cz/Serein) — a native Rust Discord client. This repo carries the translation layer; everything else is upstream.

---

## Why

Every Discord translation plugin today (Vencord Translate & friends) ships your messages to Google, DeepL or Kagi. That means API keys, rate limits, accounts, and your conversations on someone else's server. Serein Translate is the first client-integrated translation that never leaves the machine: a 4B model quantized to 4 bits runs entirely on a consumer GPU and answers in a fraction of a second.

| | **Serein Translate** | Vencord Translate |
|---|---|---|
| Where inference runs | your GPU (localhost) | Google / DeepL / Kagi cloud |
| API key needed | none | yes (per service) |
| Inline translation under messages | ✅ full markdown | ✅ |
| Auto-translate incoming | ✅ with same-language skip | ✅ |
| Translate before sending | ✅ direct **or preview-confirmed** | ✅ (no preview) |
| Conversation-context translation | ✅ last 6 messages resolve pronouns/names | ❌ |
| Extension API for plugins | ✅ host-mediated `RequestTranslation` | — (userscript) |
| Privacy | text never leaves `127.0.0.1` | text leaves the machine |
| Cost | free, offline-capable | metered / account-bound |

## Features

- **Inline translation** — click ⇄ on any message; the translation renders under it through the full markdown pipeline, with a *Translated from {language}* caption and a one-click hide in the hover toolbar.
- **Auto-translate incoming** — new messages translate themselves. Messages already in your target language are detected and skipped silently.
- **Translate before sending** — two modes: send immediately, or get a **preview card** above the composer with *Send translation* / *Keep original draft*.
- **Context-aware mode** — the last 6 channel messages are passed along, so the model resolves *"เจอกันพรุ่งนี้นะ"* vs *"see you there"* correctly based on who said what. Toggle it on when nuance matters; off when speed does.
- **Settings window in the chat bar** — target languages, incoming/outgoing toggles, preview and context switches, live service indicator.
- **Sandboxed extension API** — plugins can request translations through a validated host-mediated action; the Wasm sandbox keeps its no-network guarantee.

![UX preview](docs/assets/ux-preview.svg)

## Performance

Measured on RTX 5060 (8 GB, Vulkan) · gemma3-4b Q4_K_M, all 34 layers on GPU · Ryzen 7800X3D:

| Message type | Latency |
|---|---|
| Short chat (≤ 10 words) | **259–349 ms** |
| Medium technical | ~509 ms |
| Long paragraph (TH→EN) | ~3.1 s |
| Context-aware (6 messages ctx) | ~1.5–2× base |

Quality holds across TH↔EN, JP→EN; internet slang is the known weak spot of a 4B model.

## Architecture

![Architecture](docs/assets/architecture.svg)

1. **Serein** (this fork) renders chat and hosts `local_translate` — raw HTTP/1.1 over a background thread pool, no new dependencies added to the UI crate.
2. **LTEngine** (separate process, LibreTranslate's Rust llama.cpp server, patched in by the setup wizard) exposes `/translate` and `/translate-context` on `127.0.0.1:5050`.
3. **gemma3-4b** runs on your GPU via Vulkan. Nothing — not one byte of message text — ever leaves localhost.

## Getting started

### 1. Run the translation engine

```bash
# LTEngine — LibreTranslate's llama.cpp server (Vulkan build)
```

### 1. One-command setup

The setup wizard checks your toolchain, builds the LTEngine server (patching in the `/translate-context` endpoint this fork uses for context-aware mode), picks a gemma3 GGUF for your GPU, downloads it (no Hugging Face account needed) and writes the launcher config.

 ```powershell
# Windows (PowerShell)
git clone https://github.com/neckttiie090520/Serein-AI-Translate-Extension
cd Serein-AI-Translate-Extension
powershell -ExecutionPolicy Bypass -File scripts\setup-translate.ps1
 ```

 ```bash
# macOS / Linux
git clone https://github.com/neckttiie090520/Serein-AI-Translate-Extension
cd Serein-AI-Translate-Extension
./scripts/setup-translate.sh
 ```

Start the engine any time with `scripts\start-translate.ps1` (Windows) or `scripts/start-translate.sh` (macOS/Linux), then run the client. Prefer manual setup? See [docs/translate-setup.md](docs/translate-setup.md).

## Hardware & model picks

Any Vulkan-capable GPU works (Metal on macOS; `--cpu` runs CPU-only but slowly). GGUF quant is Q4_K_M; VRAM is the model resident on GPU.

| Setup | GPU VRAM | Disk | Expectation |
|---|---|---|---|
| gemma3-1b | 2 GB+ | ~0.8 GB | smoke test only — quality suffers |
| **gemma3-4b (default)** | 4 GB+ | ~2.5 GB | the sweet spot: 259–349 ms short chat messages |
| gemma3-12b | 10 GB+ | ~7.5 GB | noticeably better nuance and slang |
| gemma3-27b | 18 GB+ | ~16.5 GB | best quality, slowest |

Comfortable minimums for the whole stack: Windows 10+/Linux/macOS, 8 GB RAM, ~10 GB free disk (model + Rust toolchain + build artifacts), up-to-date GPU drivers. The wizard auto-picks by detected VRAM; override with `-Model gemma3-12b` / `--model gemma3-12b`.

### 2. Build & run the client

```bash
git clone https://github.com/neckttiie090520/Serein-AI-Translate-Extension
cd Serein-AI-Translate-Extension
cargo run --release
```

Optional environment knobs:

| Variable | Default | Purpose |
|---|---|---|
| `SEREIN_TRANSLATE_HOST` | `127.0.0.1:5050` | LTEngine address |
| `SEREIN_TRANSLATE_TARGET` | `en` | default target language |
| `SEREIN_LOCAL_TRANSLATE` | `1` | set `0` to fall back to browser links |

Then open the **⇄ Globe** button in the chat bar to pick languages and toggles.

### 3. Set your languages once (30 seconds)

In the settings window: pick **incoming target** + turn on *auto-translate incoming*, pick **outgoing target** + turn on *auto-translate outgoing* (with *preview* if you want to confirm before sending), and toggle **context-aware** when nuance matters.

**Working in an English workplace, reading Thai?** Set incoming target to `th` and outgoing target to `en`. English messages translate to Thai inline as they arrive; type in Thai, hit enter, and the English translation waits in a preview card — *Send translation* posts it. One window, no copy-pasting into external tools.

## Engineering notes

- **No-dependency HTTP client.** The UI crate gained zero deps: translation calls are hand-rolled HTTP/1.1 POSTs on `std::thread`, JSON escaping/unescape included — egui stays render-loop-clean, results arrive via repaint requests.
- **Bounded work pool.** At most 24 concurrent inline translations; finished entries are evicted oldest-first, all-pending state refuses new work instead of queueing silently.
- **Same-language detection without a second model.** The engine returns the detected language; entries whose detection matches the target are dropped invisibly in auto mode, but manual clicks always show the result — silence should never look like a dead button.
- **Stale-translation guard.** Messages edited after translation are detected by normalized-text comparison and re-requested automatically.
- **Extension sandbox preserved.** `RequestTranslation` is an Apply-consented host action — the plugin never gains network capability, the host does the fetch and re-invokes the plugin with the result.

## Roadmap

- [ ] Prebuilt binaries via GitHub Actions (Windows / Linux / macOS)
- [ ] Language auto-detection heuristics before the request (skip obvious targets client-side)
- [ ] Upstream PR: `RequestTranslation` as a generic extension-SDK action

## Credits & license

- **[Serein](https://github.com/ViceVerse-cz/Serein)** by ViceVerse-cz and contributors — the client this builds on. Apache-2.0.
- **[LTEngine](https://github.com/LibreTranslate/LTEngine)** — AGPL-3.0, runs as a separate process (not linked).
- **gemma3** — Google, model weights via Hugging Face under its own license.

This fork: Apache-2.0. See `NOTICE` for attribution details.

Community standards: [CONTRIBUTING.md](CONTRIBUTING.md) · [CODE_OF_CONDUCT.md](.github/CODE_OF_CONDUCT.md) · [SECURITY.md](SECURITY.md) · [CHANGELOG.md](CHANGELOG.md) · [SUPPORT.md](.github/SUPPORT.md)

## Warning

Using unofficial Discord clients automates a normal account outside the official OAuth2/bot API, which violates Discord's Terms of Service and carries a risk of account termination. This is an interoperability experiment for personal use — review [Serein's compatibility notes](docs/discord-compatibility.md) before logging in with any account you care about.

