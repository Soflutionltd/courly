<p align="center">
  <img src="assets/readme/banner.svg" alt="Courly: mail, calendar and contacts in one fast, native app" width="100%">
</p>

<p align="center">
  <a href="https://github.com/Soflutionltd/courly/releases/latest/download/Courly.dmg"><img src="assets/readme/download-macos.svg" alt="Download Courly for Mac" height="56"></a>
  &nbsp;
  <img src="assets/readme/coming-soon.svg" alt="Windows and Linux: coming soon" height="56">
</p>

<p align="center">
  <a href="https://github.com/Soflutionltd/courly/releases/latest"><img src="https://img.shields.io/github/v/release/Soflutionltd/courly?color=16B3B0&label=version" alt="Latest version"></a>
  <img src="https://img.shields.io/badge/macOS-13%2B%20%C2%B7%20Apple%20silicon%20%26%20Intel-22C97A?logo=apple&logoColor=white" alt="macOS 13 or later, Apple silicon and Intel">
  <img src="https://img.shields.io/badge/written%20in-Rust-0E8F8C?logo=rust&logoColor=white" alt="Written in Rust">
  <img src="https://img.shields.io/badge/no%20Electron-no%20web%20view-16B3B0" alt="No Electron, no web view">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-PolyForm%20Shield%201.0.0-16B3B0" alt="License: PolyForm Shield 1.0.0"></a>
  <img src="https://img.shields.io/badge/languages-English%20%C2%B7%20Fran%C3%A7ais-22C97A" alt="English and French">
</p>

<p align="center">
  <img src="assets/readme/screenshot-inbox.jpg" alt="Courly: every account in one sidebar, the message list and a conversation side by side" width="100%">
</p>

---

**Courly** is a desktop email client that brings Gmail, Outlook and Exchange, iCloud and any IMAP or JMAP mailbox into one window, with the calendar and the address book right beside your mail. It is written in Rust and draws every pixel itself on the GPU, so it opens in an instant, scrolls smoothly through tens of thousands of messages, and keeps your mail on your own computer.

## Download

