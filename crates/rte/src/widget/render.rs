//! Paragraph caching, draw logic, block-level rendering.
//!
//! Provides:
//! - [`build_spans_for_block`] - converts a [`Block`]'s [`StyledRun`]s to iced [`Span`]s
//! - [`ParagraphCache`] - stores pre-built paragraphs per block with dirty flags
//! - Drawing helpers for horizontal rules, blockquote borders, and list markers

use crate::document::{
    Block, BlockAttrs, HeadingLevel, InlineImage, InlineStyle, Rgb, StyledRun, Table, TextAlignment,
};
use crate::table_box::{
    CellBox, VerticalAlign, cell_box, column_widths, table_background, table_spacing, table_width,
};
use std::sync::Arc;

use iced::advanced::renderer;
use iced::advanced::text::{Paragraph, Span, Text};
use iced::{Color, Font, Point, Rectangle, Size};

// ── Font size constants ─────────────────────────────────
//
// These mirror the app's layout.rs type scale. They live here so the
// editor crate is self-contained (no dependency on the app crate).

/// Body text / paragraph font size (px).
pub const FONT_SIZE_BODY: f32 = 13.0;
/// Heading H3 font size (px).
pub const FONT_SIZE_H3: f32 = 14.0;
/// Heading H2 font size (px).
pub const FONT_SIZE_H2: f32 = 16.0;
/// Heading H1 font size (px).
pub const FONT_SIZE_H1: f32 = 18.0;

/// Default line-height multiplier (relative).
pub const LINE_HEIGHT_MULTIPLIER: f32 = 1.4;

// ── Block spacing constants ─────────────────────────────

/// Vertical spacing after a paragraph block (px).
/// None: a new paragraph starts on the next line, as in every mail client,
/// and as it is sent. An empty line is an empty paragraph.
pub const SPACING_PARAGRAPH: f32 = 0.0;
/// Space under an image standing on a line of its own.
pub const SPACING_IMAGE: f32 = 8.0;
/// Vertical spacing after a heading block (px).
pub const SPACING_HEADING: f32 = 12.0;
/// Vertical spacing after a list block (px).
pub const SPACING_LIST: f32 = 8.0;
/// Vertical spacing after a blockquote block (px).
pub const SPACING_BLOCKQUOTE: f32 = 12.0;
/// Vertical spacing after a horizontal rule (px).
pub const SPACING_HR: f32 = 12.0;
/// Vertical spacing between list items (px).
pub const SPACING_LIST_ITEM: f32 = 4.0;

// ── Blockquote constants ────────────────────────────────

/// Width of the blockquote left border (px).
pub const BLOCKQUOTE_BORDER_WIDTH: f32 = 2.0;
/// Horizontal indent for blockquote content (px).
pub const BLOCKQUOTE_INDENT: f32 = 16.0;

// ── List constants ──────────────────────────────────────

/// Width of the leading container for bullet/number (px).
pub const LIST_MARKER_WIDTH: f32 = 24.0;
/// Additional indent per nesting level (px).
pub const LIST_INDENT_PER_LEVEL: f32 = 24.0;
/// Left margin per indent level of a paragraph or heading (px).
pub const PARAGRAPH_INDENT_PER_LEVEL: f32 = 24.0;

/// The bullet character for unordered lists.
const BULLET_CHAR: &str = "\u{2022}"; // •

// ── Horizontal rule constants ───────────────────────────

/// Height of the horizontal rule line (px).
pub const HR_LINE_HEIGHT: f32 = 1.0;
/// Total height of the horizontal rule block including padding (px).
pub const HR_BLOCK_HEIGHT: f32 = 16.0;

// ── Image placeholder constants ──────────────────────────

/// Default placeholder height for image blocks (px).
pub const IMAGE_PLACEHOLDER_HEIGHT: f32 = 150.0;
/// Padding inside the image placeholder rectangle (px).
pub const IMAGE_PLACEHOLDER_PADDING: f32 = 8.0;

// ── Font size resolution ────────────────────────────────

/// Returns the font size in pixels for a given block.
pub fn block_font_size(block: &Block) -> f32 {
    match block {
        Block::Heading { level, .. } => heading_font_size(*level),
        Block::Paragraph { .. } => FONT_SIZE_BODY,
        Block::ListItem { .. } => FONT_SIZE_BODY,
        Block::BlockQuote { .. } => FONT_SIZE_BODY,
        Block::HorizontalRule => FONT_SIZE_BODY,
        Block::Image { .. } => FONT_SIZE_BODY,
        Block::Table(_) => FONT_SIZE_BODY,
    }
}

/// Returns the font size for a heading level.
pub fn heading_font_size(level: HeadingLevel) -> f32 {
    match level {
        HeadingLevel::H1 => FONT_SIZE_H1,
        HeadingLevel::H2 => FONT_SIZE_H2,
        HeadingLevel::H3 => FONT_SIZE_H3,
    }
}

/// Returns the vertical spacing after a block (px).
pub fn block_spacing(block: &Block) -> f32 {
    match block {
        Block::Paragraph { .. } => SPACING_PARAGRAPH,
        Block::Heading { .. } => SPACING_HEADING,
        Block::ListItem { .. } => SPACING_LIST,
        Block::BlockQuote { .. } => SPACING_BLOCKQUOTE,
        Block::HorizontalRule => SPACING_HR,
        Block::Image { .. } => SPACING_IMAGE,
        // A table brings its own space: the padding of its cells.
        Block::Table(_) => 0.0,
    }
}

// ── Span building ───────────────────────────────────────

/// Resolve the iced [`Font`] for an inline style.
///
/// `base_font` is the font family to use (typically the editor's configured
/// body font). Bold/italic/bold-italic variants are derived from it.
pub fn font_for_style(base_font: Font, style: InlineStyle) -> Font {
    let bold = style.contains(InlineStyle::BOLD);
    let italic = style.contains(InlineStyle::ITALIC);
    match (bold, italic) {
        (true, true) => Font {
            weight: iced::font::Weight::Bold,
            style: iced::font::Style::Italic,
            ..base_font
        },
        (true, false) => Font {
            weight: iced::font::Weight::Bold,
            ..base_font
        },
        (false, true) => Font {
            style: iced::font::Style::Italic,
            ..base_font
        },
        (false, false) => base_font,
    }
}

