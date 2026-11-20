# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Courly is a native desktop app, not a website: a Rust binary that draws its
own interface with iced. The schema this file follows has no desktop value,
and iced does not use the operating system's widgets, so the design language
is Courly's own on every OS. Treat "web" above as "free-form interface", and
read `UI.md` for the real constraints of the toolkit.

## Users

Two audiences, served by the same product:

- People inside a company, on Exchange or Microsoft 365, who process hundreds
  of emails a day and keep years of history. Mail and calendar are their main
  work tool, open all day.
- Independents and small structures who run several mailboxes, often across
  several businesses, and switch between them constantly.

Both read, triage and answer at volume. Speed of the common gestures matters
more to them than discoverability of rare ones.

## Product Purpose

Courly is a desktop email client with mail and calendar in one place. It
exists because no free client handles Exchange and calendar together at this
scale. Success is a mailbox of 150 GB and more that still opens, scrolls and
searches instantly.

## Positioning

Four reasons to choose Courly over Outlook or Apple Mail, all confirmed by the
owner:

- Exchange and calendar handled together, well.
- Speed and deep local search on very large mailboxes, five years of history
  and more.
- Local and private: mail stays on the machine, and nothing goes to a third
  party without a deliberate gesture from the user.
- A clearer, lighter interface than Outlook, with none of its functions lost.

## Operating Context

- A three-pane desktop window: folders, thread list, reading pane. Compose
  opens in the reading pane or in its own window.
- Used for hours at a stretch, at a desk, alongside other work apps.
- Providers: Exchange and Microsoft 365 (Graph), Gmail, JMAP, IMAP.
- Several accounts side by side, including shared mailboxes.

## Capabilities and Constraints

- Performance, storage efficiency, search speed and deduplication are hard
  requirements, not goals.
- Mail bodies and attachments are stored and searched locally.
- The interface ships in French and English.
- macOS is the released platform. A Windows version is not released.
- Light and dark appearance, a set of built-in themes, and three display
  densities (compact, comfortable, spacious).
- Distribution: free of charge. The source is public under the PolyForm
  Shield License 1.0.0, which is source-available and not an open source
  license: it forbids using the code for a competing product. Copy must say
  "free" and "source available", never "open source", unless the license
  changes.

## Brand Commitments

- Name: Courly. Publisher: Soflution Ltd.
- Examples, placeholders and screenshots never show a real company, client or
  account. Use Courly itself or invented names.

## Evidence on Hand

- The running app and its deterministic test mailbox (`dev-seed`).
- Measured performance numbers, where recorded, live in
  `reference/performance.md`.
- No testimonials, customer names, press or usage figures exist. Future work
  must not invent any.

## Product Principles

1. Instant comes first. A gesture that waits is a defect, whatever the size of
   the mailbox.
2. The message is the content. Chrome, borders and controls step back so mail
   reads first.
3. Nothing leaves the machine by surprise. Any send to a third party is a
   visible, deliberate act.
4. Every Outlook habit has a home. Simpler never means a missing function.
5. One product for the employee and the multi-mailbox independent. No mode
   that favours one against the other.
