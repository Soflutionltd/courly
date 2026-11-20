//! Rich text editor widget for iced.
//!
//! Custom `Widget` trait implementation. Renders blocks as `Paragraph::with_spans`,
//! handles keyboard/mouse/IME input, cursor blink, and selection.
//!
//! # Usage
//!
//! ```ignore
//! use rte::widget::{EditorState, RichTextEditor, Action};
//!
//! struct State {
//!     editor: EditorState,
//! }
//!
//! #[derive(Debug, Clone)]
//! enum Message {
//!     EditorAction(Action),
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     rich_text_editor(&state.editor)
//!         .on_action(Message::EditorAction)
//!         .into()
//! }
//!
//! fn update(state: &mut State, message: Message) {
//!     match message {
//!         Message::EditorAction(action) => {
//!             state.editor.perform(action);
//!         }
//!     }
//! }
//! ```

pub mod cursor;
mod editor_state;
pub mod input;
pub mod render;

pub use editor_state::{Action, EditorState};

use crate::document::{Block, CellStep, Table};

use cursor::{BlockSelectionKind, CURSOR_WIDTH, SELECTION_ALPHA, SelectionRect};
use input::KeyAction;
use render::ParagraphCache;

use iced::advanced::Shell;
use iced::advanced::clipboard;
use iced::advanced::image as iced_image;
use iced::advanced::image::Renderer as _;
use iced::advanced::layout;
use iced::advanced::mouse::click::Click;
use iced::advanced::renderer;
use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Paragraph as _;
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Widget};
use iced::keyboard;
use iced::mouse;
use iced::time::{Duration, Instant};
use iced::window;
use iced::{Color, Element, Event, Font, Length, Padding, Point, Rectangle, Size, Vector};
use std::collections::HashMap;

/// The paragraph type used by the iced Renderer.
type IcedParagraph = <iced::Renderer as TextRenderer>::Paragraph;

/// Whether a height grows with the document instead of taking the space
/// it is offered.
fn is_content_sized(height: Length) -> bool {
    !matches!(height, Length::Fixed(_)) && height.fill_factor() == 0
}

/// The scroll offset after a wheel movement, kept inside the document.
fn scrolled_offset(current: f32, delta: f32, total_height: f32, viewport_height: f32) -> f32 {
    let max_scroll = (total_height - viewport_height).max(0.0);
    (current + delta).clamp(0.0, max_scroll)
}

// ── Widget tree state ───────────────────────────────────

/// Internal widget state stored in the iced widget tree. Holds the paragraph
/// cache and focus/blink timing.
struct WidgetState {
    /// Paragraph cache: one entry per document block.
    cache: ParagraphCache<IcedParagraph>,
    /// Focus tracking for cursor blink.
    focus: Option<FocusState>,
    /// Last mouse click for double/triple click detection.
    last_click: Option<Click>,
    /// Whether a drag is active.
    dragging: bool,
    /// Vertical scroll offset in pixels. 0.0 means the top of the document
    /// is aligned with the top of the viewport.
    scroll_offset: f32,
    /// Set when a paste has requested a clipboard read but the contents have
    /// not yet arrived. The runtime delivers the read asynchronously via a
    /// `clipboard::Event::Read`; this flags that we are the widget waiting on
    /// it, and for which format, so we know what to emit when it lands.
    pending_paste: Option<PendingPaste>,
    /// The images of the document, decoded once per source. `None` for a
    /// source that holds no bytes (remote, `cid:`), shown as a placeholder.
    images: HashMap<String, Option<ImageSlot>>,
    /// The image whose corner is being dragged, if any.
    resizing: Option<ImageResize>,
    /// The modifier keys held, as last reported.
    modifiers: keyboard::Modifiers,
    /// Where the last move up or down left the caret, and the horizontal
    /// position it aims for: a run of such moves keeps its column through
    /// shorter lines. Stale as soon as the caret is anywhere else.
    vertical_x: Option<(crate::document::DocPosition, f32)>,
}

/// Where a run of blocks is laid out: the blocks of the document, or those
/// of a table cell.
#[derive(Clone, Copy)]
struct Placed<'a> {
    cache: &'a ParagraphCache<IcedParagraph>,
    /// Top left corner of the blocks, the document's own being the origin.
    origin: Point,
    /// Width the blocks are laid out in (px).
    width: f32,
}

impl Placed<'_> {
    /// A point of the document's content, relative to these blocks.
    fn local(&self, content_pos: Point) -> Point {
        Point::new(content_pos.x - self.origin.x, content_pos.y - self.origin.y)
    }

    /// The layout of the cell at `row`, `column` of the table that is block
    /// `block_index` of these blocks.
    fn cell(&self, step: CellStep) -> Option<Self> {
        let entry = self.cache.get(step.block_index)?;
        let cell = entry.table()?.cell(step.row, step.column)?;
        let table_origin = Point::new(self.origin.x, self.origin.y + entry.y_offset());
        Some(Placed {
            cache: &cell.cache,
            origin: Point::new(
                table_origin.x + cell.content_origin.x,
                table_origin.y + cell.content_origin.y,
            ),
            width: cell.content_width,
        })
    }
}

/// The layout of the blocks `scope` leads to, `width` being the width of
/// the document's own.
fn placed_at<'a>(
    cache: &'a ParagraphCache<IcedParagraph>,
    width: f32,
    scope: &[CellStep],
) -> Option<Placed<'a>> {
    scope.iter().try_fold(
        Placed {
            cache,
            origin: Point::ORIGIN,
            width,
        },
        |placed, step| placed.cell(*step),
    )
}

/// The table cell under a point of the content, however deep, as the steps
/// leading to it, with the layout of its blocks. No step, and the layout
/// of the document's own blocks, when the point is on none.
fn scope_at(
    cache: &ParagraphCache<IcedParagraph>,
    width: f32,
    content_pos: Point,
) -> (Vec<CellStep>, Placed<'_>) {
    let mut scope = Vec::new();
    let mut placed = Placed {
        cache,
        origin: Point::ORIGIN,
        width,
    };
    loop {
        let local = placed.local(content_pos);
        let step = placed.cache.block_at_y(local.y).and_then(|block_index| {
            let entry = placed.cache.get(block_index)?;
            let (row, column) = entry
                .table()?
                .cell_at(Point::new(local.x, local.y - entry.y_offset()))?;
            Some(CellStep {
                block_index,
                row,
                column,
            })
        });
        let Some(inner) = step.and_then(|step| Some((step, placed.cell(step)?))) else {
            return (scope, placed);
        };
        scope.push(inner.0);
        placed = inner.1;
    }
}

/// An image source decoded for drawing.
struct ImageSlot {
    handle: iced_image::Handle,
    /// Pixel size read from the file header, `None` when the format is not
    /// one whose header is read: the renderer is then asked.
    size: Option<(u32, u32)>,
}

/// A resize in progress: the corner of the selected image follows the
/// pointer, and the document only changes when the button is released.
#[derive(Debug, Clone, Copy)]
struct ImageResize {
    block_index: usize,
    /// The size the image was shown at when the drag started.
    start: Size,
    /// Where the pointer was when the drag started.
    pointer_start: Point,
    /// The size the drag has reached, in pixels.
    current: (u32, u32),
}

/// Height of the area the blocks of a table cell are drawn in: tall enough
/// to never cut a cell short (px).
const NESTED_AREA_HEIGHT: f32 = 1_000_000.0;

/// Side of the square handle at the corner of a selected image (px).
const IMAGE_HANDLE_SIZE: f32 = 10.0;
/// How far around that handle a press still grabs it (px).
const IMAGE_HANDLE_GRAB_MARGIN: f32 = 4.0;

/// Decode the images that entered the document and forget those that left.
fn refresh_images(
    images: &mut HashMap<String, Option<ImageSlot>>,
    document: &crate::document::Document,
) {
    let sources = image_sources(document);
    for src in &sources {
        if !images.contains_key(*src) {
            let slot = crate::image::data_uri_bytes(src).map(|bytes| ImageSlot {
                size: crate::image::pixel_size(&bytes),
                handle: iced_image::Handle::from_bytes(bytes),
            });
            images.insert((*src).to_owned(), slot);
        }
    }
    if images.len() > sources.len() {
        images.retain(|known, _| sources.contains(&known.as_str()));
    }
}

/// The source of every image of the document, image blocks and images in a
/// line of text alike, each listed once.
fn image_sources(document: &crate::document::Document) -> Vec<&str> {
    let mut sources: Vec<&str> = Vec::new();
    collect_image_sources(&document.blocks, &mut sources);
    sources
}

fn collect_image_sources<'a>(blocks: &'a [std::sync::Arc<Block>], sources: &mut Vec<&'a str>) {
    for block in blocks {
        match block.as_ref() {
            Block::Image { src, .. } => add_image_source(sources, src),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_image_sources(&cell.blocks, sources);
                    }
                }
            }
            block => {
                for run in block.runs().unwrap_or(&[]) {
                    if let Some(image) = run.format.image {
                        add_image_source(sources, &image.data().src);
                    }
                }
            }
        }
    }
}

