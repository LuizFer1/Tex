# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Tex is a personal desktop assistant: universal search (files, folders, installed apps) plus clipboard history. Implemented: tray icon, global hotkeys, popup, file/folder/app search (service + results UI), the clipboard history (capture of text and images, encrypted storage, retention, the history window opened by Alt+V) the settings window (General, Search and Clipboard tabs) and the first-run onboarding (`Local/specs/2026-10-17-onboarding-design.md`). The clipboard design is in `Local/specs/2026-10-16-clipboard-design.md`.

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
"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" /DAppVersion=0.4.1 installer\tex.iss   # after cargo build --release; AppVersion = workspace version; output target\installer\Tex-Setup-0.4.1.exe
cargo run --release -p tex-indexer --example fake_server -- C:\some\folder   # no admin: serve one folder for app testing
```

Only one Tex can run at a time (named mutex). Quit the running one from the tray menu before starting another, or the new one exits immediately.

## Architecture

Cargo workspace, crates in `crates/`:

- `tex-app` (binary `tex`): Slint UI and app wiring. `.slint` files are in `ui/` and compiled by `build.rs` with the Fluent style. `ui/app.slint` re-exports the six root components: `Popup` (frameless search window), `HistoryWindow` (frameless clipboard history, `ui/history.slint`), `AboutWindow`, `SettingsWindow` (General, Search and Clipboard tabs), `OnboardingWindow` (first-run wizard, `ui/onboarding.slint`), and `TexTray` (Slint's built-in `SystemTrayIcon`, so there is no `tray-icon` crate; its menu also opens the history and pauses capture). Visual identity ("v3 Mascote"): `ui/theme.slint` (`Tokens`), `ui/artwork.slint` and `ui/kit/` (own components that only read `Tokens`; the Fluent widgets remain only in the About window because of `AboutSlint`); Nunito and IBM Plex Mono fonts embedded (OFL) and Phosphor icons (MIT) in `ui/icons`. `theme.rs` holds the modular designs: a new design is a `Design` value in `DESIGNS`, and no `.slint` outside the theme files uses a literal color (`tests/tokens.rs` enforces it). The popups are cut with `shape_tabbed_window`; the settings window is frameless (`drag_window`, `drag_resize_window`, `round_corners`). The executable icon is `assets/tex.ico`, embedded by `build.rs` (`embed-resource`); `assets/make_assets.py` regenerates the icon and illustrations from the design handoff. search_client.rs owns the pipe on a background thread and coalesces keystrokes (only the newest search is sent), and the popup drops `Results` whose id is not the newest search sent; results.rs holds the pure row/selection logic. icons.rs loads real app icons on a COM thread (tex_platform::app_icon) into a 256-entry cache that lives on the UI thread; rows show the plain icon until the real one arrives. settings.rs is the pure settings model (`%APPDATA%\Tex\settings.toml`, read field by field so bad values fall back to defaults; an unparsable file is renamed to `.bad`); shortcut.rs converts shortcuts between text, Slint key events and `global-hotkey`. app.rs holds the UI-thread `AppState` (settings, the two registered hotkeys, Windows-key hook, when the popup closed, the window to paste into) in a thread-local; changes apply immediately and are saved on every edit. The Tex and history shortcuts can never be the same, and both are unregistered while either one is being recorded. clipboard_client.rs runs a worker thread that owns the `tex_clipboard::Store` (`%LOCALAPPDATA%\Tex\clipboard.db`, key in `clipboard.key` protected by DPAPI; a key that is missing or does not open the database moves both to `.bad` and starts empty): the UI sends `Command`s and gets `Event`s back through `invoke_from_event_loop`, never touching the database. The worker coalesces searches, applies retention, and waits on its channel with `recv_timeout` until the next expiry (plain `recv` when nothing can expire). history.rs holds the pure history rows, selection, sizes and dates; the history window drops its rows when hidden. onboarding.rs is the pure onboarding model (step 0–10, answers as option indexes where 0 is always the factory default and `None` keeps the current value, `apply`). The wizard opens at start while `general.onboarding_done` is false, which includes upgrades from before it existed; until it is done nothing is captured (`app::client_settings`) and the Windows-key hook stays off (`Settings::winkey_active`). Choosing the theme previews it (`AppState::theme_preview`), and the answers apply through `apply_settings`, the same path as the settings window. Started with `--autostart` (the installer's `Run` value) and "Start with Windows" off, the app exits before creating windows. All visible text uses `@tr`; `ui/translations/pt_BR/LC_MESSAGES/tex-app.po` is bundled by `build.rs` (no translation context), so add every new string there. Rust passes data (categories, drive letters, numbers) and Slint builds the sentences.
- `tex-platform`: OS-specific code behind a per-OS module with the same public API (`SingleInstance`, `work_area_under_cursor`, `primary_work_area`, `open_path`, `reveal_in_folder`, `app_icon`, `pick_folder`, `pick_file`, `user_language`, `system_uses_dark_theme`, `local_time`, `WinKeyHook`, `catch_windows_key`, `ClipboardWatcher`, `write_text`, `write_image`, `foreground_window`, `take_foreground`, `restore_foreground`, `lock_foreground`, `paste_into`, `shape_tabbed_window`, `round_corners`, `protect`, `unprotect`). `WinKeyHook` is a low-level keyboard hook on its own thread: it swallows the Windows key until another key makes it a combination (then it sends the key-down first), and a lone press calls back. Windows does not call that hook while a window of Tex has the focus, so the popup's window procedure is subclassed (`catch_windows_key`): it sends the mask key VK 0xE8 on the Windows key-down and closes the popup on the lone release. The popup takes the foreground with `take_foreground` (never winit's `focus_window`, whose injected Alt makes Windows open the Start menu), locks it while open so the Start/Search window cannot steal it, and on close gives it back to the window that had it (`restore_foreground`) instead of letting Windows activate the Search window. `ClipboardWatcher` is a message-only window on its own thread (`AddClipboardFormatListener`) that reads each update into a `Clip`: text or DIB, the opt-out formats, the owner exe name, and whether Tex wrote it. It first reads only the formats (`ClipFormats`) and asks a callback which `Payload` to read, so skipped copies (paused, opted out, ignored apps) read no content and the DIB is read only when the image is the one captured; the app answers with `ClipboardClient::wanted`, which applies the capture rules to the settings `configure` last set. Windows can queue several `WM_CLIPBOARDUPDATE` for one clipboard state, so an update whose `GetClipboardSequenceNumber` matches the last one is not read. Tex's own writes carry the `Tex.Copy` format so they are not captured again; `paste_into` gives the focus back and sends Ctrl+V with `SendInput`. `protect`/`unprotect` are DPAPI for the current user. The hook, the watcher, clipboard writes and paste are only verified manually. Uses `windows-sys` 0.59, the same version `global-hotkey` pulls in.
- `tex-clipboard`: the clipboard history, pure and tested (no OS calls; the key comes from outside). `crypto.rs` seals fields with XChaCha20-Poly1305 (`nonce || ciphertext`). `image.rs` converts DIB ↔ RGBA ↔ PNG and makes thumbnails. `store.rs` is the SQLite database (`rusqlite` bundled, WAL, `auto_vacuum = INCREMENTAL`, schema in `user_version`): one row per copy with `kind`, `copied_at`, `size` and `favorite` in clear and `name`, `summary` and `content` sealed; a check row in `meta` detects a wrong key, and a row that does not decrypt shows as unreadable and can only be deleted. Search folds with `tex_index::fold`, keeps the filter in the `WHERE`, matches images only by name, and can be cancelled. `retention.rs` holds the pure rules (time; storage from 90% down to 80%; favorites never removed nor counted) and `capture.rs` decides whether a copy is saved, in the spec's order, and fingerprints content to recognize the same copy written again (`is_repeat`).
- `tex-index`: pure in-memory file name index (no OS calls). One `VolumeIndex` per drive: a `Vec` of 12-byte entries positioned by MFT record number, plus a single UTF-8 name buffer. Renames append names; the buffer is compacted when over 25% is garbage. `search` scans all volumes in parallel (`std::thread::scope`, top-K heap per thread) and can be cancelled. Queries split on `/` and `\`: the last piece is the name and earlier pieces are folders that must match ancestors in order, nearest first, through a per-search byte table of matching directories (a leading `C:` picks the drive); a name term starting with `.` must end the name. Snapshots are a versioned binary format; bump `VERSION` in `snapshot.rs` on any layout change. Queries carry `app_roots` (Start Menu folders, built by the service from `%ProgramData%` and the request's home); shortcuts below them ignore hidden ancestors, and apps rank above everything else. Hidden and system are separate filters (the Windows folder and root metadata count as system), and queries also carry excluded folders and which categories to return; both are applied before a hit takes a slot in the limit.
- `tex-ipc`: messages between the app and the search service (`serde` + `postcard`, u32 length prefix, 4 MB limit).
- `tex-indexer` (lib + binary): the search service. Each NTFS volume gets a thread (`runner.rs`) that loads its snapshot from `%ProgramData%\Tex\index` or enumerates the MFT (`FSCTL_ENUM_USN_DATA`), then blocks on `FSCTL_READ_USN_JOURNAL` and applies close records. `engine.rs` answers requests and is pure (tested); `pipe.rs` serves `\\.\pipe\tex-indexer` with a DACL for SYSTEM, Administrators and either the installing user (`--allow <sid>`) or every interactive user (`--allow-interactive`, from `install --all-users`). The pipe is strictly request→response: never read and write the same synchronous pipe handle from two threads. New volumes are discovered at start and on every `Status` request (no device notifications). Opening volumes needs admin, so `runner.rs` and `volumes.rs` I/O are only verified manually. The SCM starts the exe as `tex-indexer service --allow…` (`service.rs`); that subcommand is not for manual use. Data lives in `%ProgramData%\Tex` (private ACL): snapshots in `index\<volume serial>.bin`, written via temp file + rename, and the log in `logs\indexer.log`, rotated at 1 MB. Check the log first when the service misbehaves.

Releases: `installer/tex.iss` (Inno Setup, UTF-8 with BOM) installs both exes to Program Files. Before copying it removes any existing `tex-indexer` service using the new exe extracted to `{tmp}`, then runs `install` or `install --all-users` according to its "Who can use Tex" page and puts the shortcut and the `Run` value in the user or the common scope. `.github/workflows/release.yml` runs `cargo test`, then builds the installer and a draft GitHub Release on every `v*` tag, which must match the workspace version. There is no other CI, so run `cargo test`, `cargo clippy --all-targets` and `cargo fmt` locally before committing.

Event flow: everything runs on the main thread inside Slint's winit event loop (`run_event_loop_until_quit`, so hiding every window does not exit). `global-hotkey` needs a Win32 message loop on the thread that created the manager, and winit's loop provides it. Hotkey callbacks arrive through `set_event_handler` and go back into Slint with `invoke_from_event_loop`; the history hotkey is told apart by its id. Nothing polls and there are no timers, so the CPU stays at 0 while idle. Keep it that way.

Slint uses the software renderer (`renderer-software`) with winit. Access to the raw winit window (focus, skip taskbar, focus-lost events) goes through the `unstable-winit-030` feature, which is tied to Slint's minor version. Recheck it when upgrading Slint.

## Repository rules

- `Local/` is gitignored and must never be committed (requirements, specs, plans stay on the machine). Never `git add -f` anything in it.
- Commits use Conventional Commits in Portuguese, without accents (e.g. `docs: adiciona readme e ignora pasta local`). Do not add a `Co-Authored-By` trailer.
- The author identity is set in the repo-local git config. Do not override it.

## Stack

- Rust for everything: UI and features, built as one native desktop program. The UI framework is Slint, chosen for its low RAM and CPU footprint. Target: under ~80 MB of RAM at idle with the popup closed.
- Windows first, with macOS and Linux planned later. Keep OS-specific code (clipboard, file index, installed apps, global hotkey, tray) in `tex-platform`'s per-OS modules, all exposing the same public API, so other platforms can be added without touching the UI or the core.
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
- **One copy = one row.** Every copy event creates a new history entry, even when the content is identical. Never deduplicate. The one exception, decided by the user: apps often write a single copy more than once (OLE writes and then flushes; some tools write twice), so content identical to the last saved row within 1 s (`REPEAT_WINDOW_MS`) is that same copy and is not saved again. Entries are fully independent: rename, edit, favorite, delete, and expiry act on one row only.
- **v1 content types** are text and images. Text supports search by content and editing. Images support preview, rename, favorite, and copy back when the format is supported. Large files, folders, and ZIPs are out of scope for v1.
- **Retention** follows one user-chosen policy:
  - By time: non-favorites expire after 3 days, 1 week, or 1 month.
  - By storage: at 90% of the configured limit, remove the oldest non-favorites until there is real headroom, not one entry at a time.
  - Each row therefore needs a copy timestamp, its size in bytes, and a favorite flag. Retention runs in the background, without the UI being open.
- **Favorites** are never removed by time or storage cleanup. Only the user deletes them.

Decided by the user (details in `Local/specs/2026-10-16-configuracoes-design.md` and `Local/specs/2026-10-16-clipboard-design.md`):

- Storage limits are fixed presets (250 MB, 500 MB, 1 GB, 2 GB, 5 GB), never a typed value.
- Favorites do not count toward the storage limit, so they are never limited.
- Sensitive content is the user's call: Tex does not detect passwords. It offers pause, ignored apps, respecting the clipboard formats other apps use to opt out, and deleting entries.
- Images can only be renamed; their content is not editable.
- Tex's own copies back to the clipboard are not captured.

## Open decisions

Do not settle open product decisions on your own. Ask the user. None are open right now.
