//! Document model for the rich text editor.
//!
//! Block tree with inline runs. Maps naturally to both HTML (DOM) and iced
//! rendering (column of `Paragraph` widgets).
//!
//! Blocks are wrapped in `Arc` for structural sharing: after an edit, only the
//! affected block gets a new allocation. Unchanged blocks are `Arc::clone`.

use std::ops::Range;
use std::sync::Arc;

use bitflags::bitflags;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn text_len(text: &str) -> usize {
    UnicodeSegmentation::graphemes(text, true).count()
}

pub(crate) fn text_to_byte_offset(text: &str, offset: usize) -> usize {
    UnicodeSegmentation::grapheme_indices(text, true)
        .nth(offset)
        .map_or(text.len(), |(byte_idx, _)| byte_idx)
}

// ── Inline style ────────────────────────────────────────

bitflags! {
    /// Inline formatting flags. Composable via bitwise OR.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct InlineStyle: u8 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const UNDERLINE     = 0b0000_0100;
        const STRIKETHROUGH = 0b0000_1000;
    }
}

// ── Run format ──────────────────────────────────────────

/// An opaque sRGB text color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// The color as `#rrggbb`.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Parse a CSS color written as `#rgb`, `#rrggbb` or `rgb(r, g, b)`.
    /// Named colors and colors with transparency are not read.
    pub fn parse_css(value: &str) -> Option<Self> {
        let value = value.trim();
        if let Some(hex) = value.strip_prefix('#') {
            let digit = |index: usize| {
                hex.get(index..=index)
                    .and_then(|d| u8::from_str_radix(d, 16).ok())
            };
            return match hex.len() {
                3 => Some(Self::new(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
                6 => Some(Self::new(
                    digit(0)? * 16 + digit(1)?,
                    digit(2)? * 16 + digit(3)?,
                    digit(4)? * 16 + digit(5)?,
                )),
                _ => None,
            };
        }
        let lower = value.to_ascii_lowercase();
        let channels = lower.strip_prefix("rgb(")?.strip_suffix(')')?;
        let mut parts = channels.split(',').map(|part| part.trim().parse::<u8>());
        let color = Self::new(
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        parts.next().is_none().then_some(color)
    }
}

/// A font family a run can ask for, by name. Any installed family can be
/// named; [`FontFamily::SAFE`] lists the ones a reader is likely to have,
/// each with the families to fall back on.
///
/// Names are interned, which keeps the type `Copy` like the rest of a
/// [`RunFormat`]. The set of names is bounded by the fonts the user picks
/// and the ones named in the mail they open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontFamily(&'static str);

/// The longest family name kept. Real names are far shorter; the cap bounds
/// what a hostile message can make the interner hold.
const FONT_NAME_MAX_CHARS: usize = 64;

/// The CSS generic families: fallbacks, never a font of their own.
const GENERIC_FAMILIES: [&str; 9] = [
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "inherit",
    "initial",
    "unset",
];

impl FontFamily {
    pub const ARIAL: Self = Self("Arial");
    pub const GEORGIA: Self = Self("Georgia");
    pub const COURIER_NEW: Self = Self("Courier New");

    /// The families a reader on macOS or Windows is likely to have, each
    /// with its CSS fallback list: the nearest families on the other
    /// system, then the generic one.
    pub const SAFE: [(Self, &'static str); 30] = [
        (Self("Andale Mono"), "'Courier New', monospace"),
        (Self::ARIAL, "Helvetica, sans-serif"),
        (Self("Arial Black"), "'Arial Bold', Arial, sans-serif"),
        (Self("Arial Narrow"), "Arial, sans-serif"),
        (
            Self("Baskerville"),
            "'Baskerville Old Face', Georgia, serif",
        ),
        (Self("Brush Script MT"), "cursive"),
        (Self("Calibri"), "Arial, sans-serif"),
        (Self("Cambria"), "Georgia, serif"),
        (Self("Century Gothic"), "Futura, Arial, sans-serif"),
        (Self("Comic Sans MS"), "'Comic Sans', cursive"),
        (Self("Consolas"), "Menlo, Monaco, monospace"),
        (Self::COURIER_NEW, "Courier, monospace"),
        (Self("Didot"), "'Bodoni MT', 'Times New Roman', serif"),
        (Self("Futura"), "'Century Gothic', Arial, sans-serif"),
        (Self("Garamond"), "Georgia, serif"),
        (Self::GEORGIA, "serif"),
        (Self("Gill Sans"), "'Gill Sans MT', Calibri, sans-serif"),
        (Self("Helvetica"), "Arial, sans-serif"),
        (Self("Helvetica Neue"), "Helvetica, Arial, sans-serif"),
        (Self("Impact"), "'Arial Black', sans-serif"),
        (Self("Lucida Grande"), "'Lucida Sans Unicode', sans-serif"),
        (Self("Menlo"), "Consolas, monospace"),
        (Self("Monaco"), "Consolas, monospace"),
        (Self("Optima"), "Candara, sans-serif"),
        (
            Self("Palatino"),
            "'Palatino Linotype', 'Book Antiqua', serif",
        ),
        (Self("Segoe UI"), "'Helvetica Neue', Arial, sans-serif"),
        (Self("Tahoma"), "Verdana, sans-serif"),
        (Self("Times New Roman"), "Times, serif"),
        (Self("Trebuchet MS"), "Tahoma, sans-serif"),
        (Self("Verdana"), "Geneva, sans-serif"),
    ];

    /// The family called `name`, or `None` if the name is not one a
    /// `font-family` value can carry: empty, a generic keyword, too long,
    /// or holding a character that would end the quoted name.
    pub fn new(name: &str) -> Option<Self> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > FONT_NAME_MAX_CHARS
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '\'' | '"' | ';' | '<' | '>' | '\\'))
            || GENERIC_FAMILIES
                .iter()
                .any(|generic| generic.eq_ignore_ascii_case(name))
        {
            return None;
        }
        if let Some((safe, _)) = Self::SAFE
            .iter()
            .find(|(safe, _)| safe.0.eq_ignore_ascii_case(name))
        {
            return Some(*safe);
        }
        Some(Self(intern_font_name(name)))
    }

    /// The family name, as shown to the user and as fonts are looked up.
    pub const fn name(self) -> &'static str {
        self.0
    }

    /// The CSS `font-family` value: the family, then what a reader without
    /// it falls back on.
    pub fn css(self) -> String {
        let fallback = Self::SAFE
            .iter()
            .find(|(safe, _)| *safe == self)
            .map_or("sans-serif", |(_, fallback)| fallback);
        if self.0.contains(' ') {
            format!("'{}', {fallback}", self.0)
        } else {
            format!("{}, {fallback}", self.0)
        }
    }

    /// The first family of a CSS `font-family` list, generic keywords
    /// aside.
    pub fn parse_css(value: &str) -> Option<Self> {
        value
            .split(',')
            .find_map(|family| Self::new(family.trim().trim_matches(['"', '\''])))
    }
}

