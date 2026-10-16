# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Tex is a personal desktop assistant: universal search (files, folders, installed apps) plus clipboard history. Only the foundation exists so far: tray icon, global hotkey, and an empty search popup. Search, clipboard capture, storage, and retention are not implemented yet.

The full functional requirements document (in Portuguese) is `Local/Tex_Documento_Requisitos_Funcionais.md`. `README.md` is the public English summary of it. Design specs live in `Local/specs/`.

## Commands

```sh
cargo run -p tex-app                     # debug build, keeps a console for eprintln!
cargo build --release                    # target/release/tex.exe, no console window
cargo test                               # all tests
cargo test -p tex-app popup_origin       # tests matching a name
cargo clippy --all-targets
cargo fmt
```

Only one Tex can run at a time (named mutex). Quit the running one from the tray menu before starting another, or the new one exits immediately.

## Architecture

Cargo workspace, crates in `crates/`:

- `tex-app` (binary `tex`): Slint UI and app wiring. `.slint` files are in `ui/` and compiled by `build.rs` with the Fluent style. `ui/app.slint` re-exports the three root components: `Popup` (frameless search window), `AboutWindow`, and `TexTray` (Slint's built-in `SystemTrayIcon`, so there is no `tray-icon` crate).
- `tex-platform`: OS-specific code behind a per-OS module with the same public API (`SingleInstance`, `work_area_under_cursor`). Uses `windows-sys` 0.59, the same version `global-hotkey` pulls in.

Event flow: everything runs on the main thread inside Slint's winit event loop (`run_event_loop_until_quit`, so hiding every window does not exit). `global-hotkey` needs a Win32 message loop on the thread that created the manager, and winit's loop provides it. Hotkey callbacks arrive through `set_event_handler` and go back into Slint with `upgrade_in_event_loop`. Nothing polls and there are no timers, so the CPU stays at 0 while idle. Keep it that way.

Slint uses the software renderer (`renderer-software`) with winit. Access to the raw winit window (focus, skip taskbar, focus-lost events) goes through the `unstable-winit-030` feature, which is tied to Slint's minor version. Recheck it when upgrading Slint.

## Repository rules

- `Local/` is gitignored and must never be committed (requirements, specs, plans stay on the machine). Never `git add -f` anything in it.
- Commits use Conventional Commits in Portuguese, without accents (e.g. `docs: adiciona readme e ignora pasta local`). Do not add a `Co-Authored-By` trailer.
- The author identity is set in the repo-local git config. Do not override it.

## Stack

- Rust for everything: UI and features, built as one native desktop program. The UI framework is Slint, chosen for its low RAM and CPU footprint. Target: under ~80 MB of RAM at idle with the popup closed.
- Windows first, with macOS and Linux planned later. Keep OS-specific code (clipboard, file index, installed apps, global hotkey, tray) behind traits so other platforms can be added without touching the UI or the core.
- The UI is a frameless popup opened by a global hotkey (Spotlight/PowerToys Run style). It lives in the system tray the rest of the time.

## License constraints

- Tex is under PolyForm Noncommercial 1.0.0 (`LICENSE.md`). Keep the `Required Notice:` line at the end of that file.
- Slint is used under its Royalty-free Desktop license 2.0, not GPLv3. That license requires the `AboutSlint` widget on an About screen reachable from the app's top-level menu, or the Slint badge on the download page. Don't remove it.
- Do not add GPL, AGPL, or LGPL-only dependencies: copyleft terms conflict with the noncommercial restriction. MIT, Apache-2.0, BSD, Zlib, and similar licenses are fine.
- SQLite for the clipboard history, stored in a local file and encrypted at rest (basic encryption).
- No external services. No network calls, telemetry, accounts, analytics, or backend SDKs. Every new dependency must work fully offline.

## Domain invariants

The implementation should hold to these rules from the spec:

- **Search** covers every disk available to the user plus installed apps, in one query. Results are grouped by category (files, folders, apps), filterable, and show the location for files and folders. It must feel instant, so use a local index that stays up to date instead of scanning disks on each query.
- **One copy = one row.** Every copy event creates a new history entry, even when the content is identical. Never deduplicate. Entries are fully independent: rename, edit, favorite, delete, and expiry act on one row only.
- **v1 content types** are text and images. Text supports search by content and editing. Images support preview, rename, favorite, and copy back when the format is supported. Large files, folders, and ZIPs are out of scope for v1.
- **Retention** follows one user-chosen policy:
  - By time: non-favorites expire after 3 days, 1 week, or 1 month.
  - By storage: at 90% of the configured limit, remove the oldest non-favorites until there is real headroom, not one entry at a time.
  - Each row therefore needs a copy timestamp, its size in bytes, and a favorite flag. Retention runs in the background, without the UI being open.
- **Favorites** are never removed by time or storage cleanup. Only the user deletes them.

## Open decisions

Do not settle these on your own. Ask the user:

- What happens when favorites alone exceed the storage limit.
- Whether "edit" for images means only the name or also the image content.
- How to handle sensitive content (passwords, 2FA codes).
- Whether storage limits are presets or any value the user types.
