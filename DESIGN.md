---
name: Courly
description: A calm desktop mail client where the message comes first.
colors:
  primary: "#0F6CBD"
  primary-dark: "#479EF5"
  surface: "#FFFFFF"
  surface-dark: "#292929"
  ink: "#242424"
  ink-dark: "#F5F5F5"
  success: "#107C10"
  success-dark: "#54B054"
  warning: "#BC4B09"
  warning-dark: "#F7630C"
  danger: "#C50F1F"
  danger-dark: "#DC626D"
typography:
  heading:
    fontFamily: "Inter"
    fontSize: "18px"
    fontWeight: 600
  title:
    fontFamily: "Inter"
    fontSize: "16px"
    fontWeight: 600
  body:
    fontFamily: "Inter"
    fontSize: "13px"
    fontWeight: 400
  secondary:
    fontFamily: "Inter"
    fontSize: "12px"
    fontWeight: 400
  label:
    fontFamily: "Inter"
    fontSize: "11px"
    fontWeight: 400
  caption:
    fontFamily: "Inter"
    fontSize: "10px"
    fontWeight: 400
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
spacing:
  xxxs: "2px"
  xxs: "4px"
  xs: "8px"
  sm: "12px"
  md: "16px"
  lg: "24px"
  xl: "32px"
  xxl: "48px"
  xxxl: "64px"
components:
  thread-card:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    height: "68px"
  unread-dot:
    backgroundColor: "{colors.primary}"
    size: "6px"
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.surface}"
    rounded: "{rounded.sm}"
  compose-field:
    textColor: "{colors.ink}"
    typography: "{typography.body}"
---

# Design System: Courly

This file records the visual system Courly already has, so new screens stay
in the same family. It is descriptive: `UI.md` and
`crates/app/src/ui/layout.rs` remain the binding source for every constant
and for the rules of the iced toolkit. When this file and the code disagree,
the code is right and this file is stale.

## Overview

**Creative North Star: "The Quiet Desk"**

Courly is open all day in front of people who read and answer hundreds of
messages. The interface is furniture: it holds the mail and stays out of the
way. Structure comes from alignment, spacing and thin rules, not from boxes.
Colour is rare and always means something.

The direction, in the owner's words: contemporary, minimalist, soft rounded
forms, close to Apple's restraint, with every Outlook function still within
reach. The compose header is the reference case: fields sit on the surface
and a hairline separates the rows, where it used to stack bordered boxes.

**Key Characteristics:**

- Message first: chrome, borders and controls recede so mail reads first.
- Lines, not boxes: a 1px rule separates; a frame is the exception.
- One accent: blue marks unread mail and the main action, nothing decorative.
- Dense but aired: small type on a strict spacing scale, never cramped.
- Same family in light and dark, and across the built-in themes.

## Colors

A neutral surface with one blue accent, modelled on Outlook's palette so the
app feels familiar to people coming from it. The six seed colours of a theme
generate the rest of the palette (eight background levels and base, weak and
strong variants of each role).

### Primary

- **Signal Blue** (`primary`, and `primary-dark` on dark): unread dot, unread
  sender and subject, primary buttons, selection highlight at 30% opacity,
  links.

### Neutral

- **Paper** (`surface`, `surface-dark`): the base every pane rests on.
  Neighbouring panes step one background level apart to separate without a
  border.
- **Ink** (`ink`, `ink-dark`): body text. Secondary and tertiary text are
  lighter steps of the same ink, never a different grey.

### Status

- **Success**, **Warning**, **Danger**: state only (sent, attention, error,
  destructive action). Never used as decoration.

### Named Rules

**The Signal Rule.** Blue appears only where it tells the reader something:
unread, selected, primary action, link. A screen with blue used as ornament
is wrong.

**The One Step Rule.** A hover or a neighbouring surface sits exactly one
background level from what it rests on. Skipping levels makes the interface
feel heavy.

## Typography

**Font:** Inter, for every text. Icons come from Lucide, drawn at the same
sizes as the text beside them.