/// The one shared copy of a family name. A name already seen, whatever its
/// letter case, comes back as first written.
fn intern_font_name(name: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut names = NAMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(known) = names.iter().find(|known| known.eq_ignore_ascii_case(name)) {
        return known;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.push(leaked);
    leaked
}

/// The smallest and largest text size a run can carry, in CSS pixels.
pub const RUN_SIZE_RANGE: std::ops::RangeInclusive<u16> = 1..=400;

/// Character formatting beyond the on/off [`InlineStyle`] flags. `None`
/// means automatic: the editor's own color, font and block size, which is
/// what lets unformatted text follow a light or dark theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RunFormat {
    pub color: Option<Rgb>,
    /// Highlight drawn behind the text.
    pub background: Option<Rgb>,
    pub font: Option<FontFamily>,
    /// Text size in CSS pixels.
    pub size: Option<u16>,
    /// The image this run stands for. Its text is then one
    /// [`INLINE_IMAGE_CHAR`], which holds the image's place in the line.
    pub image: Option<InlineImage>,
}

/// The character standing for an image in a line of text: the object
/// replacement character, which is not white space, so that trimming and
/// line wrapping leave it alone. It is laid out invisible, at the size that
/// makes it as wide as the image.
pub const INLINE_IMAGE_CHAR: char = '\u{FFFC}';

/// The side of the box an inline image of unknown size is shown in (px).
pub const INLINE_IMAGE_FALLBACK_SIDE: u32 = 24;

/// An image sitting in a line of text, among the characters.
///
/// Interned, so that a [`RunFormat`] stays `Copy`: the same source, alt
/// text and size always come back as the same shared copy, and two images
/// are equal exactly when they are that same copy.
#[derive(Debug, Clone, Copy)]
pub struct InlineImage(&'static InlineImageData);

/// What an [`InlineImage`] holds.
#[derive(Debug, PartialEq, Eq)]
pub struct InlineImageData {
    /// Image source: URL, data-URI or `cid:`.
    pub src: String,
    pub alt: String,
    /// Width it was given, in pixels.
    pub width: Option<u32>,
    /// Height it was given, in pixels.
    pub height: Option<u32>,
    /// Pixel size read from the file, for a base64 `data:` source of a
    /// format whose header is read.
    pub natural: Option<(u32, u32)>,
}

impl InlineImage {
    /// The shared copy of the image with this source, alt text and size.
    pub fn new(src: &str, alt: &str, width: Option<u32>, height: Option<u32>) -> Self {
        static IMAGES: std::sync::Mutex<Vec<&'static InlineImageData>> =
            std::sync::Mutex::new(Vec::new());
        let mut images = IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(known) = images.iter().find(|known| {
            known.width == width && known.height == height && known.alt == alt && known.src == src
        }) {
            return Self(known);
        }
        let natural =
            crate::image::data_uri_bytes(src).and_then(|bytes| crate::image::pixel_size(&bytes));
        let leaked: &'static InlineImageData = Box::leak(Box::new(InlineImageData {
            src: src.to_owned(),
            alt: alt.to_owned(),
            width,
            height,
            natural,
        }));
        images.push(leaked);
        Self(leaked)
    }

    /// Source, alt text and sizes.
    pub const fn data(self) -> &'static InlineImageData {
        self.0
    }

    /// The size the image is shown at, in pixels: the size it was given,
    /// completed from its own proportions, or its own size.
    pub fn shown_size(self) -> (f32, f32) {
        let data = self.0;
        let natural = data.natural.unwrap_or(match (data.width, data.height) {
            (Some(width), Some(height)) => (width, height),
            (Some(side), None) | (None, Some(side)) => (side, side),
            (None, None) => (INLINE_IMAGE_FALLBACK_SIDE, INLINE_IMAGE_FALLBACK_SIDE),
        });
        crate::image::display_size(data.width, data.height, natural, 0.0)
    }
}

impl PartialEq for InlineImage {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for InlineImage {}

impl std::hash::Hash for InlineImage {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.0, state);
    }
}

impl RunFormat {
    /// Whether every attribute is automatic.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// This format with `change` applied.
    #[must_use]
    pub fn with(mut self, change: FormatChange) -> Self {
        match change {
            FormatChange::Color(color) => self.color = color,
            FormatChange::Background(background) => self.background = background,
            FormatChange::Font(font) => self.font = font,
            FormatChange::Size(size) => self.size = size.map(clamp_run_size),
        }
        self
    }
}

/// `size` brought inside [`RUN_SIZE_RANGE`].
pub fn clamp_run_size(size: u16) -> u16 {
    size.clamp(*RUN_SIZE_RANGE.start(), *RUN_SIZE_RANGE.end())
}

/// One attribute of a [`RunFormat`] set to a value, `None` for automatic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatChange {
    Color(Option<Rgb>),
    Background(Option<Rgb>),
    Font(Option<FontFamily>),
    Size(Option<u16>),
}

// ── Styled run ──────────────────────────────────────────

/// A contiguous run of text sharing the same inline style.
///
/// The normalization invariant guarantees that adjacent runs within the same
/// block always have different `(style, format, link)` triples. After every
/// edit, adjacent runs with identical formatting merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledRun {
    pub text: String,
    pub style: InlineStyle,
    /// Color, font and size, each automatic unless set.
    pub format: RunFormat,
    /// Optional hyperlink href. `None` means plain text.
    pub link: Option<String>,
}

