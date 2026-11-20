//! Text selection over a rendered email body. The body is a picture; the
//! engine also reports where every character sits, so a selection is a range
//! of those characters and its highlight is drawn on top of the picture.

use iced::advanced::Renderer as _;
use iced::advanced::mouse::{Click, click};
use iced::advanced::widget::tree;
use iced::advanced::{Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard, mouse,
};
use mail_html::TextLine;

/// Share of the accent colour laid over selected text.
const HIGHLIGHT_ALPHA: f32 = 0.3;
/// How much more a vertical gap counts than a horizontal one when looking
/// for the line nearest to the pointer, so a pointer in a margin picks the
/// line beside it rather than a closer line above.
const VERTICAL_WEIGHT: f32 = 4.0;

/// A caret position: before character `cell` of line `line`, or after the
/// last character when `cell` equals the line's character count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    line: usize,
    cell: usize,
}

/// A non-empty range of selected text, `start` before `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    start: TextPos,
    end: TextPos,
}

/// What the reader did to the selection of a body.
#[derive(Debug, Clone, Copy)]
pub enum SelectAction {
    Set(Selection),
    /// The card holding lines `first..first + count` dropped its selection:
    /// a selection touching none of them belongs to another card and stays.
    Unset {
        first: usize,
        count: usize,
    },
    SelectAll,
    Copy,
    /// Right-click on the selection: show its menu at this offset from the
    /// page's top-left corner.
    ShowMenu(Vector),
    CloseMenu,
}

impl Selection {
    fn new(a: TextPos, b: TextPos) -> Option<Self> {
        (a != b).then(|| Self {
            start: a.min(b),
            end: a.max(b),
        })
    }

    /// The whole body.
    pub fn all(lines: &[TextLine]) -> Option<Self> {
        let last = lines.len().checked_sub(1)?;
        Self::new(
            TextPos { line: 0, cell: 0 },
            TextPos {
                line: last,
                cell: lines[last].cells.len(),
            },
        )
    }

    /// Whether any of lines `first..first + count` is selected.
    pub fn touches(self, first: usize, count: usize) -> bool {
        self.start.line < first + count && self.end.line >= first
    }

    /// What is selected of lines `first..first + count`, with lines counted
    /// from `first`: the selection as one card of the body sees it.
    fn within(self, first: usize, count: usize) -> Option<Self> {
        if count == 0 || !self.touches(first, count) {
            return None;
        }
        let start = if self.start.line < first {
            TextPos { line: 0, cell: 0 }
        } else {
            TextPos {
                line: self.start.line - first,
                cell: self.start.cell,
            }
        };
        let end = if self.end.line >= first + count {
            TextPos {
                line: count - 1,
                cell: usize::MAX,
            }
        } else {
            TextPos {
                line: self.end.line - first,
                cell: self.end.cell,
            }
        };
        Self::new(start, end)
    }

    /// The same selection with lines counted from the top of the body, for
    /// a card whose first line is line `first` of the body.
    fn in_body(self, first: usize) -> Self {
        Self {
            start: TextPos {
                line: self.start.line + first,
                cell: self.start.cell,
            },
            end: TextPos {
                line: self.end.line + first,
                cell: self.end.cell,
            },
        }
    }

    /// The selected characters of each line it touches, as
    /// `(line index, first cell, one past the last cell)`.
    fn spans<'a>(self, lines: &'a [TextLine]) -> impl Iterator<Item = (usize, usize, usize)> + 'a {
        let last = self.end.line.min(lines.len().saturating_sub(1));
        (self.start.line..=last).filter_map(move |index| {
            let count = lines.get(index)?.cells.len();
            let from = if index == self.start.line {
                self.start.cell
            } else {
                0
            };
            let to = if index == self.end.line {
                self.end.cell.min(count)
            } else {
                count
            };
            (from < to).then_some((index, from, to))
        })
    }

    /// The highlight, one rectangle per line, relative to the page.
    pub fn rectangles(self, lines: &[TextLine]) -> Vec<Rectangle> {
        self.spans(lines)
            .map(|(index, from, to)| {
                let line = &lines[index];
                let cells = &line.cells[from..to];
                let left = cells
                    .iter()
                    .map(|cell| cell.x)
                    .fold(f32::INFINITY, f32::min);
                let right = cells
                    .iter()
                    .map(|cell| cell.x + cell.width)
                    .fold(f32::NEG_INFINITY, f32::max);
                Rectangle::new(
                    Point::new(left, line.y),
                    Size::new(right - left, line.height),
                )
            })
            .collect()
    }

    /// The selected text. Wrapped lines of one block are joined as they
    /// were written; a new block starts on a new line.
    pub fn text(self, lines: &[TextLine]) -> String {
        let mut copied = String::new();
        let mut previous_block = None;
        for (index, from, to) in self.spans(lines) {
            let line = &lines[index];
            if previous_block.is_some_and(|block| block != line.block) {
                copied.truncate(copied.trim_end().len());
                copied.push('\n');
            }
            previous_block = Some(line.block);
            copied.push_str(&line.text[line.cells[from].start..line.cells[to - 1].end]);
        }
        copied.truncate(copied.trim_end().len());
        copied
    }

    fn contains(self, lines: &[TextLine], point: Point) -> bool {
        self.rectangles(lines)
            .iter()
            .any(|rectangle| rectangle.contains(point))
    }
}