**Character:** small, even and quiet. Hierarchy comes from weight and from
the ink step, rarely from size.

### Hierarchy

- **Heading** (semibold, 18px): the title of a settings page or a dialog.
- **Title** (semibold, 16px): the subject of the open thread.
- **Body** (regular, 13px): message lists, fields, compose text.
- **Secondary** (regular, 12px): menu items, snippets.
- **Label** (regular, 11px): field labels, banners, metadata.
- **Caption** (regular, 10px): timestamps and counters.

### Named Rules

**The Weight Before Size Rule.** An unread thread is bold and blue at the
same size as a read one. Emphasis changes weight or colour before it changes
size.

## Layout

A desktop window in three panes: folders (180px), thread list (400px), and a
reading pane that takes the rest. A right sidebar (240px) collapses on its
own below 1200px of width. Panes are resizable.

Spacing follows one geometric scale (2, 4, 8, 12, 16, 24, 32, 48, 64). Tight
inside a group, generous between groups. Three densities change row heights
and paddings together: a thread card is 56px compact, 68px comfortable, 82px
spacious.

Every list item reserves a fixed slot for its leading mark, so text aligns
whether the mark is there or not. The unread dot (6px) has 8px of air on each
side, and a read row keeps the same empty gutter.

## Elevation & Depth

Flat. Depth comes from background levels: a pane, a card or a hovered row is
one step lighter or darker than its neighbour. Shadows are kept for things
that float above the window's content: menus, popovers, dialogs.

### Named Rules

**The Flat At Rest Rule.** Nothing at rest casts a shadow. A shadow means
"this is above the page and will go away".

## Shapes

Soft, small corners: 4px on controls, 6px on cards and menus, 8px on the
largest surfaces. Circles for avatars and for the unread dot. Separators are
1px rules in a quiet background step.

### Named Rules

**The Hairline Rule.** Rows of a form or a list are separated by a 1px rule,
not enclosed in a bordered box. A full frame is kept for a surface that
floats or that the reader must treat as one object, such as a menu or a card.

## Components

### Thread card

Three lines: sender and date, subject, snippet. An avatar on the left, with
the unread dot in the gutter before it. Unread shows as a blue dot plus bold
blue sender and subject. Hover is one background step; row actions appear on
the right.

### Compose fields

From, To, Cc, Bcc and Subject sit directly on the header. Labels are left
aligned in a fixed column, in the tertiary ink, at label size. One hairline
separates each row and the rule under the header closes the last. No field
has a border or a fill; the caret shows which field is active.

### Buttons

- **Primary:** Signal Blue fill, used once per surface for the main action
  (Send).
- **Secondary:** quiet fill one step from its surface, for actions in
  banners.
- **Bare icon:** no fill at rest, one background step on hover. The default
  for toolbars and row actions.

### Menus and popovers

A framed, rounded surface with a soft, offset shadow, floating above the
content.
Items have a fixed leading slot for an icon so labels align. A menu needs an
explicit width when its trigger is narrow.

### Banners

A full-width bar above a message, in a quiet background step: an icon, one
sentence, and at most one button. Used for blocked remote images, mailing
list unsubscribe, and the verification code.

## Do's and Don'ts

### Do:

- **Do** take every size, spacing, radius and padding from a named constant
  in `layout.rs`. No magic numbers in a view.
- **Do** separate rows with a 1px rule and let fields sit on the surface.
- **Do** keep hover one background step from the resting colour.
- **Do** reserve a fixed slot for a leading icon, avatar or dot, so labels
  align in every row.
- **Do** check a change in light and dark, and at the three densities.

### Don't:

- **Don't** enclose form fields in bordered boxes or add a heavy focus frame.
- **Don't** use blue, or any status colour, as decoration.
- **Don't** put a shadow on something that does not float.
- **Don't** introduce a second typeface or a size outside the type scale.
- **Don't** show a real company, client or account in a placeholder, an
  example or a screenshot.