impl StyledRun {
    /// Create a plain (unstyled, unlinked) run.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: InlineStyle::empty(),
            format: RunFormat::default(),
            link: None,
        }
    }

    /// Create a styled run with no link.
    pub fn styled(text: impl Into<String>, style: InlineStyle) -> Self {
        Self {
            text: text.into(),
            style,
            format: RunFormat::default(),
            link: None,
        }
    }

    /// A run standing for `image` in a line of text.
    pub fn image(image: InlineImage) -> Self {
        Self {
            text: INLINE_IMAGE_CHAR.to_string(),
            style: InlineStyle::empty(),
            format: RunFormat {
                image: Some(image),
                ..RunFormat::default()
            },
            link: None,
        }
    }

    /// Create a linked run.
    pub fn linked(text: impl Into<String>, style: InlineStyle, href: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style,
            format: RunFormat::default(),
            link: Some(href.into()),
        }
    }

    /// This run with its format replaced.
    #[must_use]
    pub fn with_format(mut self, format: RunFormat) -> Self {
        self.format = format;
        self
    }

    /// The number of characters in this run.
    pub fn char_len(&self) -> usize {
        text_len(&self.text)
    }

    /// Whether this run has the same formatting (style + format + link) as
    /// `other`, so that the two can be one run. An image is a run of its
    /// own: it never joins another, not even the same image.
    pub fn same_formatting(&self, other: &Self) -> bool {
        self.format.image.is_none()
            && self.style == other.style
            && self.format == other.format
            && self.link == other.link
    }

    /// Whether this run is empty (zero characters).
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Split this run at a character offset, returning `(left, right)`.
    ///
    /// Panics if `char_offset` is out of bounds (but 0 and char_len are valid
    /// - they produce an empty left or right respectively).
    pub fn split_at(&self, char_offset: usize) -> (Self, Self) {
        let byte_offset = self.char_to_byte_offset(char_offset);
        let (left_text, right_text) = self.text.split_at(byte_offset);
        (
            Self {
                text: left_text.to_owned(),
                style: self.style,
                format: self.format,
                link: self.link.clone(),
            },
            Self {
                text: right_text.to_owned(),
                style: self.style,
                format: self.format,
                link: self.link.clone(),
            },
        )
    }

    /// Convert a char offset to a byte offset within this run's text.
    pub(crate) fn char_to_byte_offset(&self, char_offset: usize) -> usize {
        text_to_byte_offset(&self.text, char_offset)
    }
}

// ── Heading level ───────────────────────────────────────

/// Heading levels supported in email composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
}

impl HeadingLevel {
    /// Return the numeric level (1, 2, or 3).
    pub fn as_u8(self) -> u8 {
        match self {
            Self::H1 => 1,
            Self::H2 => 2,
            Self::H3 => 3,
        }
    }

    /// Create from a numeric level. Returns `None` for invalid levels.
    pub fn from_u8(level: u8) -> Option<Self> {
        match level {
            1 => Some(Self::H1),
            2 => Some(Self::H2),
            3 => Some(Self::H3),
            _ => None,
        }
    }
}

// ── Text alignment ──────────────────────────────────────

/// Text alignment for block-level elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlignment {
    /// Left-aligned (the default for LTR text).
    #[default]
    Left,
    /// Center-aligned.
    Center,
    /// Right-aligned.
    Right,
    /// Both edges aligned, the last line left-aligned.
    Justify,
}

impl TextAlignment {
    /// The CSS `text-align` keyword.
    pub const fn css(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "justify",
        }
    }

    /// Read a CSS `text-align` value or an `align` attribute.
    pub fn parse_css(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "left" | "start" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" | "end" => Some(Self::Right),
            "justify" => Some(Self::Justify),
            _ => None,
        }
    }
}

/// The deepest indent a block can take.
pub const MAX_INDENT_LEVEL: u8 = 8;

/// The line heights a block can take, in percent of the font size.
pub const LINE_HEIGHT_RANGE: std::ops::RangeInclusive<u16> = 80..=400;

// ── Block attributes ───────────────────────────────────

/// Block-level attributes that can be changed independently of the block type.
///
/// Used by `SetBlockAttrs` to modify alignment, indent and line height
/// without changing the block variant. Paragraphs and headings store all
/// three; a list item stores its indent only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockAttrs {
    /// Text alignment for the block content.
    pub alignment: TextAlignment,
    /// Indent level: nesting depth for a list item, left margin steps for a
    /// paragraph or heading.
    pub indent_level: u8,
    /// Line height in percent of the font size, `None` for the editor's own.
    pub line_height: Option<u16>,
}

impl BlockAttrs {
    /// Whether every attribute is at its default.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

// ── Block ───────────────────────────────────────────────

/// A block-level element in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A paragraph of styled text runs.
    Paragraph {
        attrs: BlockAttrs,
        runs: Vec<StyledRun>,
    },

    /// A heading (H1-H3) with styled text runs.
    Heading {
        level: HeadingLevel,
        attrs: BlockAttrs,
        runs: Vec<StyledRun>,
    },

    /// A single list item (bullet or numbered).
    ///
    /// Each list item is a top-level, cursor-addressable block. Consecutive
    /// `ListItem` blocks with matching `ordered` flags are grouped into a
    /// single `<ul>`/`<ol>` during HTML serialization.
    ListItem {
        ordered: bool,
        indent_level: u8,
        runs: Vec<StyledRun>,
    },

    /// A block quote containing nested blocks.
    BlockQuote { blocks: Vec<Arc<Block>> },

    /// A horizontal rule (thematic break).
    HorizontalRule,

    /// An inline image (block embed).
    ///
    /// Images are atomic blocks with no inline text content. The editor
    /// stores the `src` reference; the host application provides image data
    /// at render time.
    Image {
        /// Image source: URL, data-URI, `cid:`, or `inline-image:<hash>`.
        src: String,
        /// Alt text for accessibility and placeholder display.
        alt: String,
        /// Optional explicit width in pixels.
        width: Option<u32>,
        /// Optional explicit height in pixels.
        height: Option<u32>,
    },