/// The font, size and color a run is drawn with: its own where its format
/// sets them, the block's and the editor's otherwise.
fn run_appearance(
    run: &StyledRun,
    base_font: Font,
    block_font_size: f32,
    text_color: Color,
    link_color: Color,
) -> (Font, f32, Color) {
    let family = run
        .format
        .font
        .map_or(base_font, |family| Font::new(family.name()));
    let font = font_for_style(family, run.style);
    let size = run.format.size.map_or(block_font_size, f32::from);
    let color = match run.format.color {
        Some(color) => readable_color(color, text_color),
        None if run.link.is_some() => link_color,
        None => text_color,
    };
    (font, size, color)
}

/// A run's own color, unless it would vanish against the editor: quoted
/// mail often carries black text, which a dark theme cannot show, and the
/// reverse on a light one. Such a color is drawn in the editor's text color;
/// the document keeps the color it was given.
fn readable_color(color: Rgb, text_color: Color) -> Color {
    let own = Color::from_rgb8(color.r, color.g, color.b);
    let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    let text_is_light = luminance(text_color) > 0.5;
    let vanishes = if text_is_light {
        luminance(own) < 0.35
    } else {
        luminance(own) > 0.85
    };
    if vanishes { text_color } else { own }
}

/// Convert a single [`StyledRun`] to an iced [`Span`].
///
/// The `Link` type parameter is `String` - the href of the link, if any.
/// `base_font` is the editor's configured body font. `font_size` is the
/// size for this block (e.g. heading size or body size).
/// `link_color` is the color to use for linked spans.
pub fn run_to_span<'a>(
    run: &'a StyledRun,
    base_font: Font,
    font_size: f32,
    text_color: Color,
    link_color: Color,
) -> Span<'a, String> {
    let (font, font_size, color) =
        run_appearance(run, base_font, font_size, text_color, link_color);
    // A link is told apart by its colour alone, as it is sent.
    let underline = run.style.contains(InlineStyle::UNDERLINE);
    let strikethrough = run.style.contains(InlineStyle::STRIKETHROUGH);

    let mut span = Span::new(run.text.as_str())
        .font(font)
        .size(font_size)
        .color(color)
        .underline(underline)
        .strikethrough(strikethrough);

    if let Some(href) = &run.link {
        span = span.link(href.clone());
    }

    match run.format.image {
        Some(image) => image_span(span, image, base_font),
        None => span,
    }
}

/// Build iced [`Span`]s for all runs in a block.
///
/// Returns an empty vec for blocks that have no inline content
/// (e.g. `HorizontalRule`, `List`, `BlockQuote`).
pub fn build_spans_for_block<'a>(
    block: &'a Block,
    base_font: Font,
    text_color: Color,
    link_color: Color,
) -> Vec<Span<'a, String>> {
    let Some(runs) = block.runs() else {
        return Vec::new();
    };

    let font_size = block_font_size(block);

    runs.iter()
        .map(|run| run_to_span(run, base_font, font_size, text_color, link_color))
        .collect()
}

/// Build spans for a block, handling container blocks by recursively
/// collecting styled spans from their children. Unlike [`build_spans_for_block`],
/// this never returns empty for valid blocks - container blocks (List,
/// BlockQuote) produce spans that preserve inline formatting and links.
pub fn build_spans_for_any_block(
    block: &Block,
    base_font: Font,
    text_color: Color,
    link_color: Color,
) -> Vec<Span<'static, String>> {
    // Try the normal path first (works for Paragraph, Heading).
    if let Some(runs) = block.runs() {
        let font_size = block_font_size(block);
        return runs
            .iter()
            .map(|run| owned_run_to_span(run, base_font, font_size, text_color, link_color))
            .collect();
    }

    // For container blocks, recursively collect spans from children.
    let mut spans = Vec::new();
    collect_container_spans(block, base_font, text_color, link_color, &mut spans, false);
    spans
}

/// Recursively collect styled spans from a container block's children,
/// preserving inline formatting and links.
fn collect_container_spans(
    block: &Block,
    base_font: Font,
    text_color: Color,
    link_color: Color,
    spans: &mut Vec<Span<'static, String>>,
    needs_separator: bool,
) {
    if needs_separator && !spans.is_empty() {
        spans.push(
            Span::new("\n".to_owned())
                .font(base_font)
                .size(FONT_SIZE_BODY)
                .color(text_color),
        );
    }

    match block {
        Block::Paragraph { runs, .. }
        | Block::Heading { runs, .. }
        | Block::ListItem { runs, .. } => {
            let font_size = block_font_size(block);
            for run in runs {
                spans.push(owned_run_to_span(
                    run, base_font, font_size, text_color, link_color,
                ));
            }
        }
        Block::BlockQuote { blocks } => {
            for (i, child) in blocks.iter().enumerate() {
                collect_container_spans(child, base_font, text_color, link_color, spans, i > 0);
            }
        }
        Block::HorizontalRule | Block::Image { .. } | Block::Table(_) => {}
    }
}

/// Like [`run_to_span`] but returns an owned Span (for container block flattening
/// where we can't borrow from the block).
fn owned_run_to_span(
    run: &StyledRun,
    base_font: Font,
    font_size: f32,
    text_color: Color,
    link_color: Color,
) -> Span<'static, String> {
    let (font, font_size, color) =
        run_appearance(run, base_font, font_size, text_color, link_color);
    let underline = run.style.contains(InlineStyle::UNDERLINE);
    let strikethrough = run.style.contains(InlineStyle::STRIKETHROUGH);

    let mut span = Span::new(run.text.clone())
        .font(font)
        .size(font_size)
        .color(color)
        .underline(underline)
        .strikethrough(strikethrough);

    if let Some(href) = &run.link {
        span = span.link(href.clone());
    }

    match run.format.image {
        Some(image) => image_span(span, image, base_font),
        None => span,
    }
}

// ── Images in a line of text ────────────────────────────

/// The characters an image can be drawn as, in order of preference. The
/// document holds [`crate::document::INLINE_IMAGE_CHAR`], which the text shaper gives no
/// width; what is laid out is the first of these that comes out wide.
/// Each is one grapheme of three bytes, like the character it stands for,
/// so positions in the line map one to one.
const IMAGE_DISPLAY_CHARS: [&str; 3] = ["\u{2588}", "\u{25A0}", "\u{2014}"];

/// For each base font: the character an image is drawn as, and how wide it
/// is per unit of font size.
static IMAGE_CHAR_ADVANCE: std::sync::Mutex<Vec<(Font, &'static str, f32)>> =
    std::sync::Mutex::new(Vec::new());

/// The font size the image character is measured at (px).
const IMAGE_CHAR_MEASURE_SIZE: f32 = 100.0;

fn image_char_advance(base_font: Font) -> Option<(&'static str, f32)> {
    IMAGE_CHAR_ADVANCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(font, _, _)| *font == base_font)
        .map(|(_, glyph, advance)| (*glyph, *advance))
}

