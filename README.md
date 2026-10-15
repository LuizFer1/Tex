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
- The selected result **opens directly** from the search.

### Clipboard history

Tex keeps what you copy, so you can find it and use it again later.

- Supported content is **captured automatically** whenever you copy it. There is nothing to turn on per copy.
- **Every copy is a new entry.** Copying the same content twice creates two separate entries. Duplicates are kept on purpose, because each copy is its own moment in the history.
- Each entry is **independent**. Editing, renaming, favoriting, deleting, or expiring one entry never affects another, even when their content is identical.
- You can **search** the history, **preview** an entry, and **copy it back** to the clipboard.
- You can give an entry a **custom display name**, so it is easy to recognize later.
- You can **delete** any entry by hand.

### Text and images

The first version supports two kinds of content.

| Action | Text | Image |
| --- | --- | --- |
| Automatic capture | ✅ | ✅ when the format is supported |
| View / preview | ✅ | ✅ |
| Search by content | ✅ | — |
| Copy again | ✅ | ✅ when supported |
| Favorite / unfavorite | ✅ | ✅ |
| Rename | ✅ | ✅ |
| Edit stored content | ✅ | Still open (name only, or the image itself) |
| Delete | ✅ | ✅ |

Large files, folders, and ZIP files are planned for later, not for the first version.

### Retention

The history should not grow forever. You choose one policy, and Tex applies it to each entry.

**By time.** Non-favorite entries expire after the period you pick:

- 3 days
- 1 week
- 1 month

**By storage.** You set a size limit for the history (for example, 1 GB).

- When usage reaches **90% of the limit**, Tex starts removing the **oldest non-favorite** entries.
- Cleanup frees enough space to take real pressure off the limit, instead of deleting one entry at a time at the edge.

### Favorites

Favorites are the entries you want to keep.

- A favorite **never expires** by time.
- A favorite is **never removed** by storage cleanup.
- A favorite stays until **you** delete it.
- Any entry can be favorited or unfavorited at any time.

### Privacy

What you copy can be anything: notes, addresses, links, private messages. Tex treats it that way.

- The history is stored **locally**, on your computer only.
- Stored data is protected with **basic encryption**.
- Nothing is sent to the internet. There is no account, no cloud, and no tracking.
- You stay responsible for what you keep. Search, preview, and delete make it easy to review the history and remove what you do not want stored.

### Still to be decided

| Topic | Open question |
| --- | --- |
| Favorites over the limit | What happens when favorites alone exceed the storage limit. |
| Editing images | Whether "edit" means only the name, or also the image content. |
| Sensitive content | How to handle passwords, authentication codes, and other sensitive data. |
| Storage presets | Whether there are preset limits to pick from, or any allowed value can be typed. |

---

## Stack

| Layer | Technology |
| --- | --- |
| UI | Rust |
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
- The data is protected with basic encryption at rest.

**No external services.** No account SDK, no analytics SDK, no backend SDK, no network calls. Every new dependency has to fit an app that works with no network and no telemetry.