fn line_extent(line: &TextLine) -> Option<(f32, f32)> {
    let first = line.cells.first()?;
    let last = line.cells.last()?;
    Some((
        first.x.min(last.x),
        (first.x + first.width).max(last.x + last.width),
    ))
}

/// How far `value` lies outside `low..=high`.
fn gap(value: f32, low: f32, high: f32) -> f32 {
    (low - value).max(value - high).max(0.0)
}

/// Whether `point` (relative to the page) is on a line of text.
fn is_over_text(lines: &[TextLine], point: Point) -> bool {
    lines.iter().any(|line| {
        line_extent(line).is_some_and(|(left, right)| {
            point.x >= left
                && point.x <= right
                && point.y >= line.y
                && point.y <= line.y + line.height
        })
    })
}

/// The caret position nearest to `point` (relative to the page).
fn position_at(lines: &[TextLine], point: Point) -> Option<TextPos> {
    let (index, line) = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let (left, right) = line_extent(line)?;
            let distance = gap(point.y, line.y, line.y + line.height) * VERTICAL_WEIGHT
                + gap(point.x, left, right);
            Some((distance, index, line))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, index, line)| (index, line))?;
    let cell = line
        .cells
        .iter()
        .position(|cell| point.x < cell.x + cell.width / 2.0)
        .unwrap_or(line.cells.len());
    Some(TextPos { line: index, cell })
}

fn is_word_part(text: &str) -> bool {
    text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The word under a caret position: a run of letters and digits, or the
/// single character when it is anything else.
fn word_at(lines: &[TextLine], position: TextPos) -> Option<(TextPos, TextPos)> {
    let line = lines.get(position.line)?;
    let count = line.cells.len();
    let at = position.cell.min(count.checked_sub(1)?);
    let part =
        |index: usize| is_word_part(&line.text[line.cells[index].start..line.cells[index].end]);
    let (mut from, mut to) = (at, at + 1);
    if part(at) {
        while from > 0 && part(from - 1) {
            from -= 1;
        }
        while to < count && part(to) {
            to += 1;
        }
    }
    Some((
        TextPos {
            line: position.line,
            cell: from,
        },
        TextPos {
            line: position.line,
            cell: to,
        },
    ))
}

fn line_at(lines: &[TextLine], position: TextPos) -> Option<(TextPos, TextPos)> {
    let line = lines.get(position.line)?;
    Some((
        TextPos {
            line: position.line,
            cell: 0,
        },
        TextPos {
            line: position.line,
            cell: line.cells.len(),
        },
    ))
}

/// The selection from the range a drag started on to where the pointer is.
fn dragged(anchor: (TextPos, TextPos), focus: TextPos) -> Option<Selection> {
    Selection::new(anchor.0.min(focus), anchor.1.max(focus))
}

#[derive(Default)]
struct State {
    last_click: Option<Click>,
    /// The range the drag in progress started on: a caret for a click, a
    /// word for a double click, a line for a triple click.
    drag: Option<(TextPos, TextPos)>,
    /// The last click landed on this body, so its keyboard shortcuts apply.
    active: bool,
}

/// Wraps one card of the rendered page of an email and lets the reader
/// select its text with the mouse. The selection itself lives in the
/// caller's state, with lines counted from the top of the body.
pub struct TextSelector<'a, Message> {
    content: Element<'a, Message>,
    /// The lines of this card, placed from its top-left corner.
    lines: &'a [TextLine],
    /// Where the card's first line stands among the lines of the body.
    first_line: usize,
    /// What is selected in this card, with lines counted from its first.
    selection: Option<Selection>,
    on_action: Box<dyn Fn(SelectAction) -> Message + 'a>,
}