/// Measure, once per base font, how wide the character an image is drawn
/// as comes out, so that it can be laid out exactly as wide as the image.
fn measure_image_char<P: Paragraph>(base_font: Font) {
    if image_char_advance(base_font).is_some() {
        return;
    }
    let measured = IMAGE_DISPLAY_CHARS.iter().find_map(|glyph| {
        let spans = [Span::<String>::new(*glyph)
            .font(base_font)
            .size(IMAGE_CHAR_MEASURE_SIZE)];
        let paragraph = build_paragraph::<P>(
            &spans,
            IMAGE_CHAR_MEASURE_SIZE * 10.0,
            base_font,
            IMAGE_CHAR_MEASURE_SIZE,
        );
        let advance = paragraph.min_bounds().width / IMAGE_CHAR_MEASURE_SIZE;
        (advance > 0.05).then_some((*glyph, advance))
    });
    // No font gives any of them a width: the first is kept, at one unit per
    // pixel, and the image is drawn where the text shaper puts it.
    let (glyph, advance) = measured.unwrap_or((IMAGE_DISPLAY_CHARS[0], 1.0));
    IMAGE_CHAR_ADVANCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((base_font, glyph, advance));
}

/// Lay the character standing for an image out as a box of the image's
/// size: invisible, as wide as the image, on a line as tall.
fn image_span<'a>(
    mut span: Span<'a, String>,
    image: InlineImage,
    base_font: Font,
) -> Span<'a, String> {
    let (width, height) = image.shown_size();
    let (glyph, advance) = image_char_advance(base_font).unwrap_or((IMAGE_DISPLAY_CHARS[0], 1.0));
    span.text = std::borrow::Cow::Borrowed(glyph);
    span.font(base_font)
        .size(width / advance)
        .line_height(iced::advanced::text::LineHeight::Absolute(iced::Pixels(
            height,
        )))
        .color(Color::TRANSPARENT)
        .underline(false)
        .strikethrough(false)
}

// ── Paragraph cache ─────────────────────────────────────

/// A laid-out paragraph for a child element within a container block
/// (list item or blockquote child).
pub struct ChildParagraph<P: Paragraph> {
    /// The laid-out paragraph for this child.
    pub paragraph: P,
    /// Y offset relative to the container block's top edge (px).
    pub local_y_offset: f32,
    /// Height of this child paragraph (px).
    pub height: f32,
}

/// A cached paragraph for a single document block.
pub struct CacheEntry<P: Paragraph> {
    /// The laid-out paragraph. `None` if the block has no inline content
    /// (e.g. `HorizontalRule`) or if it is a container block that uses
    /// `child_paragraphs` instead.
    paragraph: Option<P>,
    /// Per-child paragraphs for container blocks (List, BlockQuote).
    /// Empty for non-container blocks.
    child_paragraphs: Vec<ChildParagraph<P>>,
    /// Whether this entry needs re-layout on the next frame.
    dirty: bool,
    /// Y offset from the top of the editor widget (px).
    y_offset: f32,
    /// How far right of the block's content edge the paragraph sits (px):
    /// its indent, plus what centers or right-aligns it.
    x_shift: f32,
    /// Height of this block including its content (px).
    height: f32,
    /// The size an image block is drawn at, `None` for any other block and
    /// for an image shown as a placeholder.
    image_size: Option<Size>,
    /// How wide the block is when nothing makes it wrap (px).
    natural_width: f32,
    /// The layout of a table block, `None` for any other block.
    table: Option<Box<TableLayout<P>>>,
}

impl<P: Paragraph> Default for CacheEntry<P> {
    fn default() -> Self {
        Self {
            paragraph: None,
            child_paragraphs: Vec::new(),
            dirty: true,
            y_offset: 0.0,
            x_shift: 0.0,
            height: 0.0,
            image_size: None,
            natural_width: 0.0,
            table: None,
        }
    }
}

/// What every block of a layout pass is laid out with.
#[derive(Clone, Copy)]
pub struct LayoutStyle<'a> {
    /// The editor's configured body font.
    pub base_font: Font,
    pub text_color: Color,
    pub link_color: Color,
    /// The pixel size of an image source that can be drawn, `None` for one
    /// shown as a placeholder.
    pub natural_size: &'a dyn Fn(&str) -> Option<(u32, u32)>,
}

// ── Table layout ────────────────────────────────────────

/// A laid-out table.
pub struct TableLayout<P: Paragraph> {
    /// The table this layout was made for and the width it was offered: a
    /// later pass with the same two keeps it as it is.
    table: Arc<Table>,
    offered_width: f32,
    /// Width of the whole table (px).
    pub width: f32,
    /// Height of the whole table (px).
    pub height: f32,
    pub background: Option<Rgb>,
    /// The cells, row by row.
    pub rows: Vec<Vec<CellLayout<P>>>,
}

/// A laid-out table cell.
pub struct CellLayout<P: Paragraph> {
    /// Paddings, borders and background of the cell.
    pub cell_box: CellBox,
    /// The cell, relative to the top left corner of its table.
    pub bounds: Rectangle,
    /// Where its content starts, relative to the same corner.
    pub content_origin: Point,
    /// Width its content is laid out in (px).
    pub content_width: f32,
    /// The layout of the blocks of the cell.
    pub cache: ParagraphCache<P>,
    /// The same blocks laid out unwrapped, to know how wide they are.
    measure: ParagraphCache<P>,
}

impl<P: Paragraph> CellLayout<P> {
    fn new(cell_box: CellBox) -> Self {
        Self {
            cell_box,
            bounds: Rectangle::default(),
            content_origin: Point::ORIGIN,
            content_width: 0.0,
            cache: ParagraphCache::new(),
            measure: ParagraphCache::new(),
        }
    }
}

impl<P: Paragraph> TableLayout<P> {
    /// The cell under `point`, given relative to the top left corner of
    /// the table, as `(row, column)`.
    pub fn cell_at(&self, point: Point) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(row, cells)| {
            cells
                .iter()
                .position(|cell| cell.bounds.contains(point))
                .map(|column| (row, column))
        })
    }

    /// The cell at `row`, `column`.
    pub fn cell(&self, row: usize, column: usize) -> Option<&CellLayout<P>> {
        self.rows.get(row)?.get(column)
    }
}

