# Building and releasing the fork

Everything a contributor needs to build, test, and ship a release of
Serein-AI-Translate-Extension. User-facing installation lives in
[translate-setup.md](translate-setup.md); the design of the translation layer in
[translate-architecture.md](translate-architecture.md).

## Toolchain

- Rust per `rust-toolchain.toml` (pinned; the vendored egui requires a recent stable).
- Windows: VS Build Tools with CMake **and** Perl on `PATH` (both are needed by the
  vendored `boringssl` build script). The repo pins its own toolchain version — if a
  system Rust shadows it, prepend the rustup toolchain's `bin` to `PATH` for the session.

## Everyday gates

Run all four before considering work done; CI runs the same set on dispatch.

```bash
cargo fmt -p ui -p model -- --check
cargo clippy -p ui -p model -p serein --all-targets --quiet -- -D warnings
cargo test -p ui -p model -p serein --quiet
cargo xtask check
```

Known accepted noise: one vendored-egui `f32::EPSILON` deprecation warning upstream
(excluded from the gate), and one upstream composer test
(`reply_original_buttons_navigate_locally_or_request_one_page_without_sending`)
broken by an upstream commit — upstream CI is red on it too, and fork policy is not to
patch shared upstream files. Anything else red means your change broke something.

## Windows traps (all hit the hard way)

- **Close the app before linking.** The linker cannot replace `serein.exe` while the
  app runs; the build dies at the final link step with a file-in-use error.
- **Long quiet commands can be killed by terminal wrappers.** Launch long cargo runs
  detached (`Start-Process` with redirected output), poll the PID, then read the log.
- **PowerShell 5.1 reads BOM-less scripts as ANSI.** Any script containing non-ASCII
  (em dashes, Korean, emoji) must be written and re-saved as UTF-8 **with BOM**, or the
  bytes silently mutate into mojibake before execution.
- **`gh` release creation swallows asset arguments in some wrapper shells.** Always
  verify assets after `gh release create` and repair with `gh release upload --clobber`.
- **Pass `-R <fork>` to `gh`** when creating releases/uploads/dispatches — with two
  remotes, non-interactive `gh` cannot infer the default repository.

## Commit style

Conventional Commits, fork-scoped: `feat(local-translate): …`, `fix(updater): …`,
`docs: …`. One logical change per commit; body explains *why* when it is not obvious.

## CI

The fork's workflows do **not** trigger on push. Runs are dispatched manually:

```bash
gh workflow run ci.yml --ref main -R neckttiie090520/Serein-AI-Translate-Extension
```

## Release procedure

1. **Version.** Bump `version` in the workspace `Cargo.toml` (fork milestones:
   `0.2.0` → settings persistence + fork-pinned updates; `0.3.0` → skip classifier +
   status UX + one-click toggle). `Cargo.lock` updates on the next build.
2. **Update `CHANGELOG.md`** — newest entry first, Keep a Changelog sections.
3. **Gates green**, then build: `cargo build --release -p serein` (≈10 min on a
   dev laptop).
4. **Package with the script.** `powershell -File scripts\package-release.ps1 -Version {tag}`
   builds `serein-v{tag}-Windows-X64.zip` and `SHA256SUMS.txt` and verifies the layout.
   The zip must contain the full extracted-package layout - `serein.exe`, `README.md`,
   `THIRD_PARTY_NOTICES.md`, `LICENSE-MIT`, `LICENSE-APACHE` and a `licenses/` folder -
   because the updater's installation check refuses exe-only folders ("Run Serein from
   an extracted release package") and the repack step only accepts top-level paths on
   its allowlist. Never hand-roll an exe-only zip again; that is exactly what
   v0.2.0/v0.3.0 shipped and it cannot install.5. **Tag and publish.**
   ```bash
   git tag v{tag} && git push fork v{tag}
   gh release create v{tag} -R neckttiie090520/Serein-AI-Translate-Extension \
     --verify-tag --title "v{tag}" --notes "…" <zip> <SHA256SUMS.txt>
   ```
6. **Verify the updater's view of the world.**
   ```bash
   gh api repos/neckttiie090520/Serein-AI-Translate-Extension/releases/latest \
     --jq '{tag: .tag_name, urls: [.assets[].browser_download_url]}'
   ```
   The tag must be what you expect, non-draft/non-prerelease, and every
   `browser_download_url` must equal
   `https://github.com/neckttiie090520/Serein-AI-Translate-Extension/releases/download/{tag}/{name}`.
   Empty asset lists mean step 5's assets were swallowed — re-upload with `--clobber`.
7. **Dispatch CI** for the release commit and watch it go green (minus the known
   upstream test).

## Deploying to a local machine (dogfooding)

Close the running client, copy `target/release/serein.exe` over the installed one
(keep a `.pre-translate-build` backup the first time), relaunch. If the app ever
auto-updates itself to a build without the translation layer (should be impossible —
the channel is pinned to this fork), re-deploy from `target/release`.

## Upstream policy

This fork rebases on [ViceVerse-cz/Serein](https://github.com/ViceVerse-cz/Serein).
Shared upstream files (including its tests) are not modified to make the fork's gates
green — regressions there are tracked upstream. Fork-owned surfaces: the translation
module and its integration points, the launcher scripts, the fork docs, and the
updater's repository configuration.