    /// A table. Seen from the blocks around it, it is one position wide,
    /// like an image; each of its cells holds blocks of its own, reached
    /// through a [`CellStep`].
    Table(Arc<Table>),
}

// ── Table ───────────────────────────────────────────────

/// HTML attributes kept as they were read and written back unchanged, so
/// that what the editor does not understand (borders, paddings, widths)
/// still reaches the reader of the mail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HtmlAttrs(Vec<(String, String)>);

impl HtmlAttrs {
    /// Attributes from `(name, value)` pairs, in the order they are written.
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        Self(pairs)
    }

    /// The pairs, in the order they are written.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// The value of the attribute called `name`, whatever its letter case.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The value of a property of the `style` attribute. The last
    /// declaration of a property is the one that counts, as in CSS.
    pub fn style(&self, property: &str) -> Option<&str> {
        self.get("style")?
            .split(';')
            .filter_map(|declaration| declaration.split_once(':'))
            .filter(|(known, _)| known.trim().eq_ignore_ascii_case(property))
            .map(|(_, value)| value.trim())
            .next_back()
    }
}

/// A table: rows of cells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub attrs: HtmlAttrs,
    pub rows: Vec<TableRow>,
}

/// A row of a [`Table`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableRow {
    pub attrs: HtmlAttrs,
    pub cells: Vec<TableCell>,
}

/// A cell of a [`Table`]: a small document of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCell {
    pub attrs: HtmlAttrs,
    /// Whether the cell is a header cell (`<th>`).
    pub header: bool,
    /// The blocks of the cell, at least one.
    pub blocks: Vec<Arc<Block>>,
}

impl TableCell {
    /// A cell holding one empty paragraph.
    pub fn empty() -> Self {
        Self {
            attrs: HtmlAttrs::default(),
            header: false,
            blocks: vec![Arc::new(Block::empty_paragraph())],
        }
    }

    /// Whether the cell holds nothing but one empty paragraph.
    pub fn is_blank(&self) -> bool {
        matches!(
            self.blocks.as_slice(),
            [block] if matches!(block.as_ref(), Block::Paragraph { runs, .. }
                if runs.iter().all(StyledRun::is_empty))
        )
    }
}

impl Table {
    /// A table of `rows` by `columns` empty cells.
    pub fn grid(rows: usize, columns: usize) -> Self {
        Self {
            attrs: HtmlAttrs::default(),
            rows: (0..rows.max(1))
                .map(|_| TableRow {
                    attrs: HtmlAttrs::default(),
                    cells: (0..columns.max(1)).map(|_| TableCell::empty()).collect(),
                })
                .collect(),
        }
    }

    /// The cell at `row`, `column`.
    pub fn cell(&self, row: usize, column: usize) -> Option<&TableCell> {
        self.rows.get(row)?.cells.get(column)
    }

    /// The cell after (`forward`) or before the one at `row`, `column`, in
    /// reading order: along the row, then onto the next or previous row.
    pub fn neighbour(&self, row: usize, column: usize, forward: bool) -> Option<(usize, usize)> {
        let mut cells = self
            .rows
            .iter()
            .enumerate()
            .flat_map(|(row, cells)| (0..cells.cells.len()).map(move |column| (row, column)));
        if forward {
            cells.skip_while(|at| *at != (row, column)).nth(1)
        } else {
            let mut previous = None;
            cells.find_map(|at| {
                if at == (row, column) {
                    return Some(previous);
                }
                previous = Some(at);
                None
            })?
        }
    }
}

/// A change to the rows and columns of a table, made from one of its cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableChange {
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    DeleteRow,
    DeleteColumn,
    DeleteTable,
}

impl TableChange {
    /// `table` after this change made from its cell at `row`, `column`,
    /// with the cell the caret goes to. `None` when nothing of the table
    /// is left, or when there is no such cell.
    ///
    /// A new row or column takes the attributes of the one it is made
    /// from, so that it looks the same.
    pub fn apply(
        self,
        table: &Table,
        row: usize,
        column: usize,
    ) -> Option<(Table, (usize, usize))> {
        table.cell(row, column)?;
        let mut table = table.clone();
        let blank_like = |cell: &TableCell| TableCell {
            attrs: cell.attrs.clone(),
            header: cell.header,
            blocks: vec![Arc::new(Block::empty_paragraph())],
        };
        let target = match self {
            Self::InsertRowAbove | Self::InsertRowBelow => {
                let like = &table.rows[row];
                let new_row = TableRow {
                    attrs: like.attrs.clone(),
                    cells: like.cells.iter().map(blank_like).collect(),
                };
                let at = row + usize::from(self == Self::InsertRowBelow);
                table.rows.insert(at, new_row);
                (at, column)
            }
            Self::InsertColumnLeft | Self::InsertColumnRight => {
                let at = column + usize::from(self == Self::InsertColumnRight);
                for cells in &mut table.rows {
                    let Some(like) = cells.cells.get(column).or(cells.cells.last()) else {
                        continue;
                    };
                    let new_cell = blank_like(like);
                    let at = at.min(cells.cells.len());
                    cells.cells.insert(at, new_cell);
                }
                (row, at)
            }
            Self::DeleteRow => {
                table.rows.remove(row);
                (row, column)
            }
            Self::DeleteColumn => {
                for cells in &mut table.rows {
                    if column < cells.cells.len() {
                        cells.cells.remove(column);
                    }
                }
                table.rows.retain(|cells| !cells.cells.is_empty());
                (row, column)
            }
            Self::DeleteTable => return None,
        };
        // The caret goes to the nearest cell that is left.
        let row = target.0.min(table.rows.len().checked_sub(1)?);
        let column = target.1.min(table.rows[row].cells.len().checked_sub(1)?);
        Some((table, (row, column)))
    }
}

/// One step from a run of blocks into a table cell: the cell at `row`,
/// `column` of the table that is block `block_index`. A list of steps leads
/// through tables nested in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellStep {
    pub block_index: usize,
    pub row: usize,
    pub column: usize,
}