fn add_image_source<'a>(sources: &mut Vec<&'a str>, src: &'a str) {
    if !sources.contains(&src) {
        sources.push(src);
    }
}

/// The clipboard format a paste is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingPaste {
    /// The HTML form, asked first when the host takes HTML pastes.
    Html,
    /// The plain text form.
    Text,
}

/// Focus timing state for cursor blink.
#[derive(Debug, Clone)]
struct FocusState {
    updated_at: Instant,
    now: Instant,
    is_window_focused: bool,
}

impl FocusState {
    const BLINK_INTERVAL_MILLIS: u128 = 500;

    fn now() -> Self {
        let now = Instant::now();
        Self {
            updated_at: now,
            now,
            is_window_focused: true,
        }
    }

    fn is_cursor_visible(&self) -> bool {
        self.is_window_focused
            && ((self.now - self.updated_at).as_millis() / Self::BLINK_INTERVAL_MILLIS)
                .is_multiple_of(2)
    }
}

// ── RichTextEditor widget ───────────────────────────────

/// A rich text editor widget for iced.
///
/// Created via the [`rich_text_editor`] function. Renders a [`Document`] with
/// styled text, handles keyboard and mouse input, and emits [`Action`]s to the
/// application.
pub struct RichTextEditor<'a, Message> {
    state: &'a EditorState,
    on_action: Option<Box<dyn Fn(Action) -> Message + 'a>>,
    on_paste_html: Option<Box<dyn Fn(String) -> Message + 'a>>,
    font: Font,
    text_color: Color,
    link_color: Color,
    cursor_color: Color,
    selection_color: Color,
    padding: Padding,
    width: Length,
    height: Length,
}

/// Create a [`RichTextEditor`] widget for the given state.
pub fn rich_text_editor<'a, Message>(state: &'a EditorState) -> RichTextEditor<'a, Message> {
    RichTextEditor::new(state)
}

impl<'a, Message> RichTextEditor<'a, Message> {
    /// Create a new rich text editor widget.
    pub fn new(state: &'a EditorState) -> Self {
        Self {
            state,
            on_action: None,
            on_paste_html: None,
            font: Font::DEFAULT,
            text_color: Color::BLACK,
            link_color: Color::from_rgb(0.2, 0.4, 0.8),
            cursor_color: Color::BLACK,
            selection_color: Color::from_rgba(0.2, 0.4, 0.8, SELECTION_ALPHA),
            padding: Padding::new(8.0),
            width: Length::Fill,
            height: Length::Shrink,
        }
    }

    /// Set the callback for when an action occurs.
    ///
    /// If not set, the editor is disabled (read-only).
    pub fn on_action(mut self, f: impl Fn(Action) -> Message + 'a) -> Self {
        self.on_action = Some(Box::new(f));
        self
    }

    /// Take pastes that carry HTML as HTML.
    ///
    /// A paste then asks the clipboard for its HTML form first and hands it
    /// to `f`, leaving the document alone: the host decides what the HTML
    /// becomes. A clipboard without HTML pastes its text as usual.
    pub fn on_paste_html(mut self, f: impl Fn(String) -> Message + 'a) -> Self {
        self.on_paste_html = Some(Box::new(f));
        self
    }

    /// Set the base font.
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// Set the text color.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set the link color.
    pub fn link_color(mut self, color: Color) -> Self {
        self.link_color = color;
        self
    }

    /// Set the cursor (caret) color.
    pub fn cursor_color(mut self, color: Color) -> Self {
        self.cursor_color = color;
        self
    }