| Your computer | Status |
| --- | --- |
| **Mac**, Apple silicon or Intel, macOS 13 or later | [**Download Courly.dmg**](https://github.com/Soflutionltd/courly/releases/latest/download/Courly.dmg) |
| Windows | Coming soon |
| Linux | Coming soon |

One download serves every Mac: the same app runs natively on Apple silicon and on Intel processors.

## A look around

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/readme/screenshot-search.jpg" alt="Search suggesting people and messages as you type" width="100%">
      <p align="center"><b>Search as you type</b><br>People and messages are suggested from the first letters, across every mailbox.</p>
    </td>
    <td width="50%" valign="top">
      <img src="assets/readme/screenshot-compose.jpg" alt="Writing a message with the rich text editor" width="100%">
      <p align="center"><b>Write without leaving the window</b><br>A rich text editor with signatures, attachments and read receipts.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/readme/screenshot-calendar.jpg" alt="The month view of the calendar with colored calendars" width="100%">
      <p align="center"><b>Calendar beside your mail</b><br>Every calendar of every account, by day, week or month.</p>
    </td>
    <td width="50%" valign="top">
      <img src="assets/readme/screenshot-dark.jpg" alt="Courly in its dark theme" width="100%">
      <p align="center"><b>Dark when you want it</b><br>Light, dark or following the system.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/readme/screenshot-themes.jpg" alt="The appearance settings with the built-in themes" width="100%">
      <p align="center"><b>Make it yours</b><br>Built-in themes, text size, density, and what the sidebar shows.</p>
    </td>
    <td width="50%" valign="top">
    </td>
  </tr>
</table>

All screenshots show the built-in demo data: no real mailbox.

## Why Courly

<table>
  <tr>
    <td width="33%" valign="top">
      <h3>⚡ Native and fast</h3>
      No web view and no Electron. The interface is drawn by the GPU through <a href="https://iced.rs">iced</a> and wgpu, and the search index answers as you type.
    </td>
    <td width="33%" valign="top">
      <h3>📬 Every account, one inbox</h3>
      Gmail, Microsoft 365 and Exchange, iCloud, Fastmail, and any IMAP or JMAP server, side by side or merged in a single unified view.
    </td>
    <td width="33%" valign="top">
      <h3>🔒 Your mail stays yours</h3>
      Everything is stored on your Mac. Passwords and tokens are encrypted, remote images and tracking pixels are blocked, and there is no Courly server.
    </td>
  </tr>
</table>

## Features

**Mail**
- One sidebar for every account, with a unified inbox, favorites and pinned searches
- Conversation threading that follows replies across folders and accounts
- Chat view: your exchange with one person, shown as a timeline of messages like a messaging app
- Categories with their color, kept separate for each mailbox, applied from a menu under the toolbar button
- A photo you pick for a contact shows wherever that person appears
- Drag messages onto any folder, or move them from the toolbar and the right-click menu
- Instant full-text search, including inside attachments, with people and messages suggested as you type
- Smart folders driven by a query language (dates, senders, labels, flags, attachments)
- Scheduled send, snooze, flags, pins and Gmail labels
- Command palette for every action, with keyboard shortcuts throughout

**Writing**
- Rich text editor with signatures
- Recipient autocomplete from every address you have ever written to
- Attachment compression for images, PDFs and Office documents before sending
- Cloud attachments through OneDrive and Google Drive

**Calendar and contacts**
- Month, week and day views, with invitations answered right from the message
- Google Calendar, CalDAV and JMAP calendars
- CardDAV address books, and contact import from CSV, Excel and vCard files

**Privacy and security**
- Local-first storage in SQLite, with compressed message stores
- Credentials encrypted with AES-256-GCM
- HTML sanitizing, remote image blocking, tracking pixel and tracking link removal
- Brand logos (BIMI) shown only for known senders whose mail passes DMARC; the logo itself is published by the domain and not certified by a third party

**Everyday comfort**
- Light, dark or automatic theme
- English and French interface
- Signed, notarized builds that update themselves

## How Courly is built

Courly is made of four parts. The application itself lives in this repository; the protocol engine and the rendering engine live in their own repositories so they can evolve and be tested on their own.

```mermaid
flowchart TB
    subgraph APP["Courly app (this repository)"]
        UI["Interface<br/>iced views, rich text editor,<br/>HTML mail renderer, command palette"]
        SVC["Service<br/>sync, actions, notifications,<br/>calendar, smart folders"]
        STORE["Storage and search<br/>SQLite, message stores,<br/>tantivy full-text index"]
        UI <--> SVC
        SVC <--> STORE
        UI -. read-only .-> STORE
    end

    subgraph CONDUIT["conduit: protocol engine"]
        GOOGLE["Gmail API<br/>Google Calendar"]
        GRAPH["Microsoft Graph<br/>Outlook, Exchange"]
        IMAP["IMAP + SMTP"]
        JMAP["JMAP"]
        DAV["CalDAV<br/>CardDAV"]
    end

    subgraph RENDER["Rendering engine"]
        ICED["iced fork<br/>widgets, windows, input"]
        SLUG["sluggrs<br/>GPU text rendering"]
        ICED --> SLUG
    end

    SVC --> GOOGLE & GRAPH & IMAP & JMAP & DAV
    UI --> ICED
    GOOGLE & GRAPH & IMAP & JMAP & DAV --> CLOUD(("Your mail<br/>providers"))
```

The interface never writes to the database. Every change goes through the service, which applies it locally first, so the app answers at once, and then sends it to the provider in the background.

### What the code is made of

Measured on the Rust sources (non-blank lines) of each repository:

```mermaid
pie showData
    title Rust code by part (lines)
    "Protocol engine (conduit)" : 355534
    "Service, sync and actions" : 97966
    "Interface" : 93557
    "Rendering engine (iced + sluggrs)" : 99607
    "Storage and search" : 38579
    "HTML engine (Blitz, vendored)" : 26188
```

| Repository | What it does | Size |
| --- | --- | --- |
| [**courly**](https://github.com/Soflutionltd/courly) | The app: 32 crates for the interface, service, storage, search and calendar, plus `crates/conduit`, the Gmail, Microsoft Graph, IMAP, SMTP, JMAP, CalDAV and CardDAV clients in 12 crates | 586,000 lines |
| [**iced**](https://github.com/Soflutionltd/iced) | Fork of the iced GUI toolkit used by Courly | 90,000 lines |
| [**sluggrs**](https://github.com/Soflutionltd/sluggrs) | GPU text rendering for iced | 10,000 lines |

### Main building blocks

| Area | Crates | Built on |
| --- | --- | --- |
| Interface | `app`, `rte`, `mail-html`, `cmdk`, `iced-drop` | iced, wgpu, Blitz |
| Service | `service`, `sync`, `core`, `calendar`, `smart-folder`, `ai` | Tokio |
| Storage | `db`, `db-read`, `stores`, `seen` | SQLite (rusqlite), zlib (flate2) |
| Search | `search` | tantivy |
| Attachments | `squeeze`, `import` | mozjpeg, oxipng, lopdf |
| Security | `crypto-key`, `common` | AES-256-GCM, ammonia |

## Install

1. Click **Download for Mac** at the top of this page.
2. Open `Courly.dmg`.
3. Double-click **Courly** in the window that opens: it installs itself in your **Applications** folder and starts. Dragging it onto Applications works too.

Courly is signed and notarized by Apple. It checks for new versions on its own and offers to install them.

**Requirements:** a Mac with Apple silicon or an Intel processor, and macOS 13 Ventura or later. Builds for Windows and Linux are coming.

## Build from source

Courly builds against its sibling repositories, which must sit next to it:

```sh
mkdir courly-workspace && cd courly-workspace
git clone https://github.com/Soflutionltd/courly.git
git clone --branch courly https://github.com/Soflutionltd/sluggrs.git
git clone --branch sluggrs https://github.com/Soflutionltd/iced.git sluggrs/repos/iced

cd courly
cargo +1.99.0 run -p runner --features dev-seed   # demo data, no account needed
cargo +1.99.0 build --release -p runner           # optimised build
```

You need [Rust](https://rustup.rs) 1.99 and, on macOS, the Xcode command line tools.

## Contributing

Ideas, bug reports and pull requests are welcome: that is the whole point of publishing the code.

1. Open an issue to describe the bug or the change you have in mind.
2. Fork the repository and open a pull request.
3. On your first pull request, a bot asks you to sign the [Contributor License Agreement](CLA.md) with a one-line comment. You keep the copyright to your work.

Before sending, please run `cargo +1.99.0 clippy --workspace --all-targets` and `cargo +1.99.0 test --workspace`.

## License

Courly is **source-available** under the [PolyForm Shield License 1.0.0](LICENSE).

| You may | You may not |
| --- | --- |
| Download, read and build the full source code | Publish or sell a product that competes with Courly, |
| Use Courly for yourself or inside your company | free or paid, built from this code |
| Modify it for your own use |  |
| Contribute improvements back to this repository |  |

Parts of the code come from projects published under the Apache License 2.0 and remain available under that license; see [NOTICE](NOTICE). Third-party libraries keep their own licenses, listed inside the app.

<p align="center">
  <sub>Made by <a href="https://github.com/Soflutionltd">Soflution</a></sub>
</p>
