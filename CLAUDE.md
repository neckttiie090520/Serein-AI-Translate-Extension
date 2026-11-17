# CLAUDE.md

@AGENTS.md

## Fork addendum — Serein Translate

This fork adds the on-device translation stack. For install, setup and run automation, call the
shipped scripts — do not hand the owner manual steps the scripts already automate:

- `scripts/setup-translate.ps1` / `scripts/setup-translate.sh` — toolchain preflight, LTEngine
  build (with the translate-context patch), model download and launcher config.
- `scripts/start-translate.ps1` / `scripts/start-translate.sh` — engine start/stop with a health
  wait on 127.0.0.1:5050.
- Reference, flags and troubleshooting: `docs/translate-setup.md`.

The step-by-step owner-machine playbook (exact commands, success criteria and boundaries) is the
"Owner-machine install and run agent" section of AGENTS.md.