impl Block {
    /// Create an empty paragraph (the default block type).
    pub fn empty_paragraph() -> Self {
        Self::paragraph_of(vec![StyledRun::plain(String::new())])
    }

    /// Create a paragraph from plain text.
    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::paragraph_of(vec![StyledRun::plain(text)])
    }

    /// Create a list item from plain text.
    pub fn list_item(text: impl Into<String>, ordered: bool) -> Self {
        Self::ListItem {
            ordered,
            indent_level: 0,
            runs: vec![StyledRun::plain(text)],
        }
    }

    /// Create a list item with a specific indent level.
    pub fn list_item_with_indent(text: impl Into<String>, ordered: bool, indent_level: u8) -> Self {
        Self::ListItem {
            ordered,
            indent_level,
            runs: vec![StyledRun::plain(text)],
        }
    }

    /// Get the inline runs for this block, if it has any.
    ///
    /// Returns `Some` for `Paragraph`, `Heading`, and `ListItem`;
    /// `None` for all others.
    pub fn runs(&self) -> Option<&[StyledRun]> {
        match self {
            Self::Paragraph { runs, .. }
            | Self::Heading { runs, .. }
            | Self::ListItem { runs, .. } => Some(runs),
            _ => None,
        }
    }

    /// Get a mutable reference to the inline runs, if this block has any.
    pub fn runs_mut(&mut self) -> Option<&mut Vec<StyledRun>> {
        match self {
            Self::Paragraph { runs, .. }
            | Self::Heading { runs, .. }
            | Self::ListItem { runs, .. } => Some(runs),
            _ => None,
        }
    }

    /// Concatenate all run text into a single string.
    ///
    /// For blocks without inline runs (BlockQuote, HorizontalRule),
    /// returns an empty string. For Image blocks, returns the alt text.
    pub fn flattened_text(&self) -> String {
        match self {
            Self::Image { alt, .. } => alt.clone(),
            // Cells are separated by tabs, rows by line breaks.
            Self::Table(table) => table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| {
                            cell.blocks
                                .iter()
                                .map(|block| block.flattened_text())
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Self::BlockQuote { blocks } => {
                let mut buf = String::new();
                for (i, block) in blocks.iter().enumerate() {
                    if i > 0 {
                        buf.push('\n');
                    }
                    buf.push_str(&block.flattened_text());
                }
                buf
            }
            _ => match self.runs() {
                Some(runs) => {
                    let total_len: usize = runs.iter().map(|r| r.text.len()).sum();
                    let mut buf = String::with_capacity(total_len);
                    for run in runs {
                        buf.push_str(&run.text);
                    }
                    buf
                }
                None => String::new(),
            },
        }
    }

    /// Total character count of the block's flattened inline text.
    ///
    /// Non-inline blocks are atomic and return 1 so they can be selected,
    /// copied, and deleted as block-level objects.
    pub fn char_len(&self) -> usize {
        match self.runs() {
            Some(runs) => runs.iter().map(StyledRun::char_len).sum(),
            None => 1,
        }
    }

    /// Whether this block is a "leaf" block (has inline content, not children).
    pub fn is_inline_block(&self) -> bool {
        matches!(
            self,
            Self::Paragraph { .. } | Self::Heading { .. } | Self::ListItem { .. }
        )
    }

    /// Whether this block is a structural container (BlockQuote).
    pub fn is_container(&self) -> bool {
        matches!(self, Self::BlockQuote { .. })
    }

    /// Whether this block is selected/deleted as a single block-level atom.
    pub fn is_atomic_block(&self) -> bool {
        !self.is_inline_block()
    }

    /// Extract the block-level attributes from this block.
    ///
    /// A list item carries its indent only; blocks without inline text
    /// carry nothing.
    pub fn attrs(&self) -> BlockAttrs {
        match self {
            Self::Paragraph { attrs, .. } | Self::Heading { attrs, .. } => *attrs,
            Self::ListItem { indent_level, .. } => BlockAttrs {
                indent_level: *indent_level,
                ..BlockAttrs::default()
            },
            _ => BlockAttrs::default(),
        }
    }

    /// A paragraph holding `runs`, with default attributes.
    pub fn paragraph_of(runs: Vec<StyledRun>) -> Self {
        Self::Paragraph {
            attrs: BlockAttrs::default(),
            runs,
        }
    }

    /// A heading holding `runs`, with default attributes.
    pub fn heading_of(level: HeadingLevel, runs: Vec<StyledRun>) -> Self {
        Self::Heading {
            level,
            attrs: BlockAttrs::default(),
            runs,
        }
    }

    /// Return a clone of this block with the given attributes applied.
    ///
    /// Paragraphs and headings take all of them; a list item takes the
    /// indent; a block quote takes none and comes back unchanged.
    ///
    /// Returns `None` if the block type does not support any attributes
    /// (e.g., `HorizontalRule`, `Image`).
    pub fn with_attrs(&self, attrs: BlockAttrs) -> Option<Self> {
        match self {
            Self::ListItem { ordered, runs, .. } => Some(Self::ListItem {
                ordered: *ordered,
                indent_level: attrs.indent_level,
                runs: runs.clone(),
            }),
            Self::Paragraph { runs, .. } => Some(Self::Paragraph {
                attrs,
                runs: runs.clone(),
            }),
            Self::Heading { level, runs, .. } => Some(Self::Heading {
                level: *level,
                attrs,
                runs: runs.clone(),
            }),
            Self::BlockQuote { .. } => Some(self.clone()),
            Self::HorizontalRule | Self::Image { .. } | Self::Table(_) => None,
        }
    }

    /// The `BlockKind` discriminant (type without data). Used by `SetBlockType`.
    pub fn kind(&self) -> BlockKind {
        match self {
            Self::Paragraph { .. } => BlockKind::Paragraph,
            Self::Heading { level, .. } => BlockKind::Heading(*level),
            Self::ListItem { ordered, .. } => BlockKind::ListItem { ordered: *ordered },
            Self::BlockQuote { .. } => BlockKind::BlockQuote,
            Self::HorizontalRule => BlockKind::HorizontalRule,
            Self::Image { .. } => BlockKind::Image,
            Self::Table(_) => BlockKind::Table,
        }
    }

    /// Resolve a flattened char offset to `(run_index, char_offset_within_run)`.
    ///
    /// Returns `None` if this block has no inline runs or the offset is out of
    /// bounds. An offset equal to the block's `char_len()` resolves to the end
    /// of the last run.
    pub fn resolve_offset(&self, offset: usize) -> Option<(usize, usize)> {
        let runs = self.runs()?;
        let mut remaining = offset;
        for (i, run) in runs.iter().enumerate() {
            let len = run.char_len();
            if remaining < len || (remaining == len && i == runs.len() - 1) {
                return Some((i, remaining));
            }
            remaining -= len;
        }
        None
    }
}