/// Lay a table out in `available_width`. `previous` is its layout from the
/// last pass, whose cells are kept so that what did not change inside them
/// is not laid out again.
fn layout_table<P: Paragraph>(
    previous: Option<Box<TableLayout<P>>>,
    table: &Arc<Table>,
    available_width: f32,
    style: &LayoutStyle<'_>,
) -> Box<TableLayout<P>> {
    let mut previous_rows = match previous {
        Some(previous)
            if Arc::ptr_eq(&previous.table, table)
                && previous.offered_width.to_bits() == available_width.to_bits() =>
        {
            return previous;
        }
        Some(previous) => previous.rows,
        None => Vec::new(),
    };

    // One cell layout per cell, taken from the last pass where there is one.
    let mut rows: Vec<Vec<CellLayout<P>>> = table
        .rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let mut kept = previous_rows
                .get_mut(row_index)
                .map(std::mem::take)
                .unwrap_or_default()
                .into_iter();
            row.cells
                .iter()
                .map(|cell| {
                    let cell_box = cell_box(&table.attrs, &row.attrs, &cell.attrs);
                    match kept.next() {
                        Some(mut layout) => {
                            layout.cell_box = cell_box;
                            layout
                        }
                        None => CellLayout::new(cell_box),
                    }
                })
                .collect()
        })
        .collect();

    let spacing = table_spacing(&table.attrs);
    let columns = rows
        .iter()
        .map(|cells| cells.iter().map(|cell| cell.cell_box.span).sum::<usize>())
        .max()
        .unwrap_or(0);
    let gaps = spacing * (columns + 1) as f32;
    let asked = table_width(&table.attrs).map(|width| width.pixels(available_width));
    let room = (asked.map_or(available_width, |asked| asked.min(available_width)) - gaps).max(0.0);

    // The width each column asks for, from the cells that take one column.
    // A cell's width is that of its content, as in HTML: its paddings and
    // borders come on top.
    let mut fixed: Vec<Option<f32>> = vec![None; columns];
    for cells in &rows {
        let mut column = 0;
        for cell in cells {
            if cell.cell_box.span == 1
                && let Some(width) = cell.cell_box.width
            {
                let width = width.pixels(room) + cell.cell_box.horizontal_extra();
                fixed[column] = Some(fixed[column].map_or(width, |known| known.max(width)));
            }
            column += cell.cell_box.span;
        }
    }

    // The width the content of the other columns takes unwrapped.
    let mut natural = vec![0.0_f32; columns];
    for (cells, row) in rows.iter_mut().zip(&table.rows) {
        let mut column = 0;
        for (layout, cell) in cells.iter_mut().zip(&row.cells) {
            let span = layout.cell_box.span;
            if span == 1 && fixed[column].is_none() {
                let extra = layout.cell_box.horizontal_extra();
                layout
                    .measure
                    .layout_blocks(&cell.blocks, (room - extra).max(0.0), style, true);
                let content = if cell.is_blank() {
                    0.0
                } else {
                    layout.measure.natural_width()
                };
                natural[column] = natural[column].max(content + extra);
            }
            column += span;
        }
    }

    let widths = column_widths(&fixed, &natural, asked.map(|asked| asked - gaps), room);

    // Lay every cell out at its width; a row is as tall as its tallest cell.
    let mut y = spacing;
    for (cells, row) in rows.iter_mut().zip(&table.rows) {
        let mut column = 0;
        let mut x = spacing;
        let mut row_height = 0.0_f32;
        let mut content_heights = Vec::with_capacity(cells.len());
        for (layout, cell) in cells.iter_mut().zip(&row.cells) {
            let span = layout.cell_box.span.min(columns - column).max(1);
            let cell_width: f32 =
                widths[column..column + span].iter().sum::<f32>() + spacing * (span - 1) as f32;
            layout.content_width = (cell_width - layout.cell_box.horizontal_extra()).max(0.0);
            let laid_out =
                layout
                    .cache
                    .layout_blocks(&cell.blocks, layout.content_width, style, true);
            // A cell left empty takes no height of its own: it is as tall
            // as its paddings and the height it asks for.
            let content_height = if cell.is_blank() { 0.0 } else { laid_out };
            content_heights.push(content_height);
            row_height = row_height
                .max(content_height + layout.cell_box.vertical_extra())
                .max(layout.cell_box.height.unwrap_or(0.0));
            layout.bounds = Rectangle::new(Point::new(x, y), Size::new(cell_width, 0.0));
            x += cell_width + spacing;
            column += span;
        }
        for (layout, content_height) in cells.iter_mut().zip(content_heights) {
            layout.bounds.height = row_height;
            let free = (row_height - layout.cell_box.vertical_extra() - content_height).max(0.0);
            let lowered = match layout.cell_box.vertical {
                VerticalAlign::Top => 0.0,
                VerticalAlign::Middle => free / 2.0,
                VerticalAlign::Bottom => free,
            };
            layout.content_origin = Point::new(
                layout.bounds.x + layout.cell_box.content_left(),
                layout.bounds.y + layout.cell_box.content_top() + lowered,
            );
        }
        y += row_height + spacing;
    }

    Box::new(TableLayout {
        table: Arc::clone(table),
        offered_width: available_width,
        width: widths.iter().sum::<f32>() + gaps,
        height: y,
        background: table_background(&table.attrs),
        rows,
    })
}

impl<P: Paragraph> CacheEntry<P> {
    /// The pre-laid-out paragraph, if this block has inline content.
    pub fn paragraph(&self) -> Option<&P> {
        self.paragraph.as_ref()
    }

    /// Per-child paragraphs for container blocks (List, BlockQuote).
    /// Empty for non-container blocks.
    pub fn child_paragraphs(&self) -> &[ChildParagraph<P>] {
        &self.child_paragraphs
    }

    /// Whether this entry is dirty (needs re-layout).
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// How far right of the block's content edge the paragraph sits (px).
    pub fn x_shift(&self) -> f32 {
        self.x_shift
    }

    /// Y offset from the top of the editor widget.
    pub fn y_offset(&self) -> f32 {
        self.y_offset
    }

    /// Height of this block's rendered content.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// The size an image block is drawn at, `None` for any other block and
    /// for an image shown as a placeholder.
    pub fn image_size(&self) -> Option<Size> {
        self.image_size
    }

    /// The layout of a table block, `None` for any other block.
    pub fn table(&self) -> Option<&TableLayout<P>> {
        self.table.as_deref()
    }
}

/// Paragraph cache: one [`CacheEntry`] per document block.
///
/// The cache is rebuilt (or partially updated) during the widget's `layout()`
/// pass. Only dirty entries are re-laid-out; clean entries keep their existing
/// paragraph and just have their y-offsets recomputed.
pub struct ParagraphCache<P: Paragraph> {
    entries: Vec<CacheEntry<P>>,
    /// Cached total height from the last `layout()` call.
    last_layout_height: f32,
    /// Width of the widest block when nothing makes it wrap, from the last
    /// layout pass (px).
    natural_width: f32,
    /// Font and colors of the last `layout()` call. A table keeps its
    /// layout from one pass to the next, so a change of these starts over.
    look: Option<(Font, Color, Color)>,
}