/// `lines` are those of the card `content` shows, `first_line` the place of
/// the first of them among the lines of the body `selection` counts in.
pub fn text_selector<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    lines: &'a [TextLine],
    first_line: usize,
    selection: Option<Selection>,
    on_action: impl Fn(SelectAction) -> Message + 'a,
) -> TextSelector<'a, Message> {
    TextSelector {
        content: content.into(),
        lines,
        first_line,
        selection: selection.and_then(|selection| selection.within(first_line, lines.len())),
        on_action: Box::new(on_action),
    }
}

impl<Message> TextSelector<'_, Message> {
    fn set(&self, shell: &mut Shell<'_, Message>, selection: Option<Selection>) {
        if selection == self.selection {
            return;
        }
        let action = match selection {
            Some(selection) => SelectAction::Set(selection.in_body(self.first_line)),
            None => SelectAction::Unset {
                first: self.first_line,
                count: self.lines.len(),
            },
        };
        shell.publish((self.on_action)(action));
    }

    fn press(
        &self,
        state: &mut State,
        position: Option<Point>,
        origin: Point,
        shell: &mut Shell<'_, Message>,
    ) {
        let Some(position) = position else {
            // A click anywhere else drops the selection, like in a browser.
            state.active = false;
            state.drag = None;
            self.set(shell, None);
            return;
        };
        state.active = true;
        let click = Click::new(position, mouse::Button::Left, state.last_click);
        state.last_click = Some(click);
        let Some(caret) = position_at(self.lines, position - (origin - Point::ORIGIN)) else {
            return;
        };
        let anchor = match click.kind() {
            click::Kind::Single => Some((caret, caret)),
            click::Kind::Double => word_at(self.lines, caret),
            click::Kind::Triple => line_at(self.lines, caret),
        };
        state.drag = anchor;
        self.set(
            shell,
            anchor.and_then(|(from, to)| Selection::new(from, to)),
        );
        shell.capture_event();
    }

    fn key(
        &self,
        state: &State,
        key: &keyboard::Key,
        modifiers: keyboard::Modifiers,
        shell: &mut Shell<'_, Message>,
    ) {
        if !state.active || !modifiers.command() {
            return;
        }
        let keyboard::Key::Character(character) = key else {
            return;
        };
        let action = match character.as_str() {
            "c" if self.selection.is_some() => SelectAction::Copy,
            "a" => SelectAction::SelectAll,
            _ => return,
        };
        shell.publish((self.on_action)(action));
        shell.capture_event();
    }
}

impl<Message> Widget<Message, Theme, Renderer> for TextSelector<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        tree.size = tree.children[0].size;
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation<()>,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            viewport,
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
        if shell.is_event_captured() {
            return;
        }

        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Left,
                ..
            }) => {
                // No position at all means a menu covers the page: the click
                // is for the menu, and must not drop the selection it acts on.
                if cursor.position().is_some() {
                    self.press(
                        state,
                        cursor.position_over(bounds),
                        bounds.position(),
                        shell,
                    );
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(anchor) = state.drag
                    && let Some(position) = cursor.position()
                    && let Some(focus) =
                        position_at(self.lines, position - (bounds.position() - Point::ORIGIN))
                {
                    self.set(shell, dragged(anchor, focus));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => state.drag = None,
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Right,
                ..
            }) => {
                if let Some(selection) = self.selection
                    && let Some(position) = cursor.position_over(bounds)
                {
                    let at = position - bounds.position();
                    if selection.contains(self.lines, Point::ORIGIN + at) {
                        state.active = true;
                        shell.publish((self.on_action)(SelectAction::ShowMenu(at)));
                        shell.capture_event();
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                self.key(state, key, *modifiers, shell);
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        let Some(selection) = self.selection else {
            return;
        };
        let bounds = layout.bounds();
        let colour = theme
            .palette()
            .primary
            .base
            .color
            .scale_alpha(HIGHLIGHT_ALPHA);
        // A layer of its own: quads of the page's layer sit under its images.
        // It does not inherit the clip of the scrollable around the body,
        // so it is cut to what shows of the body.
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };
        renderer.with_layer(visible, |renderer| {
            for rectangle in selection.rectangles(self.lines) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: rectangle + (bounds.position() - Point::ORIGIN),
                        ..renderer::Quad::default()
                    },
                    colour,
                );
            }
        });
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let interaction = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        if interaction != mouse::Interaction::None {
            return interaction;
        }
        let bounds = layout.bounds();
        let over_text = cursor.position_over(bounds).is_some_and(|position| {
            is_over_text(self.lines, position - (bounds.position() - Point::ORIGIN))
        });
        if over_text {
            mouse::Interaction::Text
        } else {
            interaction
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}