// ── Block kind (discriminant without data) ──────────────

/// The type of a block without its content. Used in `SetBlockType` operations
/// to record what a block changed from/to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Paragraph,
    Heading(HeadingLevel),
    ListItem { ordered: bool },
    BlockQuote,
    HorizontalRule,
    Image,
    Table,
}

// ── Document position ───────────────────────────────────

/// A position within the document: block index + char offset within that
/// block's flattened inline text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocPosition {
    pub block_index: usize,
    pub offset: usize,
}

impl DocPosition {
    pub fn new(block_index: usize, offset: usize) -> Self {
        Self {
            block_index,
            offset,
        }
    }

    /// Position at the start of the document.
    pub fn zero() -> Self {
        Self {
            block_index: 0,
            offset: 0,
        }
    }
}

impl PartialOrd for DocPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DocPosition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.block_index
            .cmp(&other.block_index)
            .then(self.offset.cmp(&other.offset))
    }
}

// ── Document selection ──────────────────────────────────

/// A selection within the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocSelection {
    pub anchor: DocPosition,
    pub focus: DocPosition,
}

impl DocSelection {
    pub fn caret(pos: DocPosition) -> Self {
        Self {
            anchor: pos,
            focus: pos,
        }
    }

    pub fn range(anchor: DocPosition, focus: DocPosition) -> Self {
        Self { anchor, focus }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    pub fn start(&self) -> DocPosition {
        std::cmp::min(self.anchor, self.focus)
    }

    pub fn end(&self) -> DocPosition {
        std::cmp::max(self.anchor, self.focus)
    }

    pub fn block_range(&self) -> Range<usize> {
        let s = self.start();
        let e = self.end();
        s.block_index..e.block_index + 1
    }
}

// ── Document slice (clipboard) ──────────────────────────

/// A fragment of a document, used for clipboard copy/paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSlice {
    pub blocks: Vec<Block>,
    pub open_start: bool,
    pub open_end: bool,
}

impl DocSlice {
    pub fn single(block: Block) -> Self {
        Self {
            blocks: vec![block],
            open_start: false,
            open_end: false,
        }
    }

    pub fn inline_fragment(runs: Vec<StyledRun>) -> Self {
        Self {
            blocks: vec![Block::paragraph_of(runs)],
            open_start: true,
            open_end: true,
        }
    }
}

// ── Document ────────────────────────────────────────────

/// The root document. A sequence of blocks with `Arc` structural sharing.
///
/// Invariants (enforced by normalization):
/// - Always contains at least one block.
/// - Every inline block has at least one run (may be empty-string run).
/// - Adjacent runs within a block have different `(style, link)` pairs.
/// - `BlockQuote` contains at least one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Arc<Block>>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            blocks: vec![Arc::new(Block::empty_paragraph())],
        }
    }

    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            return Self::new();
        }
        Self {
            blocks: blocks.into_iter().map(Arc::new).collect(),
        }
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index).map(AsRef::as_ref)
    }

    pub fn replace_block(&mut self, index: usize, block: Block) -> Option<Arc<Block>> {
        self.blocks
            .get_mut(index)
            .map(|slot| std::mem::replace(slot, Arc::new(block)))
    }

    pub fn insert_block(&mut self, index: usize, block: Block) {
        self.blocks.insert(index, Arc::new(block));
    }

    pub fn remove_block(&mut self, index: usize) -> Option<Arc<Block>> {
        if self.blocks.len() <= 1 {
            return None;
        }
        Some(self.blocks.remove(index))
    }

    pub fn total_char_len(&self) -> usize {
        self.blocks.iter().map(|b| b.char_len()).sum()
    }

    pub fn flattened_text(&self) -> String {
        let mut buf = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 {
                buf.push('\n');
            }
            buf.push_str(&block.flattened_text());
        }
        buf
    }

    pub fn slice(&self, start: DocPosition, end: DocPosition) -> Option<DocSlice> {
        if start >= end {
            return None;
        }

        let mut blocks = Vec::new();
        let start_block = self.block(start.block_index)?;
        let end_block = self.block(end.block_index)?;

        if start.block_index == end.block_index {
            if start.offset == 0 && end.offset >= start_block.char_len() {
                return Some(DocSlice {
                    blocks: vec![start_block.clone()],
                    open_start: false,
                    open_end: false,
                });
            }
            if let Some(runs) = start_block.runs() {
                let extracted = extract_runs(runs, start.offset, end.offset);
                return Some(DocSlice {
                    blocks: vec![block_with_runs_like(start_block, extracted)],
                    open_start: true,
                    open_end: true,
                });
            }
            return None;
        }

        let open_start = start.offset > 0;
        let open_end = end.offset < end_block.char_len();

        if let Some(runs) = start_block.runs() {
            let extracted = extract_runs(runs, start.offset, start_block.char_len());
            blocks.push(block_with_runs_like(start_block, extracted));
        } else if start.offset == 0 {
            blocks.push(start_block.clone());
        }

        for i in (start.block_index + 1)..end.block_index {
            if let Some(block) = self.block(i) {
                blocks.push(block.clone());
            }
        }

        if let Some(runs) = end_block.runs() {
            let extracted = extract_runs(runs, 0, end.offset);
            blocks.push(block_with_runs_like(end_block, extracted));
        } else if end.offset > 0 {
            blocks.push(end_block.clone());
        }

        Some(DocSlice {
            blocks,
            open_start,
            open_end,
        })
    }

    pub fn is_valid_position(&self, pos: DocPosition) -> bool {
        if let Some(block) = self.block(pos.block_index) {
            pos.offset <= block.char_len()
        } else {
            false
        }
    }

    pub fn clamp_position(&self, pos: DocPosition) -> DocPosition {
        let block_index = pos.block_index.min(self.blocks.len().saturating_sub(1));
        let max_offset = self.block(block_index).map_or(0, Block::char_len);
        DocPosition::new(block_index, pos.offset.min(max_offset))
    }

    pub fn end_position(&self) -> DocPosition {
        let last_idx = self.blocks.len().saturating_sub(1);
        let last_len = self.block(last_idx).map_or(0, Block::char_len);
        DocPosition::new(last_idx, last_len)
    }

    /// The table `step` points at.
    pub fn table_at(&self, step: CellStep) -> Option<&Table> {
        match self.block(step.block_index)? {
            Block::Table(table) => Some(table),
            _ => None,
        }
    }

    /// The blocks of the cell `path` leads to, as a document of their own.
    /// An empty path is the document itself.
    pub fn cell_document(&self, path: &[CellStep]) -> Option<Self> {
        let mut blocks = &self.blocks;
        for step in path {
            let Block::Table(table) = blocks.get(step.block_index)?.as_ref() else {
                return None;
            };
            blocks = &table.cell(step.row, step.column)?.blocks;
        }
        Some(Self {
            blocks: blocks.clone(),
        })
    }

    /// Put `cell` back as the blocks of the cell `path` leads to. Returns
    /// false, leaving the document as it was, when the path leads nowhere.
    pub fn set_cell_document(&mut self, path: &[CellStep], cell: Self) -> bool {
        set_cell_blocks(&mut self.blocks, path, cell.blocks)
    }
}

