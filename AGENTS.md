# AGENTS.md

Guidance for agents and contributors working in this repo.

## What this is

VitaDesktop is a pure-Rust desktop environment / shell overlay for Windows 11. It
replaces the native taskbar with its own top bar, dock, launcher, and overlays,
rendered by `iced` 0.14 (wgpu/DX12) and driven by raw Win32, DWM, COM, and UI
Automation calls through the `windows` 0.62 crate.

Everything is Windows-only. There is no cross-platform fallback and no CI matrix.

## Workspace layout

Crate dependency direction is strictly one way: `vita-core` <- `vita-win32` <- `vita-shell`.

| Crate | Lines | Role |
| --- | --- | --- |
| `crates/vita-core` | ~3k | Platform-free domain layer. Config model + TOML persistence, theme tokens, spring physics, fuzzy search, calendar model. No `windows` dependency. Must stay testable on any host. |
| `crates/vita-win32` | ~9k | All native integration: monitors/DPI, taskbar guard, work-area reservation, tray proxy, DWM decoration, global menu, hotkeys, notifications, thumbnails, watchdog. |
| `crates/vita-shell` | ~7k | `iced::daemon` UI. `app.rs` (~6.2k lines) is the central state machine; `surfaces/*` are per-panel view builders. |

Rules:

- Put pure logic (anything computable from data) in `vita-core`, not `vita-win32`.
- Never make `vita-core` depend on `windows` or on shell state.
- New native feature: module in `vita-win32` + `pub use` re-export in
  `crates/vita-win32/src/lib.rs`, then consume from the shell.

## Commands

Run from the repo root. Bash-compatible shell assumed on Windows (Git Bash / WSL);
PowerShell syntax differs only in path separators.

```sh
# Quality gates (both must be clean before a commit)
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Faster inner loop
cargo test -p vita-core
cargo test -p vita-win32 --lib tray
cargo clippy -p vita-shell --all-targets -- -D warnings

# Release build -> target/release/vita-shell.exe
cargo build --release
```

`-D warnings` is the contract. Do not add `#[allow(...)]` to silence clippy; fix the
cause. The only accepted exception is `#[allow(dead_code)]` on FFI struct fields that
mirror on-wire Win32 layouts.

## Running

```sh
./target/release/vita-shell.exe                      # full shell, hides native taskbar
./target/release/vita-shell.exe --no-hide-taskbar    # companion mode, keeps taskbar
./target/release/vita-shell.exe --restore-taskbar    # unconditional restore + exit
./target/release/vita-shell.exe --preset pantheon-dark --open-settings --settings-tab bar
./target/release/vita-shell.exe --open-tray --open-dock-thumbnails --open-calendar
```

Other flags: `--watchdog <pid>` (internal), `--open-launcher`, `--open-quick-controls`,
`--open-spaces`, `--open-notifications`, `--open-global-menu`,
`--open-decorator-preview`, `--screenshot-dir <dir>`, `--debug-tuck`, `--companion`.

Logging uses `tracing` with `EnvFilter`, default `vita_shell=info,vita_win32=info,warn`.
Override with `RUST_LOG=vita_shell=debug`.

## Runtime conventions

- **Config** lives at `%APPDATA%\VitaDesktop\config.toml`, loaded through
  `ShellConfig::load_or_default()`. Every config struct is `#[serde(default)]` so new
  fields are backward compatible. Malformed files fall back to defaults with a
  `tracing::warn!`, never a panic.
- **Crash safety**: any code that hides the taskbar or mutates work areas must be
  reversible by `restore_all_taskbars()` / `restore_saved_workareas()`. The panic hook
  and the `--watchdog` child process both call them. Do not add a new destructive
  Win32 state change without a matching restore path.
- **Errors**: Win32 failures are logged (`tracing::warn!`/`error!`) and the feature
  degrades; they are not propagated to the UI as panics. `expect()` is acceptable only
  for invariants that cannot fail at runtime (window class registration, resource load).
- **Blocking work** (COM, `SendMessageTimeoutW`, process enumeration) must not run on
  the `iced` update thread. Follow the existing `tokio::spawn` + channel pattern and
  send results back as `Message` variants.
- **`unsafe`** is expected in `vita-win32` around FFI. Keep each block minimal, include
  a safety justification comment when the invariant is non-obvious, and keep `HWND`
  values wrapped rather than passing `isize` across module borders.

## Testing

Tests live inline as `#[cfg(test)] mod tests`. There is no `tests/` directory and no
mocking framework.

- `vita-core` tests are pure and always reliable.
- `vita-win32` tests touch live Win32 state. Tests that register a `Shell_TrayWnd` or
  otherwise mutate global shell state must take `SHELL_TRAY_TEST_LOCK`
  (`crates/vita-win32/src/tray.rs`). Tests run in parallel by default.
- Some `vita-win32` tests only pass when no other process owns shell windows. If you
  develop with `vita-shell.exe` or another `Shell_TrayWnd` proxy running, expect
  `shell_tray_proxy_end_to_end_real_shell_notify_icon` to fail: `Shell_NotifyIcon`
  routes to whichever process won the `FindWindowW("Shell_TrayWnd")` race, so the
  test's proxy may never receive the `WM_COPYDATA`. Kill those processes before
  treating a failure as a regression.

## Style

- `edition = "2024"`, `resolver = "3"`. Let-chains (`if let ... && ...`) are used
  throughout; prefer them over nested `if let`.
- Code is `rustfmt`-default formatted. No `rustfmt.toml` override exists; do not add one.
- No `mod.rs` files except `surfaces/mod.rs`. Prefer `snake_case` module files.
- UI code: one function per surface returning `Element<'_, Message>`; no widget
  construction inside `app.rs::update`.
- No em dashes in comments, docs, or commit messages. No decorative comments that
  restate the code.

## Commits

Conventional-commit prefixes, lowercase, scope = crate or area:

```
fix(vita-win32): use REG_SAM_FLAGS for registry key access
feat(vita-shell): add dock visibility toggle in settings
```

`Implement ...`/`Add ...` capitalized messages also appear in history; either is fine.
Keep one logical change per commit and do not commit `target/`.

## Known sharp edges

- Multiple processes registering `Shell_TrayWnd` (VitaDesktop, third-party docks) make
  tray behavior order-dependent. `find_real_explorer_tray_hwnd()` deliberately skips
  windows from the current process only, so a foreign non-Explorer proxy can be picked
  up as "the real taskbar". Verify against an actual Explorer session before trusting
  tray forwarding logic.
- Work-area backups are written to `workareas.bak`; stale backups from a crash can
  restore to a wrong-scale layout after a DPI change.
- `vita-shell` sets `windows_subsystem = "windows"` in release builds, so panics are
  invisible without a debug build or `RUST_LOG`.
