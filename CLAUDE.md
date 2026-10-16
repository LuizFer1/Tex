# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Tex is a personal desktop assistant: universal search (files, folders, installed apps) plus clipboard history. Implemented: tray icon, global hotkey, popup, file/folder/app search (service + results UI), and the settings window (General and Search tabs). Not implemented yet: clipboard capture, storage, retention, and the Clipboard settings tab (specified in `Local/specs/2026-10-16-configuracoes-design.md`).

The full functional requirements document (in Portuguese) is `Local/Tex_Documento_Requisitos_Funcionais.md`. `README.md` is the public English summary of it. Design specs live in `Local/specs/`.

## Commands

```sh
cargo run -p tex-app                     # debug build, keeps a console for eprintln!
cargo build --release                    # target/release/tex.exe, no console window
cargo test                               # all tests
cargo test -p tex-app popup_origin       # tests matching a name
cargo test --release -p tex-index --test performance -- --ignored --nocapture   # 3M-entry search benchmark
cargo clippy --all-targets
cargo fmt
cargo run -p tex-indexer -- console        # admin terminal: index and serve in the foreground
cargo run -p tex-indexer -- install        # admin terminal: register and start the service from this exe
cargo run -p tex-indexer -- install --all-users   # admin terminal: same, but every signed-in user may search
cargo run -p tex-indexer -- uninstall      # admin terminal: stop and remove the service
"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" /DAppVersion=0.1.0 installer\tex.iss   # after cargo build --release: target\installer\Tex-Setup-0.1.0.exe
cargo run --release -p tex-indexer --example fake_server -- C:\some\folder   # no admin: serve one folder for app testing
```

Only one Tex can run at a time (named mutex). Quit the running one from the tray menu before starting another, or the new one exits immediately.

## Architecture

Cargo workspace, crates in `crates/`:

- `tex-app` (binary `tex`): Slint UI and app wiring. `.slint` files are in `ui/` and compiled by `build.rs` with the Fluent style. `ui/app.slint` re-exports the four root components: `Popup` (frameless search window), `AboutWindow`, `SettingsWindow` (General and Search tabs), and `TexTray` (Slint's built-in `SystemTrayIcon`, so there is no `tray-icon` crate). search_client.rs owns the pipe on a background thread and coalesces keystrokes (only the newest search is sent); results.rs holds the pure row/selection logic. icons.rs loads real app icons on a COM thread (tex_platform::app_icon) into a 256-entry cache that lives on the UI thread; rows show the plain icon until the real one arrives. settings.rs is the pure settings model (`%APPDATA%\Tex\settings.toml`, read field by field so bad values fall back to defaults; an unparsable file is renamed to `.bad`); shortcut.rs converts shortcuts between text, Slint key events and `global-hotkey`. app.rs holds the UI-thread `AppState` (settings, registered hotkey, Windows-key hook, when the popup closed) in a thread-local; changes apply immediately and are saved on every edit. Started with `--autostart` (the installer's `Run` value) and "Start with Windows" off, the app exits before creating windows. All visible text uses `@tr`; `ui/translations/pt_BR/LC_MESSAGES/tex-app.po` is bundled by `build.rs` (no translation context), so add every new string there. Rust passes data (categories, drive letters, numbers) and Slint builds the sentences.
- `tex-platform`: OS-specific code behind a per-OS module with the same public API (`SingleInstance`, `work_area_under_cursor`, `primary_work_area`, `open_path`, `reveal_in_folder`, `app_icon`, `pick_folder`, `user_language`, `system_uses_dark_theme`, `WinKeyHook`). `WinKeyHook` is a low-level keyboard hook on its own thread: a lone Windows key press calls back and is masked with VK 0xE8 so the Start menu stays closed; it is only verified manually. Uses `windows-sys` 0.59, the same version `global-hotkey` pulls in.
- `tex-index`: pure in-memory file name index (no OS calls). One `VolumeIndex` per drive: a `Vec` of 12-byte entries positioned by MFT record number, plus a single UTF-8 name buffer. Renames append names; the buffer is compacted when over 25% is garbage. `search` scans all volumes in parallel (`std::thread::scope`, top-K heap per thread) and can be cancelled. Snapshots are a versioned binary format; bump `VERSION` in `snapshot.rs` on any layout change. Queries carry `app_roots` (Start Menu folders, built by the service from `%ProgramData%` and the request's home); shortcuts below them ignore hidden ancestors, and apps rank above everything else. Hidden and system are separate filters (the Windows folder and root metadata count as system), and queries also carry excluded folders and which categories to return; both are applied before a hit takes a slot in the limit.
- `tex-ipc`: messages between the app and the search service (`serde` + `postcard`, u32 length prefix, 4 MB limit).
- `tex-indexer` (lib + binary): the search service. Each NTFS volume gets a thread (`runner.rs`) that loads its snapshot from `%ProgramData%\Tex\index` or enumerates the MFT (`FSCTL_ENUM_USN_DATA`), then blocks on `FSCTL_READ_USN_JOURNAL` and applies close records. `engine.rs` answers requests and is pure (tested); `pipe.rs` serves `\\.\pipe\tex-indexer` with a DACL for SYSTEM, Administrators and either the installing user (`--allow <sid>`) or every interactive user (`--allow-interactive`, from `install --all-users`). The pipe is strictly request→response: never read and write the same synchronous pipe handle from two threads. New volumes are discovered at start and on every `Status` request (no device notifications). Opening volumes needs admin, so `runner.rs` and `volumes.rs` I/O are only verified manually. The SCM starts the exe as `tex-indexer service --allow…` (`service.rs`); that subcommand is not for manual use. Data lives in `%ProgramData%\Tex` (private ACL): snapshots in `index\<volume serial>.bin`, written via temp file + rename, and the log in `logs\indexer.log`, rotated at 1 MB. Check the log first when the service misbehaves.

Releases: `installer/tex.iss` (Inno Setup, UTF-8 with BOM) installs both exes to Program Files. Before copying it removes any existing `tex-indexer` service using the new exe extracted to `{tmp}`, then runs `install` or `install --all-users` according to its "Who can use Tex" page and puts the shortcut and the `Run` value in the user or the common scope. `.github/workflows/release.yml` builds the installer and a draft GitHub Release on every `v*` tag, which must match the workspace version.

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

Decided by the user (details in `Local/specs/2026-10-16-configuracoes-design.md`):

- Storage limits are fixed presets (250 MB, 500 MB, 1 GB, 2 GB, 5 GB), never a typed value.
- Favorites do not count toward the storage limit, so they are never limited.
- Sensitive content is the user's call: Tex does not detect passwords. It offers pause, ignored apps, respecting the clipboard formats other apps use to opt out, and deleting entries.

## Open decisions

Do not settle these on your own. Ask the user:

- Whether "edit" for images means only the name or also the image content.
