# Tex

> Your desktop, within reach.

Tex is a personal desktop assistant for Windows. It lives in the tray and comes up when you call it: one box to find files, folders, and apps, a history of what you copy, a calculator in that same box, and a reader for the text files you find.

![Tex](crates/tex-app/ui/assets/app-icon.png)

[Download the installer](https://github.com/LuizFer1/Tex/releases). **Alt+Space** opens search. **Alt+V** opens the clipboard history. Both shortcuts can be changed.

---

## Search

Results appear as you type, from an index that stays up to date instead of walking every folder on each query.

- Files and folders on every disk available to you, and installed apps, in the same list. Microsoft Store apps, such as Windows Terminal, are included.
- Results are grouped into apps, files, and folders. **Tab** switches All, Apps, Files, and Folders. Apps are listed first.
- Files and folders show where they are, so two items with the same name can be told apart.
- A path narrows the search. `Tex/.exe` finds `.exe` files inside folders named like Tex. `Projetos/Tex/` lists what is below them.
- **Enter** opens the selection. **Ctrl+Enter** shows a file or folder in its folder. Settings can swap the two.
- **Shift+Enter** opens a readable file in the reader.
- **Esc**, or a click outside, closes the popup. The Windows key also opens Tex, unless you turn that off. The popup follows the monitor under the cursor.

Hidden files are included by default. System files are not. Both, and whole folders you want left out, are in Settings › Search.

## Calculator

A calculation shows a card above the list, on the All tab, while you type. **Enter** copies the result and closes the popup.

- Arithmetic and functions: `2^10`, `sqrt(144)`.
- Percentages: `52% of 900`, `900 - 10%`.
- Units: `10 ft in m`, `100 km/h in m/s`, `1 GB in MB`, `145 min in timespan`. Adjacent quantities add, as in `5 ft 3 in`.
- Dates and clock times: `days until 31 mar`, `today + 30 days`, `workdays until mar 31`, `17:30 - 9:00`. Workdays are Monday to Friday, with no holiday calendar. The card shows the weekday under a date.
- Time zones: `time in tokyo`, `3pm tokyo in sao paulo`. The zone data is built into Tex.

Phrases work in Portuguese and English (`10 ft em m`, `52% de 900`). The decimal separator follows the interface language. A lone number, a file name, or a path does not open the card. There is no currency conversion. Turn the card off in Settings › Search.

## Clipboard

Tex keeps what you copy so you can find it later. Text and images are captured as you copy them.

- The history lists entries newest first, with a preview. Search matches names and text. Images match by name. Filters cover favorites, text, and images.
- **Enter** copies the entry back and closes the history. With "Copy and paste" in Settings, it also pastes into the window you were using. **Ctrl+Enter** does the other action.
- **Ctrl+D** favorites an entry, **F2** gives it a name, **Ctrl+E** edits a text, and **Del** deletes it while the search box is empty. Deleting a favorite asks first.
- Copies Tex writes itself are not captured again.

Every copy is its own entry, even when the content matches an older one. Rename, edit, favorite, delete, and expiry act on that entry alone. The one exception: some apps write a single copy more than once, so the same content written again within 1 second of the last saved entry counts as that same copy.

| | Text | Image |
| --- | --- | --- |
| Capture | Plain text. HTML and RTF are stored as plain text. | Uncompressed bitmaps, 1 to 32 bits. |
| Search | By content and by name. | By name. |
| Copy back, favorite, rename, delete | Yes. | Yes. |
| Edit the stored content | Yes. | Rename only. |

Each entry also has a size limit: 1, 5, 10, 25, or 50 MB. The default is 10 MB. Pause capture from the tray menu or from Settings, choose which kinds to keep, ignore apps, and clear everything except favorites.

### How long entries stay

You pick one policy. Tex applies it in the background, with the history closed and with capture paused.

**By time.** Non-favorites expire after 3 days, 1 week, or 1 month. One week is the default.

**By storage.** The history limit is 250 MB, 500 MB, 1 GB, 2 GB, or 5 GB. One gigabyte is the default. At 90% of the limit, Tex removes the oldest non-favorites until usage is at 80% or below. Settings shows how much of the limit is in use.

A favorite never expires by time, is never removed by storage cleanup, and does not count toward the limit. It stays until you delete it.

Large files, folders, and ZIP archives are outside the clipboard history.

## Reader

**Shift+Enter** on a file in the popup opens it in its own window.

Markdown is rendered, with its images: PNG, JPEG, GIF, WebP, and BMP, from local files or from `https://` addresses. GIFs animate only while they are on screen and the window has focus. These types open as plain text: `txt`, `log`, `json`, `jsonl`, `csv`, `tsv`, `toml`, `yaml`, `yml`, `ini`, `cfg`, `conf`, `env`, and `xml`. A file with no extension opens when its contents look like text. Logs, CSV, TSV, and JSONL keep one line per row. **Alt+Z** toggles wrapping.

**Ctrl+F** searches. **F3** and **Shift+F3** move between hits. **Ctrl+E** edits a text of up to 128 KB. **Ctrl+S** saves, keeping the file's encoding, byte-order mark, and line endings. Closing with unsaved changes asks whether to save them. A change made by another program is noticed when the window gets the focus.

Larger text files can still be read. Other types open when a [reader extension](#reader-extensions) provides a viewer.

## Keys

| Key | Action |
| --- | --- |
| **Alt+Space** | Open search. The Windows key does this too, unless you turn it off. |
| **Alt+V** | Open the clipboard history. |
| **↑ / ↓** | Move the selection. |
| **Tab** | Switch All, Apps, Files, and Folders. |
| **Enter** | Open the selection, copy a calculation, or copy a history entry. |
| **Ctrl+Enter** | Show the file in its folder, or the other clipboard action. |
| **Shift+Enter** | Open the file in the reader. |
| **Esc** | Close the popup, the history, or the reader. |
| **Ctrl+,** | Settings, from search or from the history. |

Clicking the tray icon opens or closes the popup. The menu has Open Tex, Clipboard history, Settings, About Tex, Pause capture, and Quit.

The two global shortcuts can be changed in Settings. They cannot be the same one.

## Privacy

The history is a file on this computer, in `%LOCALAPPDATA%\Tex`. Names and content are encrypted with XChaCha20-Poly1305. Windows protects the key for your user account. If the key is missing or cannot open the database, Tex sets the old history aside and starts an empty one.

Tex does not try to recognize passwords. You can pause capture, ignore apps, let Tex skip copies that other apps mark as private, and delete entries yourself.

The only network use is checking [this repository's releases](https://github.com/LuizFer1/Tex/releases) for a new version and downloading the signed installer. The check waits until the first-run questions are done, and it stays off while "Check for updates automatically" is off. "Check now" in Settings runs it on purpose. Nothing about you or your files is sent.

The second network use is remote images in Markdown files. Tex loads an `https://` image only when **Load images from the internet** is on in Settings › General, which is off by default, or when you press **Load images from the internet** in the reader's banner for that window. While it is off, the reader shows the image's alt text. Remote images are kept in memory only, never on disk.

## First run and settings

The first launch asks nine questions. Each one already has the usual choice selected: light or dark theme, whether the Windows key opens search, whether Tex starts with Windows, whether it checks for updates, what Enter does on a result, whether search includes hidden files, whether Tex may keep what you copy, whether picking a history entry also pastes it, and when old entries are removed. Nothing is captured until you finish or close the questions.

Every answer can be changed later. Settings is in the tray menu, or **Ctrl+,** in the search box, on three tabs:

- **General.** Language (follow Windows, Portuguese, or English), theme (light, dark, or follow Windows; light is the default), shortcuts, the Windows key, start with Windows, which monitor the popup uses, and updates. "Redo" walks through the first-run questions again.
- **Search.** How long the last query stays, hidden and system files, excluded folders, which categories appear, what Enter does, and the calculator.
- **Clipboard.** The history shortcut, pause, which kinds to capture, the size limit per entry, copy or copy-and-paste, the cleanup policy, apps to ignore, and clearing the history.

With updates on, Tex downloads a new version in the background. "Update to …" in the tray menu, or "Update and restart" in Settings, runs the installer. The installer is checked against a signature shipped with Tex before it runs.

## Install

Download `Tex-Setup-<version>.exe` from [Releases](https://github.com/LuizFer1/Tex/releases) and run it. It asks for administrator rights because the search service reads the NTFS file tables, and it asks whether only your account or everyone who signs in can search.

Started by the installer with "Start with Windows" off, Tex exits without opening a window.

## Building

Stable Rust with the MSVC toolchain. Windows only for now. The OS-specific code sits behind one API, so another system can be added without rewriting the interface.

```sh
cargo run -p tex-app          # debug build, with a console
cargo build --release         # target/release/tex.exe, no console
cargo test
cargo clippy --all-targets
cargo fmt
```

Only one Tex runs at a time. Quit the one in the tray before starting another.

Search needs the indexing service, which reads the NTFS file tables and must run as administrator:

```sh
cargo build --release
target\release\tex-indexer.exe install            # administrator terminal
target\release\tex-indexer.exe install --all-users
target\release\tex-indexer.exe uninstall
```

`cargo run -p tex-indexer -- console` runs the same indexer in the foreground. To try the app without administrator rights, serve one folder instead:

```sh
cargo run --release -p tex-indexer --example fake_server -- C:\Users\you
```

The installer is [Inno Setup 6](https://jrsoftware.org/isinfo.php). After a release build, from PowerShell, with the workspace version:

```powershell
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" /DAppVersion=0.4.5 installer\tex.iss
```

The result is `target\installer\Tex-Setup-0.4.5.exe`.

Settings live in `%APPDATA%\Tex\settings.toml`. The search index and the service log live in `%ProgramData%\Tex`. The clipboard database lives in `%LOCALAPPDATA%\Tex`.

Tex is a Cargo workspace. `tex-app` is the program and the Slint interface. `tex-indexer` is the search service. `tex-index`, `tex-clipboard`, `tex-reader`, and `tex-calc` are the pure cores behind search, history, the reader, and the calculator. `tex-platform` is the Windows code. `tex-ipc` is the pipe between the app and the service.

The whole program is Rust. The interface is [Slint](https://slint.dev) with the software renderer. The clipboard history is SQLite, one row per copy, with names and content encrypted at rest. There is no account, no analytics, and no backend. Apart from the optional update check, every feature works offline.

## Reader extensions

The reader can call external programs, in any language. Each extension is a folder with an `extension.toml` in `%APPDATA%\Tex\extensions\<folder>\`. Tools → "Open extensions folder" opens that directory and creates it if needed. Extensions are read again every time a file is opened and every time the Tools menu opens. A broken manifest is listed in the menu, with the reason, and is not loaded.

A **tool** transforms the document or the selection. A **viewer** opens a new file type by turning it into Markdown or plain text.

```toml
name = "Format JSON"                 # required

[[tool]]                             # any number of tools
title = "Format"                     # required; shown in the Tools menu
files = ["json", ".jsonl"]           # required; extensions, with or without the dot, any case; "*" = every file
command = ["python", "format.py"]    # required; program and arguments
input = "document"                   # "document" (default) | "selection" (edit mode, non-empty selection)
output = "replace"                   # "show" (default, output panel) | "replace" (document or selection) | "none"
shortcut = "Ctrl+Shift+F"            # optional

[[viewer]]                           # any number of viewers
files = ["srt", "vtt"]               # required; "*" is not allowed on a viewer
command = ["python", "srt.py"]       # required
output = "markdown"                  # "markdown" (default) | "text"
```

- `output = "replace"` needs an editable document: text up to 128 KB, or one already changed in the reader. The result is not saved until **Ctrl+S**. `"show"` puts the output in a panel under the text, with Copy and Close. `"none"` reports that the tool finished.
- A type opened by a viewer becomes readable from the popup. When two viewers claim the same type, the folder that comes first alphabetically wins, and the Tools menu lists the conflict. **Ctrl+E** still edits the raw file when it is text.
- Shortcuts use the same names as Tex's own (`Ctrl`, `Alt`, `Shift`, `Win`, a letter, a digit, `F1`–`F24`, or a key name such as `Space`). The reader keeps **Ctrl+E**, **Ctrl+S**, **Ctrl+Shift+E**, **Ctrl+Shift+S**, **Ctrl+F**, **Shift+F3**, and **Alt+Z**. A tool shortcut needs Ctrl or Alt. A tool that asks for a reserved key, a key without Ctrl or Alt, or a key an earlier tool took, still runs from the menu, and the menu says why. A Ctrl+Alt shortcut works only with a letter or an F key, because AltGr arrives as Ctrl+Alt and would otherwise fire the tool while typing a character.

The command runs in the extension's folder, with no console window. `command[0]` is a file in that folder when one exists under that name, and otherwise a program on `PATH`. Later arguments that name files in the folder are passed as full paths. No part of `command` may be an absolute path or contain `..`.

Standard input receives the document or the selection as UTF-8, empty for a viewer. Standard output must be UTF-8. The environment has `TEX_FILE` (the file's full path), `TEX_EXTENSION_DIR`, and `TEX_VERSION`.

A non-zero exit code fails the run. The reader shows the first line of standard error, or the exit code. A viewer failure is the error screen.

Limits are 10 seconds and 16 MB of output. While a tool runs, the reader shows "Running…" with Cancel. Cancel, the time limit, or too much output stops the command and every process it started.

Example: an "Uppercase" tool for every file, in PowerShell. `%APPDATA%\Tex\extensions\uppercase\extension.toml`:

```toml
name = "Uppercase"

[[tool]]
title = "Uppercase"
files = ["*"]
command = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "upper.ps1"]
output = "replace"
shortcut = "Ctrl+Shift+U"
```

`upper.ps1` in the same folder. Windows PowerShell 5.1 reads and writes redirected text in the OEM code page unless told otherwise:

```powershell
[Console]::InputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$text = [Console]::In.ReadToEnd()
[Console]::Out.Write($text.ToUpper())
```

## License

Tex is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE.md).

- Free for any noncommercial purpose: personal use, study, hobby projects, and use by charities, schools, and public institutions.
- Free to modify and to share your changes, as long as the license and its `Required Notice:` line go along with every copy.
- Commercial use needs a separate license from the author.

The interface is built with [Slint](https://slint.dev), under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0. As that license requires, Tex shows the `AboutSlint` widget on its About screen.

The bundled fonts, [Nunito](https://github.com/googlefonts/nunito) and [IBM Plex Mono](https://github.com/IBM/plex), are under the SIL Open Font License 1.1 (`crates/tex-app/ui/fonts`). The icons are from [Phosphor](https://phosphoricons.com), under the MIT license (`crates/tex-app/ui/icons`).