impl<P: Paragraph> Default for ParagraphCache<P> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            last_layout_height: 0.0,
            natural_width: 0.0,
            look: None,
        }
    }
}

impl<P: Paragraph> ParagraphCache<P> {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access the cache entry for block `index`.
    pub fn get(&self, index: usize) -> Option<&CacheEntry<P>> {
        self.entries.get(index)
    }

    /// Mark a single block as dirty (needs re-layout).
    pub fn mark_dirty(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.dirty = true;
        }
    }

    /// Mark all entries as dirty.
    pub fn mark_all_dirty(&mut self) {
        for entry in &mut self.entries {
            entry.dirty = true;
        }
    }

    /// Resize the cache to match the document's block count.
    ///
    /// New entries are created as dirty. Excess entries are removed.
    /// Existing entries at unchanged indices are preserved.
    pub fn resize(&mut self, block_count: usize) {
        match self.entries.len().cmp(&block_count) {
            std::cmp::Ordering::Less => {
                self.entries.resize_with(block_count, CacheEntry::default);
            }
            std::cmp::Ordering::Greater => {
                self.entries.truncate(block_count);
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    /// Mark a block as dirty and resize if the block was inserted.
    ///
    /// Call after inserting a new block at `index`.
    pub fn insert_dirty(&mut self, index: usize) {
        if index <= self.entries.len() {
            self.entries.insert(index, CacheEntry::default());
        }
    }

    /// Remove the cache entry at `index`.
    ///
    /// Call after removing a block from the document.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    /// Rebuild dirty paragraphs and recompute all y-offsets.
    ///
    /// `available_width` is the width available for text layout.
    /// `blocks` is the slice of blocks from the document (must match the
    /// cache length - call [`resize`] first).
    /// `base_font` is the editor's configured body font.
    /// `text_color` and `link_color` are the colors for normal and linked text.
    /// `natural_size` gives the pixel size of an image source that can be
    /// drawn, `None` for one shown as a placeholder.
    ///
    /// Returns the total height of all blocks.
    pub fn layout(
        &mut self,
        blocks: &[impl AsRef<Block>],
        available_width: f32,
        base_font: Font,
        text_color: Color,
        link_color: Color,
        natural_size: &dyn Fn(&str) -> Option<(u32, u32)>,
    ) -> f32 {
        measure_image_char::<P>(base_font);
        let look = Some((base_font, text_color, link_color));
        if self.look != look {
            self.entries.clear();
            self.look = look;
        }
        let style = LayoutStyle {
            base_font,
            text_color,
            link_color,
            natural_size,
        };
        self.layout_blocks(blocks, available_width, &style, false)
    }

    /// Lay `blocks` out in `available_width`. `tight` is set inside a table
    /// cell, where blocks follow each other without the space the document
    /// puts between them.
    ///
    /// Returns the total height of all blocks.
    pub fn layout_blocks(
        &mut self,
        blocks: &[impl AsRef<Block>],
        available_width: f32,
        style: &LayoutStyle<'_>,
        tight: bool,
    ) -> f32 {
        self.resize(blocks.len());

        // Pessimistic: mark every entry dirty so edits that don't change
        // block count (typing, backspace within a block, formatting toggle)
        // still get re-laid-out. This is fine for typical email-length
        // documents (tens of blocks). For very long documents (hundreds+),
        // replace with a generation counter or per-block dirty tracking
        // to avoid re-laying-out unchanged paragraphs every frame.
        self.mark_all_dirty();

        let mut y = 0.0f32;
        let mut natural_width = 0.0f32;

        for (i, block_ref) in blocks.iter().enumerate() {
            let block = block_ref.as_ref();
            let entry = &mut self.entries[i];
            entry.y_offset = y;

            if entry.dirty {
                entry.height = layout_block::<P>(entry, block, available_width, style);
                entry.dirty = false;
            }

            natural_width = natural_width.max(entry.natural_width);
            y += entry.height;
            if !tight {
                y += block_spacing(block);
            }
        }

        self.last_layout_height = y;
        self.natural_width = natural_width;
        y
    }

    /// Width of the widest block when nothing makes it wrap, from the last
    /// layout pass (px).
    pub fn natural_width(&self) -> f32 {
        self.natural_width
    }

    /// Total height of all cached blocks (including spacing).
    ///
    /// This is the value returned by the last [`layout`] call. If no layout
    /// has been performed, returns 0.
    pub fn total_height(&self) -> f32 {
        self.last_layout_height
    }

    /// Find the block index at a given y-coordinate (relative to the editor top).
    ///
    /// Returns `None` if the cache is empty. If `y` is below the last block,
    /// returns the last block index.
    pub fn block_at_y(&self, y: f32) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }

        // Binary search: find the last entry whose y_offset <= y
        let mut lo = 0usize;
        let mut hi = self.entries.len();

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.entries[mid].y_offset <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        // lo is now the first entry with y_offset > y, so the block is lo - 1
        Some(lo.saturating_sub(1))
    }

    /// Iterate over all cache entries.
    pub fn entries(&self) -> &[CacheEntry<P>] {
        &self.entries
    }
}

// ── Block layout helper ─────────────────────────────────

