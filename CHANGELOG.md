# Changelog

All notable changes carried by this fork are documented here. Upstream changes land via
rebase from [ViceVerse-cz/Serein](https://github.com/ViceVerse-cz/Serein) and are tracked
in that repository; this file covers the translation layer and fork-specific work only.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions
are fork milestones, not upstream releases.

## [0.3.2] - 2026-11-20

### Fixed

- Extension translation requests now report results by unique id instead of list position; a finished request draining while another was still in flight could previously hang the second request forever (silently consuming a pending slot) or deliver its translation to the wrong plugin action.
- Auto-translated incoming messages that are skipped (links, emoji, credentials) are no longer re-classified on every rendered frame.


## [0.3.1] - 2026-11-20

### Fixed

- **Release packages install correctly through the in-app updater.** Earlier
  fork zips shipped the exe alone, which the updater's installation check
  refuses ("Run Serein from an extracted release package"). Zips now carry the
  full extracted-package layout (`serein.exe`, `README.md`,
  `THIRD_PARTY_NOTICES.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `licenses/`), and
  `scripts/package-release.ps1` builds and verifies that layout for every
  future release. Installs upgraded by hand from an exe-only zip can run the
  script's file set (or re-extract the v0.3.1 zip over the install) to become
  updatable.
- **Extension plugin requests follow the skip policy.** The
  `RequestTranslation` host action now classifies text before spawning the
  engine call; links/emoji/mention-only, credential-shaped and bare-email
  requests return a plain-language "Not translated" note to the plugin panel
  instead of reaching the model.

## [0.3.0] - 2026-11-20

### Added

- **Pre-translation skip classifier.** Messages that carry no natural language never reach
  the engine: bare links, `@mentions`, `#channels` and emoji-only text, bare email
  addresses, and high-confidence credential shapes — `sk-…`, `ghp_…`, `xox?-…`, `AKIA…`,
  private-key blocks, labelled `password:`/`token:` values, and long opaque hex/base64
  runs. Manual attempts render a muted *not translated* note (the button never looks
  dead); auto-translate skips silently. Credential detection runs before anything else,
  so key material is never handed to the model even inside otherwise normal sentences.
- **Visible status everywhere.** Before-send translation now returns an explicit
  `SendOutcome { Started, Busy, Skipped }`: the composer reports *translating before
  send*, *draft kept — a translation is still running*, or *nothing to translate —
  sending as-is*, and confirms with *sent the translated message*. Inline results show a
  spinner while pending and a muted skip note when classified out.
- **One-click master toggle.** Clicking the chat-bar Globe now pauses the whole feature —
  the icon dims while off (`icons::toggle`), right-click opens settings, and the tooltip
  explains both in every state. The switch persists with the rest of the settings.

### Changed

- Settings window: the master switch grays out all sub-settings while translation is off,
  matching the existing nested-disable pattern used for auto-outgoing options.
- Inline results show only the reliable target-language chip; the detected-source chip —
  which surfaced the engine's frequent mis-detections (e.g. English read as Danish) as if
  they were fact — was removed.
- `TranslatePreferences` gained `enabled` with serde default `true`; existing preference
  rows load unchanged with translation on.

## [0.2.0] - 2026-11-20

### Added

- **Settings persistence across restarts.** Translate settings moved from volatile egui
  memory into the bounded `app_preferences` SQLite row (`model::TranslatePreferences`).
  Stored targets are validated against the selectable language list on load (unknown
  codes fall back to the default; hand-edited rows cannot poison the UI) and the context
  depth is clamped to the offered 2/4/6 values. Rows written before this version load
  with defaults.
- **Fork-pinned update channel.** The in-app updater tracks releases of this repository
  only (`neckttiie090520/Serein-AI-Translate-Extension`), verified against the published
  `SHA256SUMS.txt`. Upstream releases can never silently replace a local-translation
  build. Override with `SEREIN_UPDATE_REPO` if you host your own fork releases.
- **Background autostart that survives reboots.** The engine starts hidden at login —
  Task Scheduler via a wscript wrapper on Windows (no console flash, per-user, no admin
  rights), a systemd user unit on Linux, a LaunchAgent on macOS — with
  `-Status/-Stop/-Install/-Uninstall` lifecycle commands and restart-on-failure policy.

### Fixed

- `start-translate.ps1` generated syntactically broken VBS launchers on Windows
  PowerShell 5.1 (string concatenation inside `@()` is parsed as three array elements),
  and the scheduled task killed the engine's process tree the moment the launcher
  process exited — so autostart had never actually worked. Both replaced with a
  waiting launcher and a 3-attempt restart policy.

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