impl<'a, Message: 'a> From<TextSelector<'a, Message>> for Element<'a, Message> {
    fn from(selector: TextSelector<'a, Message>) -> Self {
        Element::new(selector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_html::TextCell;

    const CELL: f32 = 10.0;

    /// A line of fixed-width characters starting at `x`.
    fn line(block: usize, x: f32, y: f32, text: &str) -> TextLine {
        let cells = text
            .char_indices()
            .enumerate()
            .map(|(index, (start, character))| TextCell {
                start,
                end: start + character.len_utf8(),
                x: x + CELL * f32::from(u8::try_from(index).expect("short test line")),
                width: CELL,
            })
            .collect();
        TextLine {
            block,
            y,
            height: 20.0,
            text: text.to_string(),
            cells,
        }
    }

    fn body() -> Vec<TextLine> {
        vec![
            line(0, 0.0, 0.0, "Your code is "),
            line(0, 0.0, 20.0, "482913 today"),
            line(1, 0.0, 60.0, "Thanks"),
        ]
    }

    fn at(lines: &[TextLine], x: f32, y: f32) -> TextPos {
        position_at(lines, Point::new(x, y)).expect("a caret position")
    }

    #[test]
    fn a_drag_copies_the_text_between_its_ends() {
        let lines = body();
        let selection = dragged(
            (at(&lines, 51.0, 5.0), at(&lines, 51.0, 5.0)),
            at(&lines, 59.0, 25.0),
        )
        .expect("a selection");
        assert_eq!(selection.text(&lines), "code is 482913");
    }

    #[test]
    fn dragging_backwards_selects_the_same_text() {
        let lines = body();
        let start = at(&lines, 59.0, 25.0);
        let selection = dragged((start, start), at(&lines, 51.0, 5.0)).expect("a selection");
        assert_eq!(selection.text(&lines), "code is 482913");
    }

    #[test]
    fn a_double_click_takes_the_whole_code() {
        let lines = body();
        let (from, to) = word_at(&lines, at(&lines, 31.0, 25.0)).expect("a word");
        let selection = Selection::new(from, to).expect("a selection");
        assert_eq!(selection.text(&lines), "482913");
    }

    #[test]
    fn a_new_block_starts_on_a_new_line() {
        let lines = body();
        let selection = Selection::all(&lines).expect("a selection");
        assert_eq!(selection.text(&lines), "Your code is 482913 today\nThanks");
    }

    #[test]
    fn the_highlight_covers_one_rectangle_per_line() {
        let lines = body();
        let selection = dragged(
            (at(&lines, 51.0, 5.0), at(&lines, 51.0, 5.0)),
            at(&lines, 59.0, 25.0),
        )
        .expect("a selection");
        let rectangles = selection.rectangles(&lines);
        assert_eq!(rectangles.len(), 2);
        assert_eq!(
            rectangles[0],
            Rectangle::new(Point::new(50.0, 0.0), Size::new(80.0, 20.0))
        );
        assert_eq!(
            rectangles[1],
            Rectangle::new(Point::new(0.0, 20.0), Size::new(60.0, 20.0))
        );
    }

    #[test]
    fn a_pointer_in_the_margin_picks_the_line_beside_it() {
        let lines = body();
        assert_eq!(at(&lines, 400.0, 25.0), TextPos { line: 1, cell: 12 });
        assert_eq!(at(&lines, -5.0, 62.0), TextPos { line: 2, cell: 0 });
    }

    #[test]
    fn a_card_sees_only_its_share_of_the_selection() {
        let lines = body();
        let all = Selection::all(&lines).expect("a selection");
        let first_card = all.within(0, 2).expect("the first card is selected");
        assert_eq!(first_card.text(&lines[..2]), "Your code is 482913 today");
        let second_card = all.within(2, 1).expect("the second card is selected");
        assert_eq!(second_card.text(&lines[2..]), "Thanks");
        // Back among the lines of the body, it touches its own card only.
        let in_body = second_card.in_body(2);
        assert!(in_body.touches(2, 1));
        assert!(!in_body.touches(0, 2));
        assert_eq!(in_body.within(0, 2), None);
    }

    #[test]
    fn a_click_without_a_drag_selects_nothing() {
        let lines = body();
        let caret = at(&lines, 31.0, 25.0);
        assert_eq!(dragged((caret, caret), caret), None);
    }
}
