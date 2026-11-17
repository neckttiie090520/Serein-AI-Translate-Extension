# Changelog

All notable changes carried by this fork are documented here. Upstream changes land via
rebase from [ViceVerse-cz/Serein](https://github.com/ViceVerse-cz/Serein) and are tracked
in that repository; this file covers the translation layer and fork-specific work only.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions
are fork milestones, not upstream releases.

## [0.1.0] - 2026-11-17

### Added
- On-device translation powered by a local LTEngine server (gemma3 via llama.cpp + Vulkan):
  inline translations under any message, auto-translate for incoming traffic with
  same-language skipping, and pre-send translation with a preview-confirmed card.
- Context-aware mode: the last 6 channel messages are passed to the model so pronouns and
  names resolve correctly.
- Settings window in the chat bar: target languages, incoming/outgoing toggles, preview and
  context switches, live service indicator.
- Host-mediated `RequestTranslation` extension action so sandboxed plugins can translate
  without gaining network capability.
- Project identity and documentation: logo, banner, architecture and UX preview art; README
  with cloud-vs-local comparison, measured latency benchmarks and a full setup guide.
- One-command install wizard (`scripts/setup-translate.ps1` / `.sh`, `start-translate`
  launchers): prerequisite checks, GPU VRAM detection with an auto-picked unsloth GGUF
  model (gemma3 1b/4b/12b/27b), resumable download, LTEngine translate-context patch,
  and config written to the platform data directory.
- Discoverability and agent setup: `llms.txt` (llmstxt.org format), README FAQ matching
  search intent, keyword-rich media alt text, expanded repository topics; AGENTS.md and
  CLAUDE.md owner-machine install-and-run playbook with GPU-aware model recommendation.