    /// Set the selection highlight color.
    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    /// Set the padding.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Set the width.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Set the height.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

// ── Event handling helpers ───────────────────────────────

impl<Message> RichTextEditor<'_, Message> {
    /// Handle window focus/unfocus and redraw-requested events for cursor blink.
    fn handle_window_events(
        widget_state: &mut WidgetState,
        event: &Event,
        shell: &mut Shell<'_, Message>,
    ) {
        match event {
            Event::Window(window::Event::Unfocused) => {
                if let Some(focus) = &mut widget_state.focus {
                    focus.is_window_focused = false;
                }
            }
            Event::Window(window::Event::Focused) => {
                if let Some(focus) = &mut widget_state.focus {
                    focus.is_window_focused = true;
                    focus.updated_at = Instant::now();
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(focus) = &mut widget_state.focus
                    && focus.is_window_focused
                {
                    focus.now = *now;

                    let elapsed = (focus.now - focus.updated_at).as_millis()
                        % FocusState::BLINK_INTERVAL_MILLIS;
                    let millis_until_redraw =
                        FocusState::BLINK_INTERVAL_MILLIS.saturating_sub(elapsed);

                    shell.request_redraw_at(
                        focus.now
                            + Duration::from_millis(
                                u64::try_from(millis_until_redraw).unwrap_or(500),
                            ),
                    );
                }
            }
            _ => {}
        }
    }

    /// Handle keyboard events (key presses) when the editor is focused.
    fn handle_keyboard(
        &self,
        widget_state: &mut WidgetState,
        event: &Event,
        on_action: &dyn Fn(Action) -> Message,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            widget_state.modifiers = *modifiers;
        }

        // In a table cell, Tab goes from cell to cell and Escape leaves the
        // cell for the table as a whole.
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event
            && widget_state.focus.is_some()
            && !self.state.scope().is_empty()
            && let keyboard::Key::Named(named) = key
        {
            let action = match named {
                keyboard::key::Named::Tab => Some(Action::MoveCell {
                    forward: !modifiers.shift(),
                }),
                keyboard::key::Named::Escape => Some(Action::LeaveCell),
                _ => None,
            };
            if let Some(action) = action {
                shell.publish(on_action(action));
                shell.capture_event();
                if let Some(focus) = &mut widget_state.focus {
                    focus.updated_at = Instant::now();
                }
                return;
            }
        }

        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) = event
            && widget_state.focus.is_some()
        {
            let text_str = text.as_deref();
            let key_action = input::map_key_event(key, *modifiers, text_str);

            match key_action {
                KeyAction::Edit(edit_action) => {
                    shell.publish(on_action(Action::Edit(edit_action)));
                    shell.capture_event();
                }
                KeyAction::Move(move_action) => {
                    let action = self
                        .line_move(widget_state, move_action, false)
                        .unwrap_or(Action::Move(move_action));
                    shell.publish(on_action(action));
                    shell.capture_event();
                }
                KeyAction::Select(move_action) => {
                    let action = self
                        .line_move(widget_state, move_action, true)
                        .unwrap_or(Action::Select(move_action));
                    shell.publish(on_action(action));
                    shell.capture_event();
                }
                KeyAction::SelectAll => {
                    shell.publish(on_action(Action::SelectAll));
                    shell.capture_event();
                }
                KeyAction::Copy => {
                    let text = self.state.selection_text();
                    if !text.is_empty() {
                        shell.write_clipboard(clipboard::Content::Text(text));
                    }
                    // Emit Action::Copy so EditorState captures the structured slice.
                    shell.publish(on_action(Action::Copy));
                    shell.capture_event();
                }
                KeyAction::Cut => {
                    let text = self.state.selection_text();
                    if !text.is_empty() {
                        shell.write_clipboard(clipboard::Content::Text(text));
                    }
                    // Emit Action::Cut unconditionally: the selection may have
                    // no plain-text form (an image with empty alt) but must
                    // still be captured and deleted. EditorState no-ops on a
                    // collapsed selection.
                    shell.publish(on_action(Action::Cut));
                    shell.capture_event();
                }
                KeyAction::Paste => {
                    // Clipboard reads are asynchronous: request the contents and
                    // remember we are waiting. The paste is emitted from
                    // `handle_clipboard` once the runtime delivers the read.
                    let pending = if self.on_paste_html.is_some() {
                        shell.read_clipboard(clipboard::Kind::Html);
                        PendingPaste::Html
                    } else {
                        shell.read_clipboard(clipboard::Kind::Text);
                        PendingPaste::Text
                    };
                    widget_state.pending_paste = Some(pending);
                    shell.capture_event();
                }
                KeyAction::Undo => {
                    shell.publish(on_action(Action::Undo));
                    shell.capture_event();
                }
                KeyAction::Redo => {
                    shell.publish(on_action(Action::Redo));
                    shell.capture_event();
                }
                KeyAction::None => {}
            }

            // Reset blink on any handled key.
            if let Some(focus) = &mut widget_state.focus {
                focus.updated_at = Instant::now();
            }
        }
    }

    /// Handle an asynchronous clipboard read completing. When a paste has
    /// requested a read (`pending_paste`), emit `Action::Paste` with the
    /// delivered text, or hand delivered HTML to `on_paste_html`. A clipboard
    /// that has no HTML form is read again as text.
    fn handle_clipboard(
        &self,
        widget_state: &mut WidgetState,
        event: &Event,
        on_action: &dyn Fn(Action) -> Message,
        shell: &mut Shell<'_, Message>,
    ) {
        let Event::Clipboard(clipboard::Event::Read(read)) = event else {
            return;
        };
        let Some(pending) = widget_state.pending_paste.take() else {
            return;
        };
        let content = read.as_ref().ok().map(AsRef::as_ref);
        match (pending, content, self.on_paste_html.as_ref()) {
            (PendingPaste::Html, Some(clipboard::Content::Html(html)), Some(on_paste_html)) => {
                shell.publish(on_paste_html(html.clone()));
                shell.capture_event();
            }
            (PendingPaste::Html, _, _) => {
                shell.read_clipboard(clipboard::Kind::Text);
                widget_state.pending_paste = Some(PendingPaste::Text);
            }
            (PendingPaste::Text, Some(clipboard::Content::Text(text)), _) => {
                shell.publish(on_action(Action::Paste(text.clone())));
                shell.capture_event();
            }
            (PendingPaste::Text, _, _) => {}
        }
    }

    /// Handle a drag move: auto-scroll when above/below the viewport,
    /// normal drag within bounds.
    #[allow(clippy::too_many_arguments)]
    fn handle_drag(
        &self,
        widget_state: &mut WidgetState,
        position: Point,
        bounds: Rectangle,
        cursor_pos: mouse::Cursor,
        on_action: &dyn Fn(Action) -> Message,
        shell: &mut Shell<'_, Message>,
    ) {
        if position.y < bounds.y {
            // Cursor is above the viewport - scroll up.
            let overshoot = bounds.y - position.y;
            let scroll_amount = (overshoot * DRAG_SCROLL_SPEED).min(DRAG_SCROLL_MAX);
            widget_state.scroll_offset = (widget_state.scroll_offset - scroll_amount).max(0.0);

            // Hit-test at the top of the (freshly scrolled) viewport, not the
            // top of the document - mirroring the below-viewport branch.
            let content_pos = Point::new(
                position.x - bounds.x - self.padding.left,
                widget_state.scroll_offset,
            );
            let doc_pos = self.position_in_scope(widget_state, content_pos);
            shell.publish(on_action(Action::Drag(doc_pos)));
            shell.request_redraw();
            shell.capture_event();
        } else if position.y > bounds.y + bounds.height {
            // Cursor is below the viewport - scroll down.
            let overshoot = position.y - (bounds.y + bounds.height);
            let scroll_amount = (overshoot * DRAG_SCROLL_SPEED).min(DRAG_SCROLL_MAX);
            let viewport_height = bounds.height - self.padding.top - self.padding.bottom;
            let total_height = widget_state.cache.total_height();
            let max_scroll = (total_height - viewport_height).max(0.0);
            widget_state.scroll_offset =
                (widget_state.scroll_offset + scroll_amount).min(max_scroll);

            let content_pos = Point::new(
                position.x - bounds.x - self.padding.left,
                widget_state.scroll_offset + viewport_height,
            );
            let doc_pos = self.position_in_scope(widget_state, content_pos);
            shell.publish(on_action(Action::Drag(doc_pos)));
            shell.request_redraw();
            shell.capture_event();
        } else if let Some(rel_pos) = cursor_pos.position_in(bounds) {
            // Normal drag within bounds.
            let content_pos = Point::new(
                rel_pos.x - self.padding.left,
                rel_pos.y - self.padding.top + widget_state.scroll_offset,
            );
            let doc_pos = self.position_in_scope(widget_state, content_pos);
            shell.publish(on_action(Action::Drag(doc_pos)));
        }
    }

    /// Up or down as a move to the line above or below, as the text is laid
    /// out: within a paragraph that wraps, then into the last or first line
    /// of the neighbouring one, keeping the horizontal position of the first
    /// move. `None` when the move is not vertical or has no line of text to
    /// land on (an image, a table, the edge of the document): the editor
    /// then moves by block.
    fn line_move(
        &self,
        widget_state: &mut WidgetState,
        move_action: input::MoveAction,
        extend: bool,
    ) -> Option<Action> {
        let down = match move_action {
            input::MoveAction::Down => true,
            input::MoveAction::Up => false,
            _ => return None,
        };
        let focus = self.state.selection.focus;
        let placed = placed_at(&widget_state.cache, 0.0, self.state.scope())?;
        let document = self.state.active_document();
        let x_offset = |block_index: usize| -> Option<f32> {
            let entry = placed.cache.get(block_index)?;
            let block = document.block(block_index)?;
            Some(block_content_x_offset(block) + entry.x_shift())
        };

        let entry = placed.cache.get(focus.block_index)?;
        let paragraph = entry.paragraph()?;
        let caret = grapheme_pixel_position(paragraph, focus.offset);
        let line_height = paragraph_line_height_px(paragraph);
        let x = match widget_state.vertical_x {
            Some((at, x)) if at == focus => x,
            _ => caret.x + x_offset(focus.block_index)?,
        };

        // The middle of the line above or below, in the same paragraph.
        let line_y = if down {
            caret.y + line_height * 1.5
        } else {
            caret.y - line_height * 0.5
        };
        let (block_index, paragraph, y) = if line_y > 0.0 && line_y < paragraph.min_bounds().height
        {
            (focus.block_index, paragraph, line_y)
        } else if down {
            let next = focus.block_index + 1;
            let paragraph = placed.cache.get(next)?.paragraph()?;
            (next, paragraph, paragraph_line_height_px(paragraph) * 0.5)
        } else {
            let previous = focus.block_index.checked_sub(1)?;
            let paragraph = placed.cache.get(previous)?.paragraph()?;
            let last_line =
                paragraph.min_bounds().height - paragraph_line_height_px(paragraph) * 0.5;
            (previous, paragraph, last_line.max(0.0))
        };
        let offset = offset_at_point(paragraph, Point::new(x - x_offset(block_index)?, y))?;
        let position = crate::document::DocPosition::new(block_index, offset);
        if position == focus {
            return None;
        }
        widget_state.vertical_x = Some((position, x));
        Some(Action::MoveToLine { position, extend })
    }

    /// The position under a point of the content, among the blocks being
    /// edited: those of the table cell holding the caret, or the
    /// document's. A drag never leaves them.
    fn position_in_scope(
        &self,
        widget_state: &WidgetState,
        content_pos: Point,
    ) -> crate::document::DocPosition {
        let Some(placed) = placed_at(&widget_state.cache, 0.0, self.state.scope()) else {
            return crate::document::DocPosition::zero();
        };
        hit_test_content_point(
            placed.local(content_pos),
            placed.cache,
            &self.state.active_document(),
        )
    }

    /// Handle mouse events (clicks, drags, scroll wheel).
    #[allow(clippy::too_many_arguments)]
    fn handle_mouse(
        &self,
        widget_state: &mut WidgetState,
        event: &Event,
        bounds: Rectangle,
        cursor_pos: mouse::Cursor,
        on_action: &dyn Fn(Action) -> Message,
        shell: &mut Shell<'_, Message>,
    ) {
        let scroll_offset = widget_state.scroll_offset;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Left,
                ..
            }) => {
                if let Some(position) = cursor_pos.position_in(bounds) {
                    // Translate to content coordinates (account for padding and scroll).
                    let content_pos = Point::new(
                        position.x - self.padding.left,
                        position.y - self.padding.top + scroll_offset,
                    );

                    let content_width = bounds.width - self.padding.left - self.padding.right;

                    // The corner handle of the selected image starts a resize.
                    if widget_state.focus.is_some()
                        && let Some(placed) =
                            placed_at(&widget_state.cache, content_width, self.state.scope())
                        && let Some((block_index, shown)) =
                            selected_image_rect(self.state, placed.cache)
                        && image_handle_rect(shown)
                            .expand(IMAGE_HANDLE_GRAB_MARGIN)
                            .contains(placed.local(content_pos))
                        && let Some(pointer_start) = cursor_pos.position()
                    {
                        widget_state.resizing = Some(ImageResize {
                            block_index,
                            start: shown.size(),
                            pointer_start,
                            current: shown_pixels(shown.size()),
                        });
                        widget_state.dragging = false;
                        shell.capture_event();
                        return;
                    }

                    // The blocks under the pointer: those of a table cell,
                    // however deep, or the document's own.
                    let (scope, placed) = scope_at(&widget_state.cache, content_width, content_pos);
                    let content_pos = placed.local(content_pos);
                    let document = self
                        .state
                        .document
                        .cell_document(&scope)
                        .unwrap_or_else(|| self.state.document.clone());
                    let doc_pos = hit_test_content_point(content_pos, placed.cache, &document);

                    // Detect double/triple click using iced's Click type.
                    let click = Click::new(
                        position,
                        mouse::Button::Left,
                        widget_state.last_click.take(),
                    );
                    let click_kind = click.kind();
                    widget_state.last_click = Some(click);

                    widget_state.focus = Some(FocusState::now());

                    shell.publish(on_action(Action::Focus));
                    if scope != self.state.scope() {
                        shell.publish(on_action(Action::EnterCell(scope)));
                    }

                    match click_kind {
                        iced::advanced::mouse::click::Kind::Triple => {
                            // Triple click: select entire block.
                            widget_state.dragging = false;
                            shell.publish(on_action(Action::TripleClick(doc_pos)));
                        }
                        iced::advanced::mouse::click::Kind::Double => {
                            // Double click: select word.
                            widget_state.dragging = false;
                            shell.publish(on_action(Action::DoubleClick(doc_pos)));
                        }
                        iced::advanced::mouse::click::Kind::Single => {
                            if let Some(block_index) =
                                image_at_content_point(content_pos, placed.cache)
                            {
                                widget_state.dragging = false;
                                shell.publish(on_action(Action::SelectImage(block_index)));
                            } else if widget_state.modifiers.command()
                                && let Some(href) =
                                    link_at_content_point(content_pos, placed.cache, &document)
                            {
                                // A link is followed with the command key
                                // held; a plain click puts the caret in its
                                // text, so that it can be edited.
                                widget_state.dragging = false;
                                shell.publish(on_action(Action::LinkClicked(href)));
                            } else {
                                widget_state.dragging = true;
                                shell.publish(on_action(Action::Click(doc_pos)));
                            }
                        }
                    }

                    shell.capture_event();
                } else if widget_state.focus.is_some() {
                    // Click outside the editor: blur.
                    widget_state.focus = None;
                    widget_state.dragging = false;
                    shell.publish(on_action(Action::Blur));
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if widget_state.resizing.is_some() => {
                if let Some(resize) = widget_state.resizing.as_mut()
                    && let Some(position) = cursor_pos.position()
                {
                    let max_width = bounds.width - self.padding.left - self.padding.right;
                    resize.current = crate::image::resized(
                        (resize.start.width, resize.start.height),
                        (
                            position.x - resize.pointer_start.x,
                            position.y - resize.pointer_start.y,
                        ),
                        max_width,
                    );
                    shell.request_redraw();
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
                if widget_state.dragging
                    && let Some(position) = cursor_pos.position() =>
            {
                self.handle_drag(widget_state, position, bounds, cursor_pos, on_action, shell);
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor_pos.is_over(bounds) => {
                let delta_y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * SCROLL_LINE_HEIGHT,
                    mouse::ScrollDelta::Pixels { y, .. } => -y,
                };

                let total_height = widget_state.cache.total_height();
                let viewport_height = bounds.height - self.padding.top - self.padding.bottom;
                let offset = scrolled_offset(
                    widget_state.scroll_offset,
                    delta_y,
                    total_height,
                    viewport_height,
                );

                // An editor sized to its text, or already at the end it is
                // scrolled towards, leaves the wheel to the page around it.
                if (offset - widget_state.scroll_offset).abs() > f32::EPSILON {
                    widget_state.scroll_offset = offset;
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                widget_state.dragging = false;
                // The document takes the new size once, when the corner is
                // let go, so that a resize is a single undo step.
                if let Some(resize) = widget_state.resizing.take() {
                    if resize.current != shown_pixels(resize.start) {
                        shell.publish(on_action(Action::Edit(
                            crate::rules::EditAction::ResizeImage {
                                block_index: resize.block_index,
                                width: resize.current.0,
                                height: resize.current.1,
                            },
                        )));
                    }
                    shell.request_redraw();
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }
}

// ── Drawing helper ──────────────────────────────────────

impl<Message> RichTextEditor<'_, Message> {
    /// Draw all document content (blocks, selection, cursor) into the
    /// current renderer context. Called from `draw()` inside a layer +
    /// translation so scrolling and clipping are already applied.
    fn draw_content(
        &self,
        renderer: &mut iced::Renderer,
        widget_state: &WidgetState,
        text_bounds: &Rectangle,
        cursor_visible: bool,
    ) {
        let cache = &widget_state.cache;
        let blocks = &self.state.document.blocks;
        self.draw_highlights(renderer, blocks, cache, text_bounds);
        self.draw_blocks(renderer, blocks, cache, &widget_state.images, text_bounds);

        // The selection and the caret belong to the blocks being edited:
        // those of a table cell, or the document's own.
        let Some(placed) = placed_at(cache, text_bounds.width, self.state.scope()) else {
            return;
        };
        let document = self.state.active_document();
        let area = Rectangle::new(
            Point::new(
                text_bounds.x + placed.origin.x,
                text_bounds.y + placed.origin.y,
            ),
            Size::new(placed.width, text_bounds.height),
        );
        self.draw_selection(renderer, &document, placed.cache, &area);
        self.draw_cursor(renderer, &document, placed.cache, &area, cursor_visible);
        self.draw_image_frame(renderer, widget_state, placed.cache, &area);
    }

    /// Draw a table: its background, then each cell's background, borders
    /// and blocks.
    fn draw_table(
        &self,
        renderer: &mut iced::Renderer,
        table: &Table,
        layout: &render::TableLayout<IcedParagraph>,
        origin: Point,
        images: &HashMap<String, Option<ImageSlot>>,
    ) {
        let fill = |renderer: &mut iced::Renderer, bounds: Rectangle, color: Color| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    ..Default::default()
                },
                color,
            );
        };
        let color_of = |color: crate::document::Rgb| Color::from_rgb8(color.r, color.g, color.b);

        if let Some(background) = layout.background {
            fill(
                renderer,
                Rectangle::new(origin, Size::new(layout.width, layout.height)),
                color_of(background),
            );
        }
        for (cells, row) in layout.rows.iter().zip(&table.rows) {
            for (cell_layout, cell) in cells.iter().zip(&row.cells) {
                let bounds = Rectangle::new(
                    Point::new(
                        origin.x + cell_layout.bounds.x,
                        origin.y + cell_layout.bounds.y,
                    ),
                    cell_layout.bounds.size(),
                );
                if let Some(background) = cell_layout.cell_box.background {
                    fill(renderer, bounds, color_of(background));
                }

                let border = cell_layout.cell_box.border;
                let line_color = |line: crate::table_box::BorderLine| {
                    line.color.map_or(self.text_color, color_of)
                };
                if let Some(line) = border.top {
                    let side =
                        Rectangle::new(bounds.position(), Size::new(bounds.width, line.width));
                    fill(renderer, side, line_color(line));
                }
                if let Some(line) = border.bottom {
                    let side = Rectangle::new(
                        Point::new(bounds.x, bounds.y + bounds.height - line.width),
                        Size::new(bounds.width, line.width),
                    );
                    fill(renderer, side, line_color(line));
                }
                if let Some(line) = border.left {
                    let side =
                        Rectangle::new(bounds.position(), Size::new(line.width, bounds.height));
                    fill(renderer, side, line_color(line));
                }
                if let Some(line) = border.right {
                    let side = Rectangle::new(
                        Point::new(bounds.x + bounds.width - line.width, bounds.y),
                        Size::new(line.width, bounds.height),
                    );
                    fill(renderer, side, line_color(line));
                }

                let area = Rectangle::new(
                    Point::new(
                        origin.x + cell_layout.content_origin.x,
                        origin.y + cell_layout.content_origin.y,
                    ),
                    Size::new(cell_layout.content_width, NESTED_AREA_HEIGHT),
                );
                self.draw_highlights(renderer, &cell.blocks, &cell_layout.cache, &area);
                self.draw_blocks(renderer, &cell.blocks, &cell_layout.cache, images, &area);
            }
        }
    }

    /// Outline the selected image and put a handle on its corner; while that
    /// corner is dragged, outline the size the image is about to take.
    fn draw_image_frame(
        &self,
        renderer: &mut iced::Renderer,
        widget_state: &WidgetState,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
    ) {
        if widget_state.focus.is_none() {
            return;
        }
        let Some((_, shown)) = selected_image_rect(self.state, cache) else {
            return;
        };
        let origin = Point::new(text_bounds.x + shown.x, text_bounds.y + shown.y);
        let size = widget_state.resizing.map_or(shown.size(), |resize| {
            Size::new(resize.current.0 as f32, resize.current.1 as f32)
        });
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(origin, size),
                border: iced::Border {
                    color: self.cursor_color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            },
            Color::TRANSPARENT,
        );
        let handle = image_handle_rect(Rectangle::new(origin, size));
        renderer.fill_quad(
            renderer::Quad {
                bounds: handle,
                ..Default::default()
            },
            self.cursor_color,
        );
    }

    /// Draw the highlight behind every run that carries one. Quoted blocks
    /// are laid out per child and are not reached here.
    fn draw_highlights(
        &self,
        renderer: &mut iced::Renderer,
        blocks: &[std::sync::Arc<Block>],
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
    ) {
        for (index, block) in blocks.iter().enumerate() {
            let Some(runs) = block.runs() else {
                continue;
            };
            if runs.iter().all(|run| run.format.background.is_none()) {
                continue;
            }
            let Some(entry) = cache.get(index) else {
                continue;
            };
            let Some(paragraph) = entry.paragraph() else {
                continue;
            };
            let origin_x = text_bounds.x + block_content_x_offset(block) + entry.x_shift();
            let origin_y = text_bounds.y + entry.y_offset();

            let mut offset = 0;
            for run in runs {
                let start = offset;
                offset += run.char_len();
                let Some(background) = run.format.background else {
                    continue;
                };
                let rects = compute_selection_rects(
                    paragraph,
                    start,
                    offset,
                    origin_x,
                    origin_y,
                    text_bounds.width,
                );
                for rect in &rects {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(rect.x, rect.y),
                                Size::new(rect.width, rect.height),
                            ),
                            ..Default::default()
                        },
                        Color::from_rgb8(background.r, background.g, background.b),
                    );
                }
            }
        }
    }

    /// Draw a single list item block: marker + paragraph content.
    #[allow(clippy::too_many_arguments)]
    fn draw_list_item(
        &self,
        renderer: &mut iced::Renderer,
        paragraph: &IcedParagraph,
        item_idx: usize,
        ordered: bool,
        indent_level: u8,
        block_origin: Point,
        block_height: f32,
        text_bounds: &Rectangle,
    ) {
        let indent =
            render::LIST_MARKER_WIDTH + (indent_level as f32) * render::LIST_INDENT_PER_LEVEL;
        let content_origin = Point::new(block_origin.x + indent, block_origin.y);
        let content_bounds = Rectangle::new(
            content_origin,
            Size::new((text_bounds.width - indent).max(0.0), block_height),
        );
        render::draw_list_marker(
            renderer,
            content_bounds,
            ordered,
            item_idx,
            self.font,
            self.text_color,
            *text_bounds,
        );
        render::draw_paragraph(
            renderer,
            paragraph,
            content_origin,
            self.text_color,
            *text_bounds,
        );
    }

    /// Draw a run of blocks (paragraphs, headings, lists, tables, etc.):
    /// those of the document, or those of a table cell.
    fn draw_blocks(
        &self,
        renderer: &mut iced::Renderer,
        blocks: &[std::sync::Arc<Block>],
        cache: &ParagraphCache<IcedParagraph>,
        images: &HashMap<String, Option<ImageSlot>>,
        text_bounds: &Rectangle,
    ) {
        for (i, block) in blocks.iter().enumerate() {
            let Some(entry) = cache.get(i) else {
                continue;
            };

            let block_origin = Point::new(text_bounds.x, text_bounds.y + entry.y_offset());

            if let Some(runs) = block.runs()
                && let Some(paragraph) = entry.paragraph()
            {
                let origin = Point::new(
                    block_origin.x + block_content_x_offset(block) + entry.x_shift(),
                    block_origin.y,
                );
                self.draw_inline_images(renderer, runs, paragraph, origin, images);
            }

            match block.as_ref() {
                Block::HorizontalRule => {
                    let hr_bounds =
                        Rectangle::new(block_origin, Size::new(text_bounds.width, entry.height()));
                    render::draw_horizontal_rule(renderer, hr_bounds, self.text_color);
                }
                Block::BlockQuote { .. } => {
                    let bq_bounds =
                        Rectangle::new(block_origin, Size::new(text_bounds.width, entry.height()));
                    render::draw_blockquote_border(renderer, bq_bounds, self.text_color);

                    for child in entry.child_paragraphs() {
                        let para_origin = Point::new(
                            block_origin.x + render::BLOCKQUOTE_INDENT,
                            block_origin.y + child.local_y_offset,
                        );
                        render::draw_paragraph(
                            renderer,
                            &child.paragraph,
                            para_origin,
                            self.text_color,
                            *text_bounds,
                        );
                    }
                }
                Block::ListItem {
                    ordered,
                    indent_level,
                    ..
                } => {
                    if let Some(paragraph) = entry.paragraph() {
                        self.draw_list_item(
                            renderer,
                            paragraph,
                            list_item_number(blocks, i, *ordered, *indent_level),
                            *ordered,
                            *indent_level,
                            block_origin,
                            entry.height(),
                            text_bounds,
                        );
                    }
                }
                Block::Paragraph { .. } | Block::Heading { .. } => {
                    if let Some(paragraph) = entry.paragraph() {
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            Point::new(block_origin.x + entry.x_shift(), block_origin.y),
                            self.text_color,
                            *text_bounds,
                        );
                    }
                }
                Block::Table(table) => {
                    if let Some(layout) = entry.table() {
                        self.draw_table(renderer, table, layout, block_origin, images);
                    }
                }
                Block::Image { src, .. } => {
                    if let Some(size) = entry.image_size()
                        && let Some(Some(slot)) = images.get(src)
                    {
                        let bounds = Rectangle::new(block_origin, size);
                        renderer.draw_image(
                            iced_image::Image::new(slot.handle.clone()),
                            bounds,
                            bounds,
                        );
                        continue;
                    }

                    // Draw a placeholder rectangle with a 1px border.
                    let img_bounds =
                        Rectangle::new(block_origin, Size::new(text_bounds.width, entry.height()));
                    let border_color = Color {
                        a: 0.3,
                        ..self.text_color
                    };
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: img_bounds,
                            border: iced::Border {
                                color: border_color,
                                width: 1.0,
                                radius: 2.0.into(),
                            },
                            ..Default::default()
                        },
                        Color::TRANSPARENT,
                    );
                    // Draw alt text inside the placeholder.
                    if let Some(paragraph) = entry.paragraph() {
                        let text_origin = Point::new(
                            block_origin.x + render::IMAGE_PLACEHOLDER_PADDING,
                            block_origin.y + render::IMAGE_PLACEHOLDER_PADDING,
                        );
                        let alt_color = Color {
                            a: 0.5,
                            ..self.text_color
                        };
                        render::draw_paragraph(
                            renderer,
                            paragraph,
                            text_origin,
                            alt_color,
                            *text_bounds,
                        );
                    }
                }
            }
        }
    }

    /// Draw the images sitting in a paragraph's lines, each over the box its
    /// character holds, centered on the line's height. An image whose
    /// source holds no bytes is outlined.
    fn draw_inline_images(
        &self,
        renderer: &mut iced::Renderer,
        runs: &[crate::document::StyledRun],
        paragraph: &IcedParagraph,
        origin: Point,
        images: &HashMap<String, Option<ImageSlot>>,
    ) {
        for (index, run) in runs.iter().enumerate() {
            let Some(image) = run.format.image else {
                continue;
            };
            let Some(held) = paragraph.span_bounds(index).into_iter().next() else {
                continue;
            };
            let (width, height) = image.shown_size();
            let bounds = Rectangle::new(
                Point::new(
                    origin.x + held.x,
                    origin.y + held.y + (held.height - height) / 2.0,
                ),
                Size::new(width, height),
            );
            if let Some(Some(slot)) = images.get(&image.data().src) {
                renderer.draw_image(iced_image::Image::new(slot.handle.clone()), bounds, bounds);
            } else {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: iced::Border {
                            color: Color {
                                a: 0.3,
                                ..self.text_color
                            },
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        ..Default::default()
                    },
                    Color::TRANSPARENT,
                );
            }
        }
    }

    /// Draw selection highlights for the current selection range.
    fn draw_selection(
        &self,
        renderer: &mut iced::Renderer,
        document: &crate::document::Document,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
    ) {
        if self.state.selection.is_collapsed() {
            return;
        }

        let sel_ranges = cursor::selection_block_ranges(self.state.selection);

        for (block_idx, kind) in &sel_ranges {
            let Some(entry) = cache.get(*block_idx) else {
                continue;
            };

            let block_y = text_bounds.y + entry.y_offset();
            let content_x_off = document
                .block(*block_idx)
                .map(block_content_x_offset)
                .unwrap_or(0.0);
            let para_origin_x = text_bounds.x + content_x_off + entry.x_shift();

            // A drawn image or a table is tinted over its own area, when
            // the selection holds it.
            let atom_size = entry.image_size().or_else(|| {
                entry
                    .table()
                    .map(|table| Size::new(table.width, table.height))
            });
            if let Some(size) = atom_size {
                if selection_holds_atom(*kind) {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(Point::new(text_bounds.x, block_y), size),
                            ..Default::default()
                        },
                        self.selection_color,
                    );
                }
                continue;
            }

            let sel_rects: Vec<SelectionRect> = match (kind, entry.paragraph()) {
                (BlockSelectionKind::Full, _) => {
                    vec![SelectionRect {
                        x: text_bounds.x,
                        y: block_y,
                        width: text_bounds.width,
                        height: entry.height(),
                    }]
                }
                (
                    BlockSelectionKind::Single {
                        start_offset,
                        end_offset,
                    },
                    Some(paragraph),
                ) => compute_selection_rects(
                    paragraph,
                    *start_offset,
                    *end_offset,
                    para_origin_x,
                    block_y,
                    text_bounds.width,
                ),
                (BlockSelectionKind::First { start_offset }, Some(paragraph)) => {
                    compute_selection_rects(
                        paragraph,
                        *start_offset,
                        usize::MAX,
                        para_origin_x,
                        block_y,
                        text_bounds.width,
                    )
                }
                (BlockSelectionKind::Last { end_offset }, Some(paragraph)) => {
                    compute_selection_rects(
                        paragraph,
                        0,
                        *end_offset,
                        para_origin_x,
                        block_y,
                        text_bounds.width,
                    )
                }
                (_, None) => {
                    vec![SelectionRect {
                        x: text_bounds.x,
                        y: block_y,
                        width: text_bounds.width,
                        height: entry.height(),
                    }]
                }
            };

            for sel_rect in &sel_rects {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle::new(
                            Point::new(sel_rect.x, sel_rect.y),
                            Size::new(sel_rect.width, sel_rect.height),
                        ),
                        ..Default::default()
                    },
                    self.selection_color,
                );
            }
        }
    }

    /// Draw the cursor caret (blink visibility is determined by the caller).
    fn draw_cursor(
        &self,
        renderer: &mut iced::Renderer,
        document: &crate::document::Document,
        cache: &ParagraphCache<IcedParagraph>,
        text_bounds: &Rectangle,
        cursor_visible: bool,
    ) {
        if !cursor_visible || !self.state.selection.is_collapsed() {
            return;
        }

        let pos = self.state.selection.focus;
        let Some(entry) = cache.get(pos.block_index) else {
            return;
        };

        let content_x_off = document
            .block(pos.block_index)
            .map(block_content_x_offset)
            .unwrap_or(0.0);

        let para_origin_x = text_bounds.x + content_x_off + entry.x_shift();
        let para_origin_y = text_bounds.y + entry.y_offset();

        let atom_size = entry.image_size().or_else(|| {
            entry
                .table()
                .map(|table| Size::new(table.width, table.height))
        });
        let (cursor_x, cursor_y, cursor_height) = if let Some(size) = atom_size {
            // Before or after the image or the table, as tall as it is.
            let x = if pos.offset == 0 { 0.0 } else { size.width };
            (para_origin_x + x, para_origin_y, size.height)
        } else if let Some(paragraph) = entry.paragraph() {
            let gp = grapheme_pixel_position(paragraph, pos.offset);
            let lh = paragraph_line_height_px(paragraph);
            (para_origin_x + gp.x, para_origin_y + gp.y, lh)
        } else {
            let lh = render::FONT_SIZE_BODY * render::LINE_HEIGHT_MULTIPLIER;
            (para_origin_x, para_origin_y, lh)
        };

        // Draw the cursor directly. We're inside with_layer(bounds)
        // which clips to the viewport, so no manual intersection
        // check is needed. (The previous check against text_bounds
        // used viewport-space coordinates inside a content-space
        // translation, causing the cursor to disappear when scrolled.)
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(CURSOR_WIDTH, cursor_height),
                ),
                ..Default::default()
            },
            self.cursor_color,
        );
    }
}

