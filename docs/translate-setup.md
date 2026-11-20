# Translation setup and operations

The on-device translation stack is two processes on your machine:

1. **Serein** (this repo) — the Discord client. Talks to the engine over localhost HTTP only.
2. **[LTEngine](https://github.com/LibreTranslate/LTEngine)** — LibreTranslate's Rust llama.cpp server, built by the setup wizard with the `/translate-context` endpoint patched in (used for context-aware mode; without the patch every other feature still works and context-aware falls back to plain `/translate`).

For the engineering design see [translate-architecture.md](translate-architecture.md).
For building the client from source see [building.md](building.md).

## Quick start

```bash
# Windows
powershell -ExecutionPolicy Bypass -File scripts/setup-translate.ps1   # offers background autostart at the end
powershell -ExecutionPolicy Bypass -File scripts/start-translate.ps1  # or start the hidden engine right now

# macOS / Linux
./scripts/setup-translate.sh
./scripts/start-translate.sh

# then, from the repo root
cargo run --release
```

The wizard checks prerequisites, detects GPU VRAM, builds the patched engine, downloads
an auto-picked gemma3 GGUF (override with `-Model`/`--model`), and writes the launcher
config to the platform data directory (`%LOCALAPPDATA%\SereinTranslate` on Windows,
`~/.config/SereinTranslate` elsewhere).

## Background autostart

The engine always runs headless — no console window, no taskbar button. On Windows it
has no window at all (verified via `MainWindowHandle = 0`).

| OS | Mechanism | Notes |
|---|---|---|
| Windows | Task Scheduler task `SereinTranslateEngine`, logon trigger +30 s | Per-user, no admin rights; launched through a wscript wrapper so nothing flashes; restarts up to 3 times on failure |
| Linux | systemd user unit `serein-translate.service` | `systemctl --user enable --now serein-translate` |
| macOS | LaunchAgent `com.serein.translate.plist` | KeepAlive enabled |

Lifecycle commands:

```powershell
# Windows
powershell -ExecutionPolicy Bypass -File scripts/start-translate.ps1 -Status
powershell -ExecutionPolicy Bypass -File scripts/start-translate.ps1 -Stop
powershell -ExecutionPolicy Bypass -File scripts/start-translate.ps1 -Install    # (re)install autostart
powershell -ExecutionPolicy Bypass -File scripts/start-translate.ps1 -Uninstall
```

```bash
# macOS / Linux
./scripts/start-translate.sh --status
./scripts/start-translate.sh --stop
./scripts/start-translate.sh --install
./scripts/start-translate.sh --uninstall
```

Engine logs rotate at `%LOCALAPPDATA%\SereinTranslate\engine.log` (5 MB, one `.old`
generation kept); `autostart.log` records task launches.

## Using it (30 seconds)

- **Globe button in the chat bar** — one click pauses or resumes translation for the
  whole app (the icon dims while off). Right-click opens settings.
- **Settings window** — master switch, incoming/outgoing target languages, auto
  toggles, preview-before-send, context-aware mode with 2/4/6 message depth.
  Everything persists across restarts.
- **Inline** — click ⇄ on any message. A spinner shows while translating; the result
  renders under the message with a `→ {language}` chip.
- **Skipped messages** — links-only, emoji-only, mentions-only, bare email addresses,
  and anything shaped like a key or credential are never sent to the model. A manual
  attempt shows a muted *not translated* note instead of failing silently.
- **Before sending** — with auto-outgoing on, the composer shows what it is doing:
  *translating before send*, *draft kept* (another translation still running),
  *sending as-is* (nothing to translate), or *sent the translated message*.

## Verifying the engine by hand

```bash
curl -s http://127.0.0.1:5050/translate \
  -H 'Content-Type: application/json' \
  -d '{"q":"hello how are you","source":"auto","target":"th"}'
```

A JSON object with a translation within a few seconds means the engine is healthy; the
request field is `q` (not `text`) and `source` should be `"auto"`.

## Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `SEREIN_TRANSLATE_HOST` | `127.0.0.1:5050` | Engine address |
| `SEREIN_TRANSLATE_TARGET` | `en` | First-run default target; later changes persist in settings |
| `SEREIN_LOCAL_TRANSLATE` | `1` | `0` disables the feature and falls back to browser-link translation |
| `SEREIN_UPDATE_REPO` | this repository | GitHub `owner/name` the in-app updater tracks |

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| Clicking translate opens a browser link instead of translating | The running client predates the fork's translation layer (e.g. an updater or installer replaced the exe) | Re-deploy the fork build ([building.md](building.md), "Deploying to a local machine") |
| Result looks identical to the original (silent no-op) | Target language equals the message language | Pick the right target in settings — the first occurrence also raises a one-time notice; settings persist since 0.2.0 |
| Spinner never resolves | Engine not running or still loading the model into VRAM | `-Status` / `--status`; start it; wait ~30–60 s after login for the model to warm up |
| Engine died after a reboot (pre-0.2.0 installs) | Old broken autostart (Windows PowerShell 5.1 VBS bug) | Update the repo, then `-Install` to reinstall the fixed task |
| `cargo build` fails at the final link on Windows | The app is running and Windows locks `serein.exe` | Close the app, rebuild |
| Build fails with `rustc … is not supported` | A system Rust shadows the pinned toolchain | Prepend the rustup toolchain's `bin` to `PATH` ([building.md](building.md)) |
| Everything works but the taskbar shows a console window | Engine started manually without the scripts | Stop it and start via the scripts (hidden start is the default) |
| Model downloads again on another user account | Per-user model directory | Re-run the wizard for that account, or point `config.json`'s `modelFile` at a shared copy |

## Uninstall

1. `start-translate -Uninstall` (or `--uninstall`) removes autostart and stops the engine.
2. Delete the data directory (`%LOCALAPPDATA%\SereinTranslate` / `~/.config/SereinTranslate`)
   to remove the model and logs.
3. The client-side settings row is removed with the app's own data.

## Security notes

- The engine binds `127.0.0.1` only; message text never leaves the machine.
- Credential-shaped content (API keys, tokens, private keys, labelled passwords) is
  skipped before any request is built and never handed to the model.
- In-app updates verify `SHA256SUMS.txt` and only install releases of this repository.