/// Lay out a single block, storing the paragraph in `entry`.
/// Returns the height of the block.
fn layout_block<P: Paragraph>(
    entry: &mut CacheEntry<P>,
    block: &Block,
    available_width: f32,
    style: &LayoutStyle<'_>,
) -> f32 {
    let LayoutStyle {
        base_font,
        text_color,
        link_color,
        natural_size,
    } = *style;
    entry.image_size = None;
    let previous_table = entry.table.take();
    match block {
        Block::Table(table) => {
            let layout = layout_table::<P>(previous_table, table, available_width, style);
            entry.x_shift = 0.0;
            entry.paragraph = None;
            entry.child_paragraphs.clear();
            entry.natural_width = layout.width;
            let height = layout.height;
            entry.table = Some(layout);
            height
        }
        Block::Paragraph { .. } | Block::Heading { .. } | Block::ListItem { .. } => {
            // A list item's indent is its nesting, laid out with its marker;
            // a paragraph's or heading's is a plain left margin.
            let (indent, attrs) = if let Block::ListItem { indent_level, .. } = block {
                (
                    LIST_MARKER_WIDTH + (*indent_level as f32) * LIST_INDENT_PER_LEVEL,
                    BlockAttrs::default(),
                )
            } else {
                let attrs = block.attrs();
                (
                    f32::from(attrs.indent_level) * PARAGRAPH_INDENT_PER_LEVEL,
                    attrs,
                )
            };
            let content_width = (available_width - indent).max(0.0);
            let spans = build_spans_for_block(block, base_font, text_color, link_color);
            let font_size = block_font_size(block);

            let paragraph = build_aligned_paragraph::<P>(
                &spans,
                content_width,
                base_font,
                font_size,
                attrs.alignment,
                attrs.line_height,
            );

            // The paragraph comes back as wide as its text, its lines
            // aligned inside that box; where the box goes is ours to say.
            let slack = (content_width - paragraph.min_bounds().width).max(0.0);
            let aligned = match attrs.alignment {
                TextAlignment::Left | TextAlignment::Justify => 0.0,
                TextAlignment::Center => slack / 2.0,
                TextAlignment::Right => slack,
            };
            // The list indent is already part of the block's content edge.
            entry.x_shift = if matches!(block, Block::ListItem { .. }) {
                0.0
            } else {
                indent + aligned
            };

            let height = paragraph.min_bounds().height;
            entry.natural_width = indent + paragraph.min_bounds().width;
            entry.paragraph = Some(paragraph);
            entry.child_paragraphs.clear();
            height
        }
        Block::HorizontalRule => {
            entry.x_shift = 0.0;
            entry.natural_width = 0.0;
            entry.paragraph = None;
            entry.child_paragraphs.clear();
            HR_BLOCK_HEIGHT
        }
        Block::Image {
            src,
            alt,
            width,
            height,
        } => {
            if let Some(natural) = natural_size(src) {
                let (shown_width, shown_height) =
                    crate::image::display_size(*width, *height, natural, available_width);
                entry.x_shift = 0.0;
                entry.paragraph = None;
                entry.child_paragraphs.clear();
                entry.image_size = Some(Size::new(shown_width, shown_height));
                entry.natural_width = shown_width;
                return shown_height;
            }

            // Lay out alt text as a simple paragraph for placeholder display.
            let placeholder_text = if alt.is_empty() {
                "[image]"
            } else {
                alt.as_str()
            };
            let runs = [StyledRun::plain(placeholder_text)];
            let spans: Vec<_> = runs
                .iter()
                .map(|run| run_to_span(run, base_font, FONT_SIZE_BODY, text_color, link_color))
                .collect();
            let paragraph = build_paragraph::<P>(
                &spans,
                available_width - IMAGE_PLACEHOLDER_PADDING * 2.0,
                base_font,
                FONT_SIZE_BODY,
            );

            entry.x_shift = 0.0;
            entry.natural_width = paragraph.min_bounds().width + IMAGE_PLACEHOLDER_PADDING * 2.0;
            let text_height = paragraph.min_bounds().height;
            let block_height = height
                .map(|h| h as f32)
                .unwrap_or(IMAGE_PLACEHOLDER_HEIGHT)
                .max(text_height + IMAGE_PLACEHOLDER_PADDING * 2.0);

            entry.paragraph = Some(paragraph);
            entry.child_paragraphs.clear();
            block_height
        }
        Block::BlockQuote {
            blocks: bq_children,
        } => {
            // Lay out each child block as a separate paragraph within a
            // narrower width, storing them as child_paragraphs.
            let inner_width = (available_width - BLOCKQUOTE_INDENT).max(0.0);
            let mut children = Vec::with_capacity(bq_children.len());
            let mut y = 0.0f32;
            entry.natural_width = BLOCKQUOTE_INDENT;

            for (i, child) in bq_children.iter().enumerate() {
                let child_spans =
                    build_spans_for_any_block(child.as_ref(), base_font, text_color, link_color);
                let child_font_size = block_font_size(child.as_ref());
                let child_para =
                    build_paragraph::<P>(&child_spans, inner_width, base_font, child_font_size);
                let h = child_para.min_bounds().height;
                entry.natural_width = entry
                    .natural_width
                    .max(BLOCKQUOTE_INDENT + child_para.min_bounds().width);
                children.push(ChildParagraph {
                    paragraph: child_para,
                    local_y_offset: y,
                    height: h,
                });
                y += h;

                if i + 1 < bq_children.len() {
                    y += SPACING_PARAGRAPH;
                }
            }

            entry.x_shift = 0.0;
            entry.paragraph = None;
            entry.child_paragraphs = children;
            y
        }
    }
}

