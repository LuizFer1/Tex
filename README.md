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

To build the installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and, after the release build, run `ISCC.exe /DAppVersion=0.4.1 installer\tex.iss` with the workspace version. The result is `target\installer\Tex-Setup-0.4.1.exe`.

To try the app without administrator rights, serve a single folder instead:

```sh
cargo run --release -p tex-indexer --example fake_server -- C:\Users\you
```

Press **Alt+Space** to open the search popup and type. **↑/↓** move the selection, **Enter** opens the item and **Ctrl+Enter** shows it in its folder (Settings can swap the two), **Tab** switches between All, Apps, Files and Folders, and **Esc** or clicking outside closes the popup. Apps (Start Menu shortcuts and executables) are listed first. Press **Alt+V** for the clipboard history. Clicking the tray icon opens or closes the popup, and its menu has Open Tex, Clipboard history, Settings, About Tex, Pause capture, and Quit.

Measured with the installed 0.3.0 release on Windows 10 at 1920×1080, with every window closed after a session of use: `tex.exe` holds about 36 MB working set (7 MB private) and uses almost no CPU. The search service's memory grows with the number of files indexed.

---

## License

Tex is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE.md).

- **Free to use** for any noncommercial purpose: personal use, study, hobby projects, and use by charities, schools, and public institutions.
- **Free to modify** and to share your changes, as long as the license and its `Required Notice:` line go along with every copy.
- **Commercial use is not allowed** under this license. To use Tex commercially, contact the author for a separate license.

The UI is built with [Slint](https://slint.dev), used under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0. As that license requires, Tex shows the `AboutSlint` widget on its About screen.

The bundled fonts, [Nunito](https://github.com/googlefonts/nunito) and [IBM Plex Mono](https://github.com/IBM/plex), are under the SIL Open Font License 1.1 (`crates/tex-app/ui/fonts`). The icons come from [Phosphor](https://phosphoricons.com) under the MIT license (`crates/tex-app/ui/icons`).