fn set_cell_blocks(blocks: &mut Vec<Arc<Block>>, path: &[CellStep], new: Vec<Arc<Block>>) -> bool {
    let Some((step, rest)) = path.split_first() else {
        *blocks = new;
        return true;
    };
    let Some(slot) = blocks.get_mut(step.block_index) else {
        return false;
    };
    let Block::Table(table) = slot.as_ref() else {
        return false;
    };
    let mut table = Table::clone(table);
    let Some(cell) = table
        .rows
        .get_mut(step.row)
        .and_then(|row| row.cells.get_mut(step.column))
    else {
        return false;
    };
    if !set_cell_blocks(&mut cell.blocks, rest, new) {
        return false;
    }
    *slot = Arc::new(Block::Table(Arc::new(table)));
    true
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ─────────────────────────────────────────────

/// Rebuild `template`'s block type and attributes around a new run list.
/// Non-inline templates produce a plain `Paragraph`.
pub(crate) fn block_with_runs_like(template: &Block, runs: Vec<StyledRun>) -> Block {
    match template {
        Block::Heading { level, attrs, .. } => Block::Heading {
            level: *level,
            attrs: *attrs,
            runs,
        },
        Block::Paragraph { attrs, .. } => Block::Paragraph {
            attrs: *attrs,
            runs,
        },
        Block::ListItem {
            ordered,
            indent_level,
            ..
        } => Block::ListItem {
            ordered: *ordered,
            indent_level: *indent_level,
            runs,
        },
        _ => Block::paragraph_of(runs),
    }
}

fn extract_runs(runs: &[StyledRun], start_offset: usize, end_offset: usize) -> Vec<StyledRun> {
    if start_offset >= end_offset {
        return vec![StyledRun::plain(String::new())];
    }
    let mut result = Vec::new();
    let mut pos = 0;
    for run in runs {
        let run_len = run.char_len();
        let run_start = pos;
        let run_end = pos + run_len;
        if run_end <= start_offset {
            pos = run_end;
            continue;
        }
        if run_start >= end_offset {
            break;
        }
        let overlap_start = start_offset.max(run_start) - run_start;
        let overlap_end = end_offset.min(run_end) - run_start;
        let byte_start = run.char_to_byte_offset(overlap_start);
        let byte_end = run.char_to_byte_offset(overlap_end);
        let text = run.text[byte_start..byte_end].to_owned();
        result.push(StyledRun {
            text,
            style: run.style,
            format: run.format,
            link: run.link.clone(),
        });
        pos = run_end;
    }
    if result.is_empty() {
        result.push(StyledRun::plain(String::new()));
    }
    result
}

// ── Run utilities used by operations ────────────────────

/// Isolate the runs covering character range `[start..end)` within a run list.
pub fn isolate_runs(runs: &mut Vec<StyledRun>, start: usize, end: usize) -> Range<usize> {
    assert!(start <= end, "isolate_runs: start ({start}) > end ({end})");
    if runs.is_empty() || start == end {
        return 0..0;
    }
    let start_idx = split_runs_at(runs, start);
    let end_idx = split_runs_at(runs, end);
    start_idx..end_idx
}

fn split_runs_at(runs: &mut Vec<StyledRun>, offset: usize) -> usize {
    let mut pos = 0;
    for i in 0..runs.len() {
        let run_len = runs[i].char_len();
        if pos == offset {
            return i;
        }
        if pos + run_len > offset {
            let local_offset = offset - pos;
            let (left, right) = runs[i].split_at(local_offset);
            runs[i] = left;
            runs.insert(i + 1, right);
            return i + 1;
        }
        pos += run_len;
    }
    runs.len()
}

/// Public wrapper for [`split_runs_at`].
pub fn split_runs_at_char_offset(runs: &mut Vec<StyledRun>, offset: usize) -> usize {
    split_runs_at(runs, offset)
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_one_block() {
        let doc = Document::new();
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0).map(Block::char_len), Some(0));
    }

    #[test]
    fn styled_run_split_at() {
        let run = StyledRun::styled("hello world", InlineStyle::BOLD);
        let (left, right) = run.split_at(5);
        assert_eq!(left.text, "hello");
        assert_eq!(right.text, " world");
    }

    #[test]
    fn styled_run_offsets_are_grapheme_based() {
        let run = StyledRun::plain("a\u{301}b");
        assert_eq!(run.char_len(), 2);
        let (left, right) = run.split_at(1);
        assert_eq!(left.text, "a\u{301}");
        assert_eq!(right.text, "b");
    }

    #[test]
    fn block_resolve_offset() {
        let block = Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("hello"),
                StyledRun::styled(" world", InlineStyle::BOLD),
            ],
        };
        assert_eq!(block.resolve_offset(0), Some((0, 0)));
        assert_eq!(block.resolve_offset(5), Some((1, 0)));
        assert_eq!(block.resolve_offset(11), Some((1, 6)));
        assert_eq!(block.resolve_offset(12), None);
    }

    #[test]
    fn doc_position_ordering() {
        let a = DocPosition::new(0, 5);
        let b = DocPosition::new(1, 0);
        assert!(a < b);
    }

    #[test]
    fn isolate_runs_splits_correctly() {
        let mut runs = vec![
            StyledRun::plain("hello"),
            StyledRun::styled(" world", InlineStyle::BOLD),
        ];
        let range = isolate_runs(&mut runs, 3, 8);
        assert_eq!(runs.len(), 4);
        assert_eq!(range, 1..3);
    }

    #[test]
    fn list_item_is_inline_block() {
        let item = Block::list_item("hello", false);
        assert!(item.is_inline_block());
        assert!(!item.is_container());
        assert!(item.runs().is_some());
        assert_eq!(item.char_len(), 5);
        assert_eq!(item.kind(), BlockKind::ListItem { ordered: false });
    }

    #[test]
    fn image_block_char_len_is_atomic_unit() {
        let img = Block::Image {
            src: String::new(),
            alt: "x".into(),
            width: None,
            height: None,
        };
        assert_eq!(img.char_len(), 1);
    }

    #[test]
    fn slice_whole_heading_preserves_block_type() {
        let doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H2,
            runs: vec![StyledRun::plain("Title")],
        }]);
        let slice = doc
            .slice(DocPosition::new(0, 0), DocPosition::new(0, 5))
            .expect("slice");

        assert!(!slice.open_start);
        assert!(!slice.open_end);
        assert!(matches!(
            slice.blocks.first(),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H2,
                ..
            })
        ));
    }

    #[test]
    fn slice_multi_block_preserves_edge_block_types() {
        let doc = Document::from_blocks(vec![
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H1,
                runs: vec![StyledRun::plain("Title")],
            },
            Block::list_item_with_indent("item", true, 1),
        ]);
        let slice = doc
            .slice(DocPosition::new(0, 0), DocPosition::new(1, 4))
            .expect("slice");

        assert!(matches!(
            slice.blocks.first(),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert!(matches!(
            slice.blocks.get(1),
            Some(Block::ListItem {
                ordered: true,
                indent_level: 1,
                ..
            })
        ));
        assert!(!slice.open_start);
        assert!(!slice.open_end);
    }

    #[test]
    fn slice_partial_single_block_preserves_block_type() {
        let doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H2,
            runs: vec![StyledRun::plain("Title")],
        }]);
        let slice = doc
            .slice(DocPosition::new(0, 1), DocPosition::new(0, 4))
            .expect("slice");

        assert!(slice.open_start);
        assert!(slice.open_end);
        assert!(matches!(
            slice.blocks.first(),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H2,
                ..
            })
        ));
        assert_eq!(slice.blocks[0].flattened_text(), "itl");
    }

    #[test]
    fn slice_whole_atom_preserves_block() {
        let doc = Document::from_blocks(vec![Block::HorizontalRule]);
        let slice = doc
            .slice(DocPosition::new(0, 0), DocPosition::new(0, 1))
            .expect("slice");

        assert_eq!(slice.blocks, vec![Block::HorizontalRule]);
    }

    // ── BlockAttrs tests ────────────────────────────────

    #[test]
    fn list_item_attrs_returns_indent() {
        let item = Block::list_item_with_indent("hello", false, 3);
        let attrs = item.attrs();
        assert_eq!(attrs.indent_level, 3);
        assert_eq!(attrs.alignment, TextAlignment::Left);
    }

    #[test]
    fn paragraph_attrs_returns_defaults() {
        let para = Block::paragraph("hello");
        let attrs = para.attrs();
        assert_eq!(attrs, BlockAttrs::default());
    }

    #[test]
    fn list_item_with_attrs_sets_indent() {
        let item = Block::list_item("hello", true);
        let new_item = item
            .with_attrs(BlockAttrs {
                indent_level: 2,
                ..BlockAttrs::default()
            })
            .expect("with_attrs should succeed for ListItem");
        assert_eq!(new_item.attrs().indent_level, 2);
        assert_eq!(new_item.flattened_text(), "hello");
        assert!(matches!(new_item, Block::ListItem { ordered: true, .. }));
    }

    #[test]
    fn with_attrs_on_hr_returns_none() {
        let hr = Block::HorizontalRule;
        assert!(hr.with_attrs(BlockAttrs::default()).is_none());
    }

    #[test]
    fn with_attrs_on_paragraph_stores_them() {
        let para = Block::paragraph("hello");
        let attrs = BlockAttrs {
            alignment: TextAlignment::Center,
            indent_level: 5,
            line_height: Some(150),
        };
        let result = para.with_attrs(attrs).expect("a paragraph takes attrs");
        assert_eq!(result.attrs(), attrs);
        assert_eq!(result.flattened_text(), "hello");
    }

    #[test]
    fn a_list_item_keeps_the_indent_only() {
        let item = Block::list_item("hello", false)
            .with_attrs(BlockAttrs {
                alignment: TextAlignment::Right,
                indent_level: 2,
                line_height: Some(200),
            })
            .expect("a list item takes attrs");
        assert_eq!(
            item.attrs(),
            BlockAttrs {
                indent_level: 2,
                ..BlockAttrs::default()
            }
        );
    }

    #[test]
    fn text_alignment_default_is_left() {
        assert_eq!(TextAlignment::default(), TextAlignment::Left);
    }
}