/// Build a left-aligned iced [`Paragraph`] at the editor's line height.
fn build_paragraph<P: Paragraph>(
    spans: &[Span<'_, String>],
    available_width: f32,
    base_font: Font,
    font_size: f32,
) -> P {
    build_aligned_paragraph(
        spans,
        available_width,
        base_font,
        font_size,
        TextAlignment::Left,
        None,
    )
}

/// Build an iced [`Paragraph`] from a set of spans. `line_height` is in
/// percent of the font size, `None` for the editor's own.
fn build_aligned_paragraph<P: Paragraph>(
    spans: &[Span<'_, String>],
    available_width: f32,
    base_font: Font,
    font_size: f32,
    alignment: TextAlignment,
    line_height: Option<u16>,
) -> P {
    use iced::advanced::text::Alignment;
    let text = Text {
        content: spans,
        bounds: Size::new(available_width, f32::INFINITY),
        size: iced::Pixels(font_size),
        line_height: iced::advanced::text::LineHeight::Relative(
            line_height.map_or(LINE_HEIGHT_MULTIPLIER, |percent| f32::from(percent) / 100.0),
        ),
        font: base_font,
        align_x: match alignment {
            TextAlignment::Left => Alignment::Default,
            TextAlignment::Center => Alignment::Center,
            TextAlignment::Right => Alignment::Right,
            TextAlignment::Justify => Alignment::Justified,
        },
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Advanced,
        wrapping: iced::advanced::text::Wrapping::Word,
        ellipsis: iced::advanced::text::Ellipsis::default(),
        hint_factor: None,
    };
    P::with_spans(text)
}

// ── Drawing helpers ─────────────────────────────────────

/// Draw a horizontal rule across the given bounds.
///
/// Renders a 1px line horizontally centered within `bounds`.
pub fn draw_horizontal_rule<R: iced::advanced::Renderer>(
    renderer: &mut R,
    bounds: Rectangle,
    color: Color,
) {
    let y_center = bounds.y + bounds.height / 2.0;
    renderer.fill_quad(
        renderer::Quad {
            bounds: Rectangle::new(
                Point::new(bounds.x, y_center - HR_LINE_HEIGHT / 2.0),
                Size::new(bounds.width, HR_LINE_HEIGHT),
            ),
            ..Default::default()
        },
        color,
    );
}

/// Draw the left border for a blockquote.
///
/// Renders a vertical line on the left side of the blockquote bounds.
pub fn draw_blockquote_border<R: iced::advanced::Renderer>(
    renderer: &mut R,
    bounds: Rectangle,
    color: Color,
) {
    renderer.fill_quad(
        renderer::Quad {
            bounds: Rectangle::new(
                Point::new(bounds.x, bounds.y),
                Size::new(BLOCKQUOTE_BORDER_WIDTH, bounds.height),
            ),
            ..Default::default()
        },
        color,
    );
}

/// Draw a list item marker (bullet or number).
///
/// For unordered lists, draws a bullet character. For ordered lists, draws
/// the item number. The marker is drawn in a fixed-width leading container
/// to the left of `content_bounds`.
///
/// `item_index` is the 0-based index of the item in the list.
pub fn draw_list_marker<R>(
    renderer: &mut R,
    content_bounds: Rectangle,
    ordered: bool,
    item_index: usize,
    base_font: Font,
    text_color: Color,
    clip_bounds: Rectangle,
) where
    R: iced::advanced::Renderer + iced::advanced::text::Renderer,
{
    let marker_text = if ordered {
        let mut s = (item_index + 1).to_string();
        s.push('.');
        s
    } else {
        BULLET_CHAR.to_owned()
    };

    let marker_para = <R::Paragraph as Paragraph>::with_text(Text {
        content: marker_text.as_str(),
        bounds: Size::new(LIST_MARKER_WIDTH, f32::INFINITY),
        size: iced::Pixels(FONT_SIZE_BODY),
        line_height: iced::advanced::text::LineHeight::Relative(LINE_HEIGHT_MULTIPLIER),
        font: base_font,
        align_x: iced::advanced::text::Alignment::Right,
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Advanced,
        wrapping: iced::advanced::text::Wrapping::None,
        ellipsis: iced::advanced::text::Ellipsis::default(),
        hint_factor: None,
    });

    let marker_x = content_bounds.x - LIST_MARKER_WIDTH;
    renderer.fill_paragraph(
        &marker_para,
        Point::new(marker_x, content_bounds.y),
        text_color,
        clip_bounds,
    );
}

/// Draw a filled paragraph at a given position.
///
/// Convenience wrapper around `renderer.fill_paragraph()`.
pub fn draw_paragraph<R>(
    renderer: &mut R,
    paragraph: &R::Paragraph,
    position: Point,
    text_color: Color,
    clip_bounds: Rectangle,
) where
    R: iced::advanced::text::Renderer,
{
    renderer.fill_paragraph(paragraph, position, text_color, clip_bounds);
}

// ── AsRef<Block> for Arc<Block> ─────────────────────────
// Already implemented by std, but we need it for the layout method.
// `Arc<Block>` implements `AsRef<Block>` automatically.

#[cfg(test)]
mod tests {
    use super::*;

    /// The paragraph type the editor is drawn with.
    type IcedParagraph = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph;

    /// A signature laid out the way mail clients write them: a logo cell of
    /// a fixed width, then a cell with a line on its left holding a table
    /// of its own, whose second row starts lower and holds an icon in its
    /// line of text.
    const LAID_OUT_SIGNATURE: &str = "<table border=\"0\" cellspacing=\"0\" cellpadding=\"0\" \
        style=\"width:520px\"><tr>\
        <td style=\"padding:0cm 13.5pt 0cm 0.75pt; width:150pt\">\
        <p style=\"text-align:center\"><a href=\"https://example.com\">\
        <img src=\"cid:logo\" width=\"180\" height=\"80\"></a></p></td>\
        <td style=\"border-left:1.5pt solid rgb(200, 200, 200); padding:0cm 0cm 0cm 13.5pt; \
        vertical-align:top\">\
        <table border=\"0\" cellspacing=\"0\" cellpadding=\"0\">\
        <tr><td style=\"padding:0cm\"><p>MonApp Support</p></td></tr>\
        <tr><td style=\"padding:13.5pt 0cm 0cm\"><p>\
        <img src=\"cid:icon\" width=\"12\" height=\"12\"> 01 23 45 67 89</p></td></tr>\
        </table></td></tr></table><p>Under the table</p>";

    fn lay_out(
        html: &str,
        width: f32,
    ) -> (crate::document::Document, ParagraphCache<IcedParagraph>) {
        let document = crate::html_parse::from_html(html);
        let mut cache = ParagraphCache::<IcedParagraph>::new();
        cache.layout(
            &document.blocks,
            width,
            Font::DEFAULT,
            Color::BLACK,
            Color::BLACK,
            &|_| None,
        );
        (document, cache)
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.5
    }

    #[test]
    fn a_laid_out_signature_keeps_its_columns_borders_and_nested_table() {
        let (_, cache) = lay_out(LAID_OUT_SIGNATURE, 600.0);
        let table_entry = cache.get(0).expect("table entry");
        let table = table_entry.table().expect("table layout");

        // The table takes the width it asks for, and the paragraph under
        // it starts where the table ends.
        assert!(close(table.width, 520.0), "table width {}", table.width);
        assert!(close(table_entry.height(), table.height));
        let under = cache.get(1).expect("paragraph entry");
        assert!(close(under.y_offset(), table.height));

        // The content of the logo cell is as wide as asked (150pt), its
        // paddings on top; the other cell takes the rest.
        let logo = table.cell(0, 0).expect("logo cell");
        let text = table.cell(0, 1).expect("text cell");
        assert!(close(logo.bounds.x, 0.0));
        assert!(
            close(logo.content_width, 200.0),
            "logo {}",
            logo.content_width
        );
        assert!(
            close(logo.bounds.width, 219.0),
            "logo {}",
            logo.bounds.width
        );
        assert!(close(text.bounds.x, 219.0));
        assert!(
            close(text.bounds.width, 301.0),
            "text {}",
            text.bounds.width
        );

        // Paddings and the left line push the content in.
        assert!(close(logo.content_origin.x, 1.0));
        assert!(close(text.content_origin.x, 219.0 + 2.0 + 18.0));
        assert_eq!(
            text.cell_box.border.left.map(|line| line.color),
            Some(Some(Rgb::new(200, 200, 200)))
        );

        // The row is at least as tall as the logo; the logo is centered in
        // it, the text cell starts at its top.
        assert!(logo.bounds.height >= 80.0);
        assert!(close(logo.bounds.height, text.bounds.height));
        assert!(close(text.content_origin.y, 0.0));
        assert!(logo.content_origin.y >= 0.0);

        // The table inside the text cell: two rows, the second pushed down
        // by its top padding, all of it inside the cell.
        let inner = text
            .cache
            .get(0)
            .and_then(CacheEntry::table)
            .expect("inner table");
        assert_eq!(inner.rows.len(), 2);
        let first = inner.cell(0, 0).expect("first inner cell");
        let second = inner.cell(1, 0).expect("second inner cell");
        assert!(first.bounds.height > 0.0);
        assert!(close(second.bounds.y, first.bounds.height));
        assert!(close(second.content_origin.y, second.bounds.y + 18.0));
        assert!(inner.width <= text.content_width + 0.5);
        // The line holding the icon is at least as tall as the icon.
        assert!(second.bounds.height >= 18.0 + 12.0);
    }

    #[test]
    fn an_image_in_a_line_holds_its_own_width_before_the_text_after_it() {
        let (_, cache) = lay_out(
            "<p><img src=\"cid:icon\" width=\"12\" height=\"12\">after</p>\
             <p style=\"text-align:center\"><img src=\"cid:logo\" width=\"200\" height=\"80\"></p>",
            600.0,
        );
        let line = cache.get(0).and_then(CacheEntry::paragraph).expect("line");
        let icon = line.span_bounds(0);
        let text = line.span_bounds(1);
        assert!(close(icon[0].width, 12.0), "icon held {icon:?}");
        assert!(close(text[0].x, 12.0), "text starts at {text:?}");

        // A centered image alone in its line sits in the middle.
        let logo = cache.get(1).expect("logo line");
        let held = logo.paragraph().expect("logo paragraph").span_bounds(0);
        assert!(close(held[0].width, 200.0), "logo held {held:?}");
        assert!(close(logo.x_shift() + held[0].x, 200.0), "logo at {held:?}");
    }

    #[test]
    fn an_empty_cell_is_as_tall_as_it_asks_and_a_table_too_wide_is_squeezed() {
        let (_, cache) = lay_out(
            "<table cellspacing=\"0\" cellpadding=\"0\"><tr>\
             <td style=\"height:36pt; width:400px\"></td>\
             <td style=\"width:400px\"></td></tr></table>",
            300.0,
        );
        let table = cache
            .get(0)
            .and_then(CacheEntry::table)
            .expect("table layout");
        assert!(close(table.height, 48.0), "height {}", table.height);
        assert!(close(table.width, 300.0), "width {}", table.width);
        let left = table.cell(0, 0).expect("left cell");
        let right = table.cell(0, 1).expect("right cell");
        assert!(close(left.bounds.width, 150.0));
        assert!(close(right.bounds.x, 150.0));
    }

    #[test]
    fn a_table_that_did_not_change_keeps_its_layout_from_one_pass_to_the_next() {
        let (document, mut cache) = lay_out(LAID_OUT_SIGNATURE, 600.0);
        let first_pass: *const TableLayout<IcedParagraph> =
            cache.get(0).and_then(CacheEntry::table).expect("layout");
        cache.layout(
            &document.blocks,
            600.0,
            Font::DEFAULT,
            Color::BLACK,
            Color::BLACK,
            &|_| None,
        );
        let second_pass: *const TableLayout<IcedParagraph> =
            cache.get(0).and_then(CacheEntry::table).expect("layout");
        assert!(std::ptr::eq(first_pass, second_pass));

        // Another width lays it out again.
        cache.layout(
            &document.blocks,
            400.0,
            Font::DEFAULT,
            Color::BLACK,
            Color::BLACK,
            &|_| None,
        );
        let narrower = cache.get(0).and_then(CacheEntry::table).expect("layout");
        assert!(narrower.width <= 400.5, "width {}", narrower.width);
    }

    #[test]
    fn font_size_constants_match_spec() {
        assert!((FONT_SIZE_BODY - 13.0).abs() < f32::EPSILON);
        assert!((FONT_SIZE_H1 - 18.0).abs() < f32::EPSILON);
        assert!((FONT_SIZE_H2 - 16.0).abs() < f32::EPSILON);
        assert!((FONT_SIZE_H3 - 14.0).abs() < f32::EPSILON);
    }

    #[test]
    fn block_font_size_paragraph() {
        let block = Block::empty_paragraph();
        assert!((block_font_size(&block) - FONT_SIZE_BODY).abs() < f32::EPSILON);
    }

    #[test]
    fn block_font_size_headings() {
        let h1 = Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H1,
            runs: vec![StyledRun::plain("test")],
        };
        assert!((block_font_size(&h1) - FONT_SIZE_H1).abs() < f32::EPSILON);

        let h2 = Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H2,
            runs: vec![StyledRun::plain("test")],
        };
        assert!((block_font_size(&h2) - FONT_SIZE_H2).abs() < f32::EPSILON);

        let h3 = Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H3,
            runs: vec![StyledRun::plain("test")],
        };
        assert!((block_font_size(&h3) - FONT_SIZE_H3).abs() < f32::EPSILON);
    }

    #[test]
    fn font_for_style_variants() {
        let base = Font::DEFAULT;

        let plain = font_for_style(base, InlineStyle::empty());
        assert_eq!(plain.weight, iced::font::Weight::Normal);
        assert_eq!(plain.style, iced::font::Style::Normal);

        let bold = font_for_style(base, InlineStyle::BOLD);
        assert_eq!(bold.weight, iced::font::Weight::Bold);
        assert_eq!(bold.style, iced::font::Style::Normal);

        let italic = font_for_style(base, InlineStyle::ITALIC);
        assert_eq!(italic.weight, iced::font::Weight::Normal);
        assert_eq!(italic.style, iced::font::Style::Italic);

        let bold_italic = font_for_style(base, InlineStyle::BOLD | InlineStyle::ITALIC);
        assert_eq!(bold_italic.weight, iced::font::Weight::Bold);
        assert_eq!(bold_italic.style, iced::font::Style::Italic);
    }

    #[test]
    fn block_spacing_values() {
        // A paragraph starts on the next line: the gap is an empty line.
        assert!(block_spacing(&Block::empty_paragraph()).abs() < f32::EPSILON);
        assert!(block_spacing(&Block::HorizontalRule) > 0.0);
    }

    #[test]
    fn block_font_size_list_item() {
        let block = Block::list_item("item", false);
        assert!((block_font_size(&block) - FONT_SIZE_BODY).abs() < f32::EPSILON);
    }

    #[test]
    fn block_font_size_blockquote() {
        let block = Block::BlockQuote {
            blocks: vec![std::sync::Arc::new(Block::paragraph("quoted"))],
        };
        assert!((block_font_size(&block) - FONT_SIZE_BODY).abs() < f32::EPSILON);
    }
}
