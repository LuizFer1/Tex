# Tex

> Your desktop, within reach.

---

## Name

**Tex** is a personal desktop assistant. It brings two everyday tasks into one place: **finding** files, folders, and apps, and **reusing** what you copied before.

The name is short on purpose. Tex is meant to be called up, used in a few seconds, and put away.

---

## Purpose

Finding a file often means opening folder after folder, disk by disk. Something copied an hour ago is gone the moment you copy something else. Tex exists to remove that friction: less time looking, less time retyping.

### Universal search

One search box for everything on your computer.

- Covers **files and folders on every disk** available to the user.
- Includes **installed apps** in the same results. There is no separate search for each kind.
- Results are **grouped by category** and can be **filtered** by files, folders, or apps.
- Files and folders show **where they are**, so two items with the same name can be told apart.
- Type a **path** to search inside folders: `Tex/.exe` finds `.exe` files anywhere inside folders named like "Tex", and `Projetos/Tex/` lists everything below them.
- The selected result **opens directly** from the search.

### Quick calculator

The search box also does math, so a quick calculation needs no other app.

- Calculates **while you type** and shows the result above the files. **Enter** copies it.
- Operations and functions: `2^10`, `sqrt(144)`.
- Percentages: `52% of 900`, `900 - 10%`.
- Unit conversions: `10 ft in m`, `100 km/h in m/s`, `1 GB in MB`, `145 min in timespan`.
- Understands **Portuguese and English** phrases (`10 ft em m`, `52% de 900`), and the **decimal separator** (comma or dot) follows the interface language.
- Dates and times: `days until 31 mar`, `today + 30 days`, `workdays until mar 31` (Monday to Friday, no holidays), `17:30 - 9:00`. The card shows the weekday under the result.
- Time zones: `time in tokyo`, `3pm tokyo in sao paulo`. The zones come from a database built into Tex, so they work offline too.
- Works fully **offline**. There are no currencies, because they would need the network.
- Turn it off in the **Search** tab of Settings.

### Clipboard history

Tex keeps what you copy, so you can find it and use it again later. Press **Alt+V** (you can change it) to open the history.

- Text and images are **captured automatically** whenever you copy them. There is nothing to turn on per copy.
- **Every copy is a new entry.** Copying the same content twice creates two separate entries. Duplicates are kept on purpose, because each copy is its own moment in the history. The one exception: some apps write a single copy to the clipboard more than once, so the same content written again within 1 second of the last saved entry counts as that same copy.
- Each entry is **independent**. Editing, renaming, favoriting, deleting, or expiring one entry never affects another, even when their content is identical.
- The history lists entries newest first, with a **preview** on the side. You can **search** names and text, and filter by favorites, text, or images.
- **Enter** (or a double click) **copies the entry back** to the clipboard and closes the history. If you choose "Copy and paste" in Settings, it also pastes into the window you were using. **Ctrl+Enter** does the other one.
- **Ctrl+D** favorites an entry, **F2** gives it a **custom name**, **Ctrl+E** edits a text, and **Del** deletes it while the search box is empty (a favorite asks first). **Ctrl+,** opens the clipboard settings.
- Copies Tex makes itself are not captured again, so picking an old entry does not add a new one.
- Capture can be **paused** from the tray menu or in Settings. Settings also has the size limit per entry, which kinds to capture, the cleanup policy, ignored apps, and a button that clears everything except favorites.

### Text and images

The first version supports two kinds of content.

| Action | Text | Image |
| --- | --- | --- |
| Automatic capture | ✅ plain text | ✅ bitmaps (1 to 32 bits, uncompressed) |
| View / preview | ✅ | ✅ |
| Search by content | ✅ | — (by name) |
| Copy again | ✅ | ✅ |
| Favorite / unfavorite | ✅ | ✅ |
| Rename | ✅ | ✅ |
| Edit stored content | ✅ | — (rename only) |
| Delete | ✅ | ✅ |

Formatted text (HTML, RTF) is kept as plain text. Large files, folders, and ZIP files are planned for later, not for the first version.

### Retention

The history should not grow forever. You choose one policy, and Tex applies it to each entry.

**By time.** Non-favorite entries expire after the period you pick:

- 3 days
- 1 week
- 1 month

**By storage.** You pick a size limit for the history: 250 MB, 500 MB, 1 GB, 2 GB, or 5 GB.

- When usage reaches **90% of the limit**, Tex starts removing the **oldest non-favorite** entries.
- Cleanup goes on until usage is at **80% or less**, instead of deleting one entry at a time at the edge.
- Settings shows how much of the limit is in use.