// ── Widget trait implementation ─────────────────────────

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for RichTextEditor<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<WidgetState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(WidgetState {
            cache: ParagraphCache::new(),
            focus: None,
            last_click: None,
            dragging: false,
            scroll_offset: 0.0,
            pending_paste: None,
            images: HashMap::new(),
            resizing: None,
            modifiers: keyboard::Modifiers::default(),
            vertical_x: None,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) {
        let widget_state = tree.state.downcast_mut::<WidgetState>();
        refresh_images(&mut widget_state.images, &self.state.document);
        let images = &widget_state.images;
        let natural_size = |src: &str| {
            let slot = images.get(src)?.as_ref()?;
            slot.size.or_else(|| {
                renderer
                    .measure_image(&slot.handle)
                    .map(|size| (size.width, size.height))
            })
        };
        let cache = &mut widget_state.cache;

        let limits = limits.width(self.width).height(self.height);
        let max_size = limits.bounds();
        let available_width = (max_size.width - self.padding.left - self.padding.right).max(0.0);

        // Layout paragraphs using the cache.
        let total_height = cache.layout(
            &self.state.document.blocks,
            available_width,
            self.font,
            self.text_color,
            self.link_color,
            &natural_size,
        );

        let content_height = total_height + self.padding.top + self.padding.bottom;

        let size = if is_content_sized(self.height) {
            // The widget is as tall as its document, within any bounds the
            // length carries (`Length::Shrink.min(..)`).
            limits.height(Length::Fixed(content_height)).bounds()
        } else {
            // Fill-like: the widget takes the offered height and scrolls.
            limits.bounds()
        };
        tree.size = size;

        // Clamp scroll offset to valid range after layout.
        let viewport_height = size.height - self.padding.top - self.padding.bottom;
        let max_scroll = (total_height - viewport_height).max(0.0);
        widget_state.scroll_offset = widget_state.scroll_offset.clamp(0.0, max_scroll);

        // Auto-scroll to keep the cursor visible after edits/moves.
        // Use the actual caret line position within the block (not just
        // the block top) so scrolling works correctly in wrapped paragraphs.
        if self.state.cursor.is_focused() {
            let focus_pos = self.state.selection.focus;
            // The caret may be in a table cell: its blocks start lower down.
            if let Some(placed) = placed_at(cache, available_width, self.state.scope())
                && let Some(entry) = placed.cache.get(focus_pos.block_index)
            {
                let (caret_y_in_block, line_h) = if let Some(para) = entry.paragraph() {
                    let glyph_pos = grapheme_pixel_position(para, focus_pos.offset);
                    (glyph_pos.y, paragraph_line_height_px(para))
                } else {
                    (0.0, render::FONT_SIZE_BODY * render::LINE_HEIGHT_MULTIPLIER)
                };

                let cursor_y = placed.origin.y + entry.y_offset() + caret_y_in_block;
                ensure_cursor_visible(
                    &mut widget_state.scroll_offset,
                    cursor_y,
                    line_h,
                    viewport_height,
                    total_height,
                );
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: iced::advanced::Layout,
        cursor_pos: mouse::Cursor,
        _renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Some(on_action) = self.on_action.as_ref() else {
            return;
        };

        let widget_state = tree.state.downcast_mut::<WidgetState>();
        let bounds = layout.bounds();

        Self::handle_window_events(widget_state, event, shell);
        self.handle_keyboard(widget_state, event, on_action, shell);
        self.handle_clipboard(widget_state, event, on_action, shell);
        self.handle_mouse(widget_state, event, bounds, cursor_pos, on_action, shell);
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: iced::advanced::Layout,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let widget_state = tree.state.downcast_ref::<WidgetState>();
        let bounds = layout.bounds();
        // Inside a scrollable, the editor may be partly or wholly scrolled
        // out: a layer does not inherit the clip of the one around it, so
        // the part still in view is worked out here.
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };
        let text_bounds = bounds.shrink(self.padding);
        let scroll_offset = widget_state.scroll_offset;

        let cursor_visible = widget_state
            .focus
            .as_ref()
            .is_some_and(FocusState::is_cursor_visible);

        // Clip to what shows of the widget and translate by -scroll_offset
        // so content scrolls upward as scroll_offset increases.
        renderer.with_layer(visible, |renderer| {
            renderer.with_translation(Vector::new(0.0, -scroll_offset), |renderer| {
                self.draw_content(renderer, widget_state, &text_bounds, cursor_visible);
            });
        });
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: iced::advanced::Layout,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let is_disabled = self.on_action.is_none();
        let widget_state = tree.state.downcast_ref::<WidgetState>();

        if widget_state.resizing.is_some() {
            return mouse::Interaction::ResizingDiagonallyDown;
        }
        if !is_disabled && let Some(position) = cursor.position_in(layout.bounds()) {
            let content_pos = Point::new(
                position.x - self.padding.left,
                position.y - self.padding.top + widget_state.scroll_offset,
            );
            if widget_state.focus.is_some()
                && let Some(placed) = placed_at(&widget_state.cache, 0.0, self.state.scope())
                && let Some((_, shown)) = selected_image_rect(self.state, placed.cache)
                && image_handle_rect(shown)
                    .expand(IMAGE_HANDLE_GRAB_MARGIN)
                    .contains(placed.local(content_pos))
            {
                return mouse::Interaction::ResizingDiagonallyDown;
            }
            let (_, placed) = scope_at(&widget_state.cache, 0.0, content_pos);
            if image_at_content_point(placed.local(content_pos), placed.cache).is_some() {
                return mouse::Interaction::default();
            }
        }

        if cursor.is_over(layout.bounds()) {
            if is_disabled {
                mouse::Interaction::NotAllowed
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

// ── Block content x-offset helper ────────────────────────

/// Returns the horizontal content offset for a block type. List and blockquote
/// blocks indent their paragraph content.
fn block_content_x_offset(block: &Block) -> f32 {
    match block {
        Block::ListItem { indent_level, .. } => {
            render::LIST_MARKER_WIDTH + (*indent_level as f32) * render::LIST_INDENT_PER_LEVEL
        }
        Block::BlockQuote { .. } => render::BLOCKQUOTE_INDENT,
        _ => 0.0,
    }
}

// ── Paragraph line mapping helpers ──────────────────────

/// Information about a visual line within a paragraph.
struct LineInfo {
    /// The visual line index.
    line: usize,
    /// The character offset at the start of this line (relative to block start).
    start_offset: usize,
}

/// Compute the absolute line height in pixels for a paragraph.
fn paragraph_line_height_px(paragraph: &IcedParagraph) -> f32 {
    paragraph.line_height().to_absolute(paragraph.size()).0
}

/// The document offset of a position the layout engine gives.
///
/// The engine counts bytes within one of its own lines of text; the document
/// counts characters from the start of the block. Taking one for the other
/// shifts the caret by one place per accented letter before it: "é" is two
/// bytes and one character.
fn offset_of_byte(paragraph: &IcedParagraph, line: usize, byte: usize) -> usize {
    let lines = &paragraph.buffer().lines;
    // Each earlier line of text ends with the line break that split it.
    let before: usize = lines
        .iter()
        .take(line)
        .map(|earlier| crate::document::text_len(earlier.text()) + 1)
        .sum();
    let within = lines.get(line).map_or(0, |current| {
        let text = current.text();
        crate::document::text_len(&text[..text.floor_char_boundary(byte)])
    });
    before + within
}

/// The document offset under a point of a paragraph, `None` outside it.
fn offset_at_point(paragraph: &IcedParagraph, point: Point) -> Option<usize> {
    let position = paragraph.hit_test(point)?.cursor();
    Some(offset_of_byte(paragraph, position.line, position.index))
}

/// Build a list of `(line_index, start_char_offset)` pairs for all visual lines
/// in a paragraph, read from the lines the text was actually laid out in.
///
/// Probing the paragraph at multiples of its nominal line height does not
/// work: the laid out lines are not exactly that tall, so a probe meant for
/// the second line can land on the first, and every position past the first
/// line is then drawn on it.
fn build_line_starts(paragraph: &IcedParagraph) -> Vec<(usize, usize)> {
    let mut line_starts: Vec<(usize, usize)> = vec![(0, 0)];
    for (line_idx, run) in paragraph.buffer().layout_runs().enumerate().skip(1) {
        let Some(start) = run.glyphs.iter().map(|glyph| glyph.start).min() else {
            continue;
        };
        let offset = offset_of_byte(paragraph, run.line_i, start);
        // Only add if this line starts at a different offset than the previous.
        if line_starts.last().is_some_and(|&(_, prev)| prev != offset) {
            line_starts.push((line_idx, offset));
        }
    }
    line_starts
}

/// Find which visual line a character offset falls on within a paragraph.
/// Returns the line index and the character offset at the start of that line.
fn find_line_for_offset(paragraph: &IcedParagraph, char_offset: usize) -> LineInfo {
    let line_starts = build_line_starts(paragraph);

    // Find the last line whose start_offset <= char_offset.
    let mut best = LineInfo {
        line: 0,
        start_offset: 0,
    };
    for &(line_idx, start) in &line_starts {
        if start <= char_offset {
            best = LineInfo {
                line: line_idx,
                start_offset: start,
            };
        } else {
            break;
        }
    }

    best
}

/// Compute the pixel position of a character offset within a paragraph.
/// Returns a point relative to the paragraph origin, or `(0, 0)` as fallback.
fn grapheme_pixel_position(paragraph: &IcedParagraph, char_offset: usize) -> Point {
    let line_info = find_line_for_offset(paragraph, char_offset);
    let within_line = char_offset.saturating_sub(line_info.start_offset);

    grapheme_position(paragraph, line_info.line, within_line).unwrap_or(Point::ORIGIN)
}

/// Pixel position of the `index`-th grapheme on visual line `line`.
///
/// Port of `iced_graphics::text::Paragraph::grapheme_position`, removed
/// upstream in 836a93a1 ("Draft `is_secure` support"); rebuilt here on the
/// public `buffer()` / `hint_factor()` accessors. The buffer's glyph
/// coordinates are in hinted (scaled) space, so results divide by the hint
/// factor to get logical pixels.
fn grapheme_position(paragraph: &IcedParagraph, line: usize, index: usize) -> Option<Point> {
    use unicode_segmentation::UnicodeSegmentation;

    let hint_factor = paragraph.hint_factor().unwrap_or(1.0);
    let run = paragraph.buffer().layout_runs().nth(line)?;

    // index represents a grapheme, not a glyph.
    // Find the first glyph for the given grapheme cluster.
    let mut last_start = None;
    let mut last_grapheme_count = 0;
    let mut graphemes_seen = 0;

    let glyph = run
        .glyphs
        .iter()
        .find(|glyph| {
            if Some(glyph.start) != last_start {
                last_grapheme_count = run.text[glyph.start..glyph.end].graphemes(false).count();
                last_start = Some(glyph.start);
                graphemes_seen += last_grapheme_count;
            }

            graphemes_seen >= index
        })
        .or_else(|| run.glyphs.last())?;

    let advance = if index == 0 {
        0.0
    } else {
        glyph.w
            * (1.0
                - graphemes_seen.saturating_sub(index) as f32 / last_grapheme_count.max(1) as f32)
    };

    // The top of the visual line: a glyph's own `y` is its offset within its
    // line, zero for ordinary text, which drew every caret of a wrapped
    // paragraph on its first line.
    Some(Point::new(
        (glyph.x + glyph.x_offset * glyph.font_size + advance) / hint_factor,
        run.line_top / hint_factor,
    ))
}

// ── Selection rectangle computation ─────────────────────

/// Compute per-line selection rectangles for a partial selection within a block.
///
/// `start_offset` and `end_offset` are character offsets within the block.
/// `para_origin_x` / `para_origin_y` are the absolute pixel positions of the
/// paragraph origin. `available_width` is the full text area width.
///
/// Returns a list of `SelectionRect`s, one per visual line that intersects the
/// selection range.
fn compute_selection_rects(
    paragraph: &IcedParagraph,
    start_offset: usize,
    end_offset: usize,
    para_origin_x: f32,
    para_origin_y: f32,
    available_width: f32,
) -> Vec<SelectionRect> {
    let line_height_px = paragraph_line_height_px(paragraph);
    let line_starts = build_line_starts(paragraph);
    let line_count = line_starts.len();
    let mut rects = Vec::new();

    for (i, &(line_idx, line_start)) in line_starts.iter().enumerate() {
        // Determine the character range for this line.
        let line_end = if i + 1 < line_count {
            line_starts[i + 1].1
        } else {
            usize::MAX // last line extends to end of block
        };

        // Skip lines that don't overlap the selection.
        if line_start >= end_offset || line_end <= start_offset {
            continue;
        }

        let line_y = para_origin_y + line_idx as f32 * line_height_px;

        // Determine x-coordinates for this line's selection portion.
        let sel_start_in_line = start_offset.max(line_start);
        let sel_end_in_line = end_offset.min(line_end);

        let x_start = if sel_start_in_line <= line_start {
            // Selection starts at or before this line - use left edge.
            para_origin_x
        } else {
            let within_line = sel_start_in_line.saturating_sub(line_start);
            let pos = grapheme_position(paragraph, line_idx, within_line).unwrap_or(Point::ORIGIN);
            para_origin_x + pos.x
        };

        let x_end = if sel_end_in_line >= line_end {
            // Selection extends to or past the end of this line - use right edge.
            // This covers both middle lines (where line_end is the next line's
            // start) and the last line when end_offset >= the block's char count
            // (or is usize::MAX, as used by BlockSelectionKind::First).
            para_origin_x + available_width
        } else {
            let within_line = sel_end_in_line.saturating_sub(line_start);
            let pos = grapheme_position(paragraph, line_idx, within_line).unwrap_or(Point::ORIGIN);
            para_origin_x + pos.x
        };

        let width = (x_end - x_start).max(0.0);
        if width > 0.0 {
            rects.push(SelectionRect {
                x: x_start,
                y: line_y,
                width,
                height: line_height_px,
            });
        }
    }

    rects
}

// ── Auto-scroll helper ───────────────────────────────────

/// Scrolling line height used for mouse wheel (px per line).
const SCROLL_LINE_HEIGHT: f32 = 20.0;

/// Drag auto-scroll: pixels scrolled per pixel of cursor overshoot beyond the
/// viewport edge.
const DRAG_SCROLL_SPEED: f32 = 2.0;

/// Drag auto-scroll: maximum scroll distance per frame (pixels).
const DRAG_SCROLL_MAX: f32 = 30.0;

/// Ensure the cursor (at `cursor_y` with `cursor_height`) is visible within
/// the viewport defined by `scroll_offset` and `viewport_height`.
///
/// Adjusts `scroll_offset` so the cursor is fully visible, scrolling up or
/// down as needed. Also clamps to `[0, max_scroll]`.
fn ensure_cursor_visible(
    scroll_offset: &mut f32,
    cursor_y: f32,
    cursor_height: f32,
    viewport_height: f32,
    total_content_height: f32,
) {
    let max_scroll = (total_content_height - viewport_height).max(0.0);

    // Cursor is above the viewport - scroll up.
    if cursor_y < *scroll_offset {
        *scroll_offset = cursor_y;
    }
    // Cursor bottom is below the viewport - scroll down.
    if cursor_y + cursor_height > *scroll_offset + viewport_height {
        *scroll_offset = cursor_y + cursor_height - viewport_height;
    }

    *scroll_offset = scroll_offset.clamp(0.0, max_scroll);
}

// ── Hit testing helper ───────────────────────────────────

/// Convert a pixel position (relative to the content origin, after padding) to a
/// `DocPosition` by hit-testing the paragraph cache.
///
/// Finds which block the point falls in via `ParagraphCache::block_at_y`, then
/// calls `Paragraph::hit_test` on that block's cached paragraph to get the
/// character offset within the block.
fn hit_test_content_point(
    content_pos: Point,
    cache: &ParagraphCache<IcedParagraph>,
    document: &crate::document::Document,
) -> crate::document::DocPosition {
    // Find the block at the click y-coordinate.
    let Some(block_index) = cache.block_at_y(content_pos.y) else {
        return crate::document::DocPosition::zero();
    };

    let Some(entry) = cache.get(block_index) else {
        return crate::document::DocPosition::new(block_index, 0);
    };

    // A drawn image and a table have two positions: before and after.
    let atom_width = entry
        .image_size()
        .map(|size| size.width)
        .or_else(|| entry.table().map(|table| table.width));
    if let Some(width) = atom_width {
        let offset = usize::from(content_pos.x >= width / 2.0);
        return crate::document::DocPosition::new(block_index, offset);
    }

    let content_x_offset = document
        .block(block_index)
        .map(block_content_x_offset)
        .unwrap_or(0.0)
        + entry.x_shift();

    // For container blocks (List, BlockQuote) with child paragraphs,
    // find which child the click falls in and hit-test that child.
    let children = entry.child_paragraphs();
    if !children.is_empty() {
        let local_y = content_pos.y - entry.y_offset();

        // Find the child whose y-range contains the click. Fall back to
        // the last child if the click is below all children.
        let child = children
            .iter()
            .rev()
            .find(|c| local_y >= c.local_y_offset)
            .unwrap_or(&children[0]);

        let local_point = Point::new(
            content_pos.x - content_x_offset,
            local_y - child.local_y_offset,
        );

        let char_offset = offset_at_point(&child.paragraph, local_point).unwrap_or(0);

        return crate::document::DocPosition::new(block_index, char_offset);
    }

    // For inline blocks, hit-test the single paragraph.
    let Some(paragraph) = entry.paragraph() else {
        // No paragraph (e.g. HorizontalRule) - place cursor at start of block.
        return crate::document::DocPosition::new(block_index, 0);
    };

    // Translate into paragraph-local coordinates.
    let local_point = Point::new(
        content_pos.x - content_x_offset,
        content_pos.y - entry.y_offset(),
    );

    let char_offset = offset_at_point(paragraph, local_point).unwrap_or(0);

    crate::document::DocPosition::new(block_index, char_offset)
}

/// The number of a list item among the items before it: how many of them,
/// back to the start of its list, share its kind and its indent.
fn list_item_number(
    blocks: &[std::sync::Arc<Block>],
    block_index: usize,
    ordered: bool,
    indent_level: u8,
) -> usize {
    let mut number = 0;
    for previous in blocks[..block_index].iter().rev() {
        let Block::ListItem {
            ordered: previous_ordered,
            indent_level: previous_indent,
            ..
        } = previous.as_ref()
        else {
            break;
        };
        if *previous_ordered == ordered && *previous_indent == indent_level {
            number += 1;
        } else if *previous_indent <= indent_level {
            break;
        }
    }
    number
}

/// The image block the selection covers exactly, with the rectangle it is
/// drawn in, in content coordinates.
fn selected_image_rect(
    state: &EditorState,
    cache: &ParagraphCache<IcedParagraph>,
) -> Option<(usize, Rectangle)> {
    let block_index = state.selected_image()?;
    let entry = cache.get(block_index)?;
    let size = entry.image_size()?;
    Some((
        block_index,
        Rectangle::new(Point::new(0.0, entry.y_offset()), size),
    ))
}

/// The image block drawn under a point in content coordinates.
fn image_at_content_point(
    content_pos: Point,
    cache: &ParagraphCache<IcedParagraph>,
) -> Option<usize> {
    let block_index = cache.block_at_y(content_pos.y)?;
    let entry = cache.get(block_index)?;
    let size = entry.image_size()?;
    Rectangle::new(Point::new(0.0, entry.y_offset()), size)
        .contains(content_pos)
        .then_some(block_index)
}

/// The square handle centered on the bottom right corner of an image.
fn image_handle_rect(image: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(
            image.x + image.width - IMAGE_HANDLE_SIZE / 2.0,
            image.y + image.height - IMAGE_HANDLE_SIZE / 2.0,
        ),
        Size::new(IMAGE_HANDLE_SIZE, IMAGE_HANDLE_SIZE),
    )
}

/// A shown size in whole pixels.
fn shown_pixels(size: Size) -> (u32, u32) {
    (
        crate::image::whole_pixels(size.width),
        crate::image::whole_pixels(size.height),
    )
}

/// Whether a selection reaching a one-position block (an image) holds it.
fn selection_holds_atom(kind: BlockSelectionKind) -> bool {
    match kind {
        BlockSelectionKind::Single {
            start_offset,
            end_offset,
        } => start_offset == 0 && end_offset >= 1,
        BlockSelectionKind::First { start_offset } => start_offset == 0,
        BlockSelectionKind::Last { end_offset } => end_offset >= 1,
        BlockSelectionKind::Full => true,
    }
}

fn link_at_content_point(
    content_pos: Point,
    cache: &ParagraphCache<IcedParagraph>,
    document: &crate::document::Document,
) -> Option<String> {
    let block_index = cache.block_at_y(content_pos.y)?;
    let entry = cache.get(block_index)?;
    let paragraph = entry.paragraph()?;
    let block = document.block(block_index)?;
    let runs = block.runs()?;
    let content_x_offset = block_content_x_offset(block) + entry.x_shift();
    let local_point = Point::new(
        content_pos.x - content_x_offset,
        content_pos.y - entry.y_offset(),
    );
    let span_index = paragraph.hit_span(local_point)?;
    runs.get(span_index).and_then(|run| run.link.clone())
}

// ── Into<Element> ───────────────────────────────────────

impl<'a, Message: 'a> From<RichTextEditor<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(editor: RichTextEditor<'a, Message>) -> Self {
        Self::new(editor)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_content_sized, scrolled_offset};
    use iced::Length;

    #[test]
    fn a_shrinking_height_with_a_minimum_still_follows_the_document() {
        assert!(is_content_sized(Length::Shrink));
        assert!(is_content_sized(Length::Shrink.min(160.0)));
        assert!(is_content_sized(Length::Fit.max(400.0)));
        assert!(!is_content_sized(Length::Fill));
        assert!(!is_content_sized(Length::Fill.min(160.0)));
        assert!(!is_content_sized(Length::Fixed(200.0)));
    }

    #[test]
    fn an_editor_that_fits_its_text_does_not_move_under_the_wheel() {
        assert_eq!(scrolled_offset(0.0, 40.0, 120.0, 300.0), 0.0);
        assert_eq!(scrolled_offset(0.0, 40.0, 500.0, 300.0), 40.0);
        assert_eq!(scrolled_offset(190.0, 40.0, 500.0, 300.0), 200.0);
        assert_eq!(scrolled_offset(10.0, -40.0, 500.0, 300.0), 0.0);
    }
}
