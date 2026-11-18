# Translation setup reference

The on-device translation stack is two processes on your machine:

1. **Serein** (this repo) — the Discord client. Talks to the engine over localhost HTTP only.
2. **[LTEngine](https://github.com/LibreTranslate/LTEngine)** — LibreTranslate's Rust llama.cpp server, built by the setup wizard with the `/translate-context` endpoint patched in (this fork uses it for context-aware mode; without the patch every other feature still works and context-aware falls back to plain `/translate`).

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

## Background autostart (no console window)

The engine always runs headless. On Windows it has no window and no taskbar
button; its output goes to the data directory:

| Platform | Engine log |
|---|---|
| Windows | `%LOCALAPPDATA%\SereinTranslate\engine.log` / `engine.err` |
| Linux | `~/.local/share/serein-translate/engine.log` / `engine.err` |
| macOS | same paths under `~/Library/Application Support` via launchd's paths in the plist |

To bring the engine back automatically after a reboot â€” the setup wizard offers
this as its last step â€” run the installer directly:

```bash
# Windows: Task Scheduler task, fires ~30 s after login (GPU-driver warm-up),
# launches the engine through wscript so no console ever appears
powershell -ExecutionPolicy Bypass -File scripts\start-translate.ps1 -Install

# macOS: LaunchAgent (kept alive) / Linux: systemd user service (restart on failure)
./scripts/start-translate.sh --install
```

Manage it with `-Status`, `-Stop`, `-Uninstall` (POSIX: `--status`, `--stop`,
`--uninstall`). A plain run of the script starts the engine hidden immediately,
autostart or not.
## What the wizard does

1. Checks `git`, `cargo` (rustup), `cmake`, `clang` and prints `winget`/`apt`/`brew` install hints for anything missing (`-CheckOnly` / `--check-only` reports without changing anything).
2. Clones LTEngine (`--depth 1`) into a sibling `LTEngine/` directory when one exists, otherwise into `%LOCALAPPDATA%\SereinTranslate\LTEngine` (`~/.local/share/serein-translate/LTEngine`).
3. Applies [scripts/ltengine-translate-context.patch](../scripts/ltengine-translate-context.patch) — 86 added lines implementing `/translate-context`. Skipped cleanly if upstream already ships the endpoint or the patch no longer applies.
4. Builds with `cargo build --release --features vulkan` (`metal` on macOS, `cuda` is a manual change). First build compiles llama.cpp: 10-30 minutes.
5. Downloads one gemma3 GGUF (Q4_K_M) from Hugging Face mirrors that need no account, sized to detected VRAM via `nvidia-smi`, and writes the launcher config.

## Flags

| Flag (ps1 / sh) | Effect |
|---|---|
| `-CheckOnly` / `--check-only` | environment report only, no changes |
| `-Model <name>` / `--model <name>` | `gemma3-1b`, `4b`, `12b`, `27b` — default: auto by VRAM |
| `-ModelFile <path>` / `--model-file <path>` | use an existing .gguf |
| `-EngineDir <path>` / `--engine-dir <path>` | existing LTEngine checkout to reuse |
| `-SkipBuild` / `--skip-build` | reuse the previous engine binary |
| `-Port <n>` / `--port <n>` | engine port, default 5050 |
| `-StartEngine` / `--start-engine` | launch the engine right after setup |
| `-Stop` / `--stop` (start-translate) | stop the engine process |

## Config and data locations

- Windows: `%LOCALAPPDATA%\SereinTranslate\config.json`, models under `models\`
- Linux/macOS: `~/.local/share/serein-translate/config.json`, models under `models/`

```json
{
  "engineExe": ".../ltengine(.exe)",
  "modelFile": ".../gemma-3-4b-it-Q4_K_M.gguf",
  "host": "127.0.0.1",
  "port": 5050,
  "model": "gemma3-4b"
}
```

The client reads `SEREIN_TRANSLATE_HOST` (default `127.0.0.1:5050`) and `SEREIN_TRANSLATE_TARGET` (default of both incoming and outgoing targets). `SEREIN_LOCAL_TRANSLATE=0` falls back to the legacy browser-link translation.

## Troubleshooting

- **Build fails on Windows** — install VS 2022 Build Tools with the *Desktop development with C++* workload, plus LLVM and CMake; the repo root `serein-cargo.cmd` pattern (vcvars + `LIBCLANG_PATH`) shows the environment the build needs.
- **Vulkan feature fails** — install GPU driver + Vulkan runtime, or Linux `libvulkan-dev`/`shaderc`. CPU fallback: build without features and run `ltengine --cpu`.
- **Model download interrupted** — rerun the wizard; `curl -C -` resumes.
- **Port 5050 busy** — `-Port 5060` and set `SEREIN_TRANSLATE_HOST=127.0.0.1:5060` before starting the client.
- **Context-aware says endpoint missing** — your engine was built without the patch; rerun the wizard (it patches automatically on a fresh clone).

## Licensing note

LTEngine is AGPL-3.0 and runs as a separate process — Serein never links it. The context patch is therefore also AGPL-3.0 and lives in this repo only as a build-time patch applied to LTEngine's own checkout.