Cleanup runs in the background while Tex sits in the tray, even with the history closed or capture paused. Switching policy applies the new one right away.

### Favorites

Favorites are the entries you want to keep.

- A favorite **never expires** by time.
- A favorite is **never removed** by storage cleanup, and favorites **do not count** toward the storage limit.
- A favorite stays until **you** delete it.
- Any entry can be favorited or unfavorited at any time.

### Privacy

What you copy can be anything: notes, addresses, links, private messages. Tex treats it that way.

- The history is stored **locally**, on your computer only, in `%LOCALAPPDATA%\Tex`.
- Names and content are **encrypted** (XChaCha20-Poly1305). The key is protected by Windows for your account, so another user of the computer cannot read the history. If the key is lost, Tex sets the old history aside and starts an empty one.
- The only connection Tex makes is to check GitHub for a new version of Tex and download it, which you can turn off in Settings. Nothing about you or your files is sent. There is no account, no cloud, and no tracking.
- You stay responsible for what you keep. Tex does not try to detect passwords on its own. Instead you can pause capture, ignore specific apps, let Tex skip what password managers mark as private, and review and delete entries.

---

## Stack

| Layer | Technology |
| --- | --- |
| UI | Rust + [Slint](https://slint.dev) |
| Features (search, clipboard capture, retention) | Rust |
| Clipboard history | SQLite |

**Rust** builds the whole app, interface and features, into a single native desktop program.

- **Fast search.** Results across every disk have to feel instant, which points to a local index kept up to date rather than scanning disks on every query.
- **Clipboard access.** The app watches the system clipboard in the background and reads both text and images.
- **Background cleanup.** Retention by time and by storage runs on its own, without the user opening the app.

**SQLite** stores the clipboard history in a local file on disk.

- One row per copy, so duplicate content stays as independent entries.
- Each row carries what retention needs: when it was copied, its size, and whether it is a favorite.
- Text and images are both stored in the history, so their size is measured to enforce the storage limit.
- Names and content are encrypted at rest, with a key protected by Windows (DPAPI).

**No external services.** No account SDK, no analytics SDK, no backend SDK. The only network call is the optional update check against this repository's GitHub Releases. Every other feature works with no network and no telemetry.

---

## Install

Download `Tex-Setup-<version>.exe` from [Releases](https://github.com/LuizFer1/Tex/releases) and run it. It asks for administrator rights because the search service reads the NTFS file tables, and it asks whether only your account or everyone who signs in to the computer can search.

On first run Tex asks a few questions, each with the usual choice already selected: light or dark theme, whether the Windows key also opens search, whether Tex starts with Windows, whether it checks for updates, what Enter does on a result, whether search includes hidden files, and, for the clipboard history, whether Tex may keep what you copy, whether picking an entry also pastes it, and when old entries are deleted. Nothing is captured until you finish or close the questions. Every answer can be changed later in Settings (tray menu, or Ctrl+, in the search box), along with the shortcuts, language, what search includes, and how the clipboard history works. With updates on, Tex downloads new versions in the background and installs them when you click "Update to <version>" in the tray menu or "Update and restart" in Settings.

---

## Development

Requires stable Rust with the MSVC toolchain. Windows only for now.

```sh
cargo run -p tex-app      # run a debug build
cargo build --release     # build target/release/tex.exe
cargo test
```

Search needs the indexing service, which reads the NTFS file tables and must run as administrator:

```sh
cargo build --release
target\release\tex-indexer.exe install     # in an administrator terminal; --all-users lets every account search; uninstall removes it
```

To build the installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and, after the release build, run `ISCC.exe /DAppVersion=0.4.4 installer\tex.iss` with the workspace version. The result is `target\installer\Tex-Setup-0.4.4.exe`.

To try the app without administrator rights, serve a single folder instead:

```sh
cargo run --release -p tex-indexer --example fake_server -- C:\Users\you
```

Press **Alt+Space** to open the search popup and type. **↑/↓** move the selection, **Enter** opens the item and **Ctrl+Enter** shows it in its folder (Settings can swap the two), **Tab** switches between All, Apps, Files and Folders, and **Esc** or clicking outside closes the popup. Apps (Start Menu shortcuts and executables) are listed first. Press **Alt+V** for the clipboard history. Clicking the tray icon opens or closes the popup, and its menu has Open Tex, Clipboard history, Settings, About Tex, Pause capture, and Quit.

Measured with the installed 0.3.0 release on Windows 10 at 1920×1080, with every window closed after a session of use: `tex.exe` holds about 36 MB working set (7 MB private) and uses almost no CPU. The search service's memory grows with the number of files indexed.

---

## Reader extensions

The text reader (Shift+Enter on a file in the search popup) can be extended with external programs, in any language. Each extension is a folder with an `extension.toml` in `%APPDATA%\Tex\extensions\<folder>\`. Tools → "Open extensions folder" in the reader opens (and creates) it. Extensions are read again every time a file is opened and every time the Tools menu opens, so a new or changed extension needs no restart. A broken manifest is listed in the Tools menu with the reason and is not loaded.

An extension has **tools**, which transform the document or the selection, and **viewers**, which open new file types by converting them to Markdown or plain text:

```toml
name = "Format JSON"                 # required

[[tool]]                             # any number of tools
title = "Format"                     # required; shown in the Tools menu
files = ["json", ".jsonl"]           # required; file extensions, with or without the dot, any case; "*" = every file
command = ["python", "format.py"]    # required; program and arguments
input = "document"                   # "document" (default) | "selection" (edit mode, non-empty selection)
output = "replace"                   # "show" (default, output panel) | "replace" (document or selection) | "none"
shortcut = "Ctrl+Shift+F"            # optional

[[viewer]]                           # any number of viewers
files = ["srt", "vtt"]               # required; "*" is not allowed here
command = ["python", "srt.py"]       # required
output = "markdown"                  # "markdown" (default) | "text"
```

- `output = "replace"` needs an editable document (text up to 128 KB, or one already changed in the reader); the result is not saved until Ctrl+S. `"show"` puts the output in a panel under the text, with Copy and Close. `"none"` only says "done".
- A file type opened by a viewer becomes readable from the popup. When two viewers claim the same type, the one in the folder that comes first alphabetically wins and the Tools menu lists the conflict. The viewer's output is what the reader shows; Ctrl+E still edits the raw file when it is text.
- Shortcuts use the same names as Tex's own shortcuts (`Ctrl`, `Alt`, `Shift`, `Win`, a letter, digit, `F1`–`F24` or a key name such as `Space`). The reader keeps Ctrl+E, Ctrl+S, Ctrl+Shift+E, Ctrl+Shift+S, Ctrl+F, Shift+F3 and Alt+Z; a tool shortcut needs Ctrl or Alt; a tool asking for a reserved key, a key without Ctrl or Alt, or a key an earlier tool took, still works from the menu without the shortcut, and the menu says why. Ctrl+Alt shortcuts only work with a letter or an F key, because AltGr arrives as Ctrl+Alt on Windows and would fire them while typing characters such as `@` or `€`.

Protocol:

- The command runs in the extension's folder, without a console window. `command[0]` is a file in that folder if one exists with that name, otherwise a program on the `PATH`; later arguments that name files in the folder are passed as full paths. Parts of `command` may not be absolute paths or contain `..`: every path stays inside the extension's folder.
- stdin gets the document or the selection as UTF-8 (empty for viewers); stdout must be UTF-8. Environment: `TEX_FILE` (the file's full path), `TEX_EXTENSION_DIR` and `TEX_VERSION`.
- A non-zero exit code is a failure: the reader shows the first line of stderr (or the exit code) in a banner, or on the error screen for a viewer.
- Limits: 10 seconds and 16 MB of output. While a tool runs the reader shows "Running…" with Cancel. Cancel, the time limit or too much output stops the command and every process it started.

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

`upper.ps1` in the same folder (Windows PowerShell 5.1 reads and writes redirected text in the OEM code page unless told otherwise):

```powershell
[Console]::InputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$text = [Console]::In.ReadToEnd()
[Console]::Out.Write($text.ToUpper())
```

---

## License

Tex is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE.md).

- **Free to use** for any noncommercial purpose: personal use, study, hobby projects, and use by charities, schools, and public institutions.
- **Free to modify** and to share your changes, as long as the license and its `Required Notice:` line go along with every copy.
- **Commercial use is not allowed** under this license. To use Tex commercially, contact the author for a separate license.

The UI is built with [Slint](https://slint.dev), used under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0. As that license requires, Tex shows the `AboutSlint` widget on its About screen.

The bundled fonts, [Nunito](https://github.com/googlefonts/nunito) and [IBM Plex Mono](https://github.com/IBM/plex), are under the SIL Open Font License 1.1 (`crates/tex-app/ui/fonts`). The icons come from [Phosphor](https://phosphoricons.com) under the MIT license (`crates/tex-app/ui/icons`).
