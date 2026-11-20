//! HTML -> Document parsing via html5ever.
//!
//! Narrow scope: only handles the editor's own HTML subset (drafts, signatures,
//! reply-quoted content). Does NOT handle arbitrary wild HTML -- that's litehtml-rs's job.
//!
//! The parser builds a simple DOM tree via html5ever's `TreeSink` trait, then
//! recursively walks it to produce `Block`s and `StyledRun`s.

mod dom;

use std::rc::Rc;
use std::sync::Arc;

use html5ever::Attribute;

use crate::document::{
    Block, BlockAttrs, Document, FontFamily, HeadingLevel, HtmlAttrs, INLINE_IMAGE_CHAR,
    InlineImage, InlineStyle, LINE_HEIGHT_RANGE, MAX_INDENT_LEVEL, RUN_SIZE_RANGE, Rgb, RunFormat,
    StyledRun, Table, TableCell, TableRow, TextAlignment,
};

use dom::{Handle, NodeData};

// ── Tag classification ──────────────────────────────────

fn is_block_element(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "div"
            | "hr"
            | "pre"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
            | "img"
    )
}

fn tag_to_inline_style(tag: &str) -> Option<InlineStyle> {
    match tag {
        "strong" | "b" => Some(InlineStyle::BOLD),
        "em" | "i" => Some(InlineStyle::ITALIC),
        "u" => Some(InlineStyle::UNDERLINE),
        "s" | "strike" | "del" => Some(InlineStyle::STRIKETHROUGH),
        _ => None,
    }
}

/// Extract the `href` attribute from an `<a>` element's attributes.
fn get_href(attrs: &[Attribute]) -> Option<String> {
    get_attr(attrs, "href")
}

/// Extract a named attribute value from an element's attributes.
fn get_attr(attrs: &[Attribute], name: &str) -> Option<String> {
    attrs.iter().find_map(|a| {
        if &*a.name.local == name {
            Some(a.value.to_string())
        } else {
            None
        }
    })
}

// ── Style stack ─────────────────────────────────────────

/// Accumulated inline style while walking inline elements.
#[derive(Clone)]
struct StyleContext {
    style: InlineStyle,
    format: RunFormat,
    link: Option<String>,
}

impl StyleContext {
    fn new() -> Self {
        Self {
            style: InlineStyle::empty(),
            format: RunFormat::default(),
            link: None,
        }
    }

    fn with_style(&self, extra: InlineStyle) -> Self {
        Self {
            style: self.style | extra,
            format: self.format,
            link: self.link.clone(),
        }
    }

    fn with_link(&self, href: String) -> Self {
        Self {
            style: self.style,
            format: self.format,
            link: Some(href),
        }
    }

    /// The context for the children of an inline element: its tag's style
    /// or link, then the color, font and size its attributes ask for.
    fn for_element(&self, tag: &str, attrs: &[Attribute]) -> Self {
        let mut child = if tag == "a" {
            match get_href(attrs) {
                Some(href) => self.with_link(href),
                None => self.clone(),
            }
        } else if let Some(inline_style) = tag_to_inline_style(tag) {
            self.with_style(inline_style)
        } else {
            // Unknown inline element or block-in-inline (shouldn't happen
            // in our own output, but handle gracefully).
            self.clone()
        };
        child.format = element_format(tag, attrs, child.format);
        child
    }

    fn formats(&self, run: &StyledRun) -> bool {
        run.style == self.style && run.format == self.format && run.link == self.link
    }
}

// ── Color, font and size ────────────────────────────────

/// `base` with the color, font and size an inline element asks for: its
/// `style` attribute, and the `color` / `face` / `size` of a `<font>`.
/// Values the editor cannot hold (named colors, relative sizes) leave the
/// inherited value in place.
fn element_format(tag: &str, attrs: &[Attribute], base: RunFormat) -> RunFormat {
    let mut format = base;
    if tag == "font" {
        if let Some(color) = get_attr(attrs, "color").and_then(|v| Rgb::parse_css(&v)) {
            format.color = Some(color);
        }
        if let Some(font) = get_attr(attrs, "face").and_then(|v| FontFamily::parse_css(&v)) {
            format.font = Some(font);
        }
        if let Some(size) = get_attr(attrs, "size").and_then(|v| legacy_font_size(&v)) {
            format.size = Some(size);
        }
    }
    let Some(style) = get_attr(attrs, "style") else {
        return format;
    };
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches("!important").trim();
        match property.trim().to_ascii_lowercase().as_str() {
            "color" => {
                if let Some(color) = Rgb::parse_css(value) {
                    format.color = Some(color);
                }
            }
            "background-color" | "background" => {
                if let Some(background) = Rgb::parse_css(value) {
                    format.background = Some(background);
                }
            }
            "font-family" => {
                if let Some(font) = FontFamily::parse_css(value) {
                    format.font = Some(font);
                }
            }
            "font-size" => {
                if let Some(size) = css_font_size(value) {
                    format.size = Some(size);
                }
            }
            _ => {}
        }
    }
    format
}

// ── Alignment, indent and line height ───────────────────

/// The left margin one indent level stands for, in CSS pixels. The
/// serializer writes the same step.
pub(crate) const INDENT_STEP_PX: u32 = 40;

/// The alignment, indent and line height a block element asks for.
fn node_block_attrs(node: &dom::Node) -> BlockAttrs {
    match node.data {
        NodeData::Element { ref attrs, .. } => element_block_attrs(attrs),
        _ => BlockAttrs::default(),
    }
}

/// Read a block element's `align` attribute and the `text-align`,
/// `margin-left` / `padding-left` and `line-height` of its `style`.
fn element_block_attrs(attrs: &[Attribute]) -> BlockAttrs {
    let mut block = BlockAttrs::default();
    if let Some(alignment) =
        get_attr(attrs, "align").and_then(|value| TextAlignment::parse_css(&value))
    {
        block.alignment = alignment;
    }
    let Some(style) = get_attr(attrs, "style") else {
        return block;
    };
    // A line height given as a length is turned into a share of the font
    // size once the whole style is read, the size possibly coming after it.
    let mut line_height_length = None;
    let mut font_size = None;
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches("!important").trim();
        match property.trim().to_ascii_lowercase().as_str() {
            "text-align" => {
                if let Some(alignment) = TextAlignment::parse_css(value) {
                    block.alignment = alignment;
                }
            }
            "margin-left" | "padding-left" => {
                if let Some(level) = css_indent_level(value) {
                    block.indent_level = level;
                }
            }
            "line-height" => {
                if let Some(percent) = css_line_height_percent(value) {
                    block.line_height = Some(percent);
                    line_height_length = None;
                } else if value.ends_with(|c: char| c.is_ascii_alphabetic()) {
                    line_height_length = crate::table_box::css_length(value);
                }
            }
            "font-size" => font_size = css_font_size(value).or(font_size),
            _ => {}
        }
    }
    if let Some(length) = line_height_length {
        let font_size = font_size.map_or(CSS_DEFAULT_FONT_SIZE, f32::from);
        let percent = crate::image::whole_pixels(length / font_size * 100.0);
        block.line_height = Some(
            u16::try_from(percent)
                .unwrap_or(*LINE_HEIGHT_RANGE.end())
                .clamp(*LINE_HEIGHT_RANGE.start(), *LINE_HEIGHT_RANGE.end()),
        );
    }
    block
}

/// The font size of text that asks for none, as HTML has it (px).
const CSS_DEFAULT_FONT_SIZE: f32 = 16.0;

/// The color, font and size a block element gives the text it holds. Its
/// background is the block's, not a highlight of the text.
fn node_run_format(tag: &str, node: &dom::Node) -> RunFormat {
    match node.data {
        NodeData::Element { ref attrs, .. } => RunFormat {
            background: None,
            ..element_format(tag, attrs, RunFormat::default())
        },
        _ => RunFormat::default(),
    }
}

/// The whole and hundredths parts of a plain decimal such as `1.5`.
fn decimal_hundredths(value: &str) -> Option<u32> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole: u32 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let mut hundredths = 0;
    let mut scale = 10;
    for digit in fraction.chars().take(2) {
        hundredths += digit.to_digit(10)? * scale;
        scale /= 10;
    }
    whole.checked_mul(100)?.checked_add(hundredths)
}

/// The indent level of a left margin given in pixels, to the nearest step.
fn css_indent_level(value: &str) -> Option<u8> {
    let pixels = decimal_hundredths(value.to_ascii_lowercase().strip_suffix("px")?.trim())? / 100;
    let level = (pixels + INDENT_STEP_PX / 2) / INDENT_STEP_PX;
    Some(u8::try_from(level).map_or(MAX_INDENT_LEVEL, |level| level.min(MAX_INDENT_LEVEL)))
}

/// A CSS `line-height` as a percentage of the font size, read from a bare
/// multiplier (`1.5`) or a percentage (`150%`), inside the range a block
/// can hold. Lengths and `normal` are not read.
fn css_line_height_percent(value: &str) -> Option<u16> {
    let percent = match value.strip_suffix('%') {
        Some(percent) => decimal_hundredths(percent.trim())? / 100,
        None => decimal_hundredths(value)?,
    };
    let percent = u16::try_from(percent).ok()?;
    Some(percent.clamp(*LINE_HEIGHT_RANGE.start(), *LINE_HEIGHT_RANGE.end()))
}

/// A CSS `font-size` in pixels, read from `px` and `pt` lengths only.
fn css_font_size(value: &str) -> Option<u16> {
    let lower = value.to_ascii_lowercase();
    let pixels = match lower.strip_suffix("px") {
        Some(px) => px.trim().parse::<f32>().ok()?,
        None => lower.strip_suffix("pt")?.trim().parse::<f32>().ok()? * 4.0 / 3.0,
    };
    clamp_run_size(pixels)
}

/// The pixel size of a `<font size="1">` to `<font size="7">`.
fn legacy_font_size(value: &str) -> Option<u16> {
    const SIZES: [u16; 7] = [10, 13, 16, 18, 24, 32, 48];
    let index = value.trim().parse::<usize>().ok()?;
    SIZES.get(index.checked_sub(1)?).copied()
}

fn clamp_run_size(pixels: f32) -> Option<u16> {
    if !pixels.is_finite() || pixels <= 0.0 {
        return None;
    }
    // The nearest size the editor holds, found without a float to integer
    // cast: the range is a few hundred values.
    RUN_SIZE_RANGE.min_by(|a, b| {
        let distance = |size: &u16| (f32::from(*size) - pixels).abs();
        distance(a).total_cmp(&distance(b))
    })
}

// ── Whitespace collapsing ───────────────────────────────

/// Collapse runs of whitespace into a single space, then trim leading/trailing.
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_ws = false;
    for ch in text.chars() {
        if ch.is_ascii_whitespace() || ch == '\u{a0}' {
            if !prev_was_ws {
                result.push(' ');
                prev_was_ws = true;
            }
        } else {
            result.push(ch);
            prev_was_ws = false;
        }
    }
    result
}

// ── DOM -> Document conversion ──────────────────────────

/// Collect inline runs from a node and its children, accumulating styles.
fn collect_inline_runs(node: &Handle, ctx: &StyleContext, runs: &mut Vec<StyledRun>) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Text(ref text) => {
            let collapsed = collapse_whitespace(text);
            if !collapsed.is_empty() {
                // Try to merge with the previous run if same formatting.
                let can_merge = runs.last().is_some_and(|last| ctx.formats(last));
                if can_merge && let Some(last) = runs.last_mut() {
                    last.text.push_str(&collapsed);
                    return;
                }
                runs.push(StyledRun {
                    text: collapsed,
                    style: ctx.style,
                    format: ctx.format,
                    link: ctx.link.clone(),
                });
            }
        }
        NodeData::Element {
            ref name,
            ref attrs,
            ..
        } => {
            let tag = name.local.as_ref();

            // If this is a <br>, add a newline-ish break. In inline context
            // we just emit a newline character that will be visible in the run.
            if tag == "br" {
                runs.push(StyledRun {
                    text: "\n".to_owned(),
                    style: ctx.style,
                    format: ctx.format,
                    link: ctx.link.clone(),
                });
                return;
            }

            // An image sits in the line as a run of its own, keeping the
            // link and the text size around it.
            if tag == "img" {
                let (width, height) = image_size(attrs);
                let image = InlineImage::new(
                    &get_attr(attrs, "src").unwrap_or_default(),
                    &get_attr(attrs, "alt").unwrap_or_default(),
                    width,
                    height,
                );
                runs.push(StyledRun {
                    text: INLINE_IMAGE_CHAR.to_string(),
                    style: InlineStyle::empty(),
                    format: RunFormat {
                        image: Some(image),
                        ..ctx.format
                    },
                    link: ctx.link.clone(),
                });
                return;
            }

            // Determine the new style context for children.
            let child_ctx = ctx.for_element(tag, attrs);

            for child in &borrow.children {
                collect_inline_runs(child, &child_ctx, runs);
            }
        }
        // Skip comments, doctypes, etc.
        _ => {}
    }
}

/// The width and height an `<img>` asks for, in pixels: its `width` and
/// `height` attributes, or the pixel lengths of its `style`.
fn image_size(attrs: &[Attribute]) -> (Option<u32>, Option<u32>) {
    let style = get_attr(attrs, "style").unwrap_or_default();
    let side = |name: &str| {
        get_attr(attrs, name)
            .and_then(|value| value.trim().parse::<u32>().ok())
            .or_else(|| {
                style.split(';').find_map(|declaration| {
                    let (property, value) = declaration.split_once(':')?;
                    if !property.trim().eq_ignore_ascii_case(name) {
                        return None;
                    }
                    let pixels = value.trim().to_ascii_lowercase();
                    let pixels = pixels.strip_suffix("px")?.trim().parse::<f32>().ok()?;
                    (pixels >= 1.0).then(|| crate::image::whole_pixels(pixels))
                })
            })
            .filter(|pixels| *pixels > 0)
    };
    (side("width"), side("height"))
}

/// The block an element holding images becomes: an image block when the
/// image is alone in a plain paragraph and carries no link, the element's
/// own block otherwise, with each image in its line. A paragraph that is
/// centered, indented or spaced keeps its image, so that it keeps its look.
fn image_or_inline_block(
    children: &[Handle],
    base: RunFormat,
    blocks: &mut Vec<Block>,
    wrap_runs: impl Fn(Vec<StyledRun>) -> Block,
) {
    let runs = collect_inline_children_with(children, base);
    let plain_paragraph = matches!(
        wrap_runs(Vec::new()),
        Block::Paragraph { attrs, .. } if attrs.is_plain()
    );
    if plain_paragraph
        && let [run] = runs.as_slice()
        && run.link.is_none()
        && let Some(image) = run.format.image
    {
        let data = image.data();
        blocks.push(Block::Image {
            src: data.src.clone(),
            alt: data.alt.clone(),
            width: data.width,
            height: data.height,
        });
        return;
    }
    blocks.push(wrap_runs(runs));
}

/// Convert a node to a list of blocks. This is the main recursive descent.
fn node_to_blocks(node: &Handle, blocks: &mut Vec<Block>) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Document => {
            for child in &borrow.children {
                node_to_blocks(child, blocks);
            }
        }
        NodeData::Text(ref text) => {
            // Bare text at block level: create a paragraph if non-whitespace.
            let collapsed = collapse_whitespace(text);
            let trimmed = collapsed.trim();
            if !trimmed.is_empty() {
                blocks.push(Block::paragraph(trimmed));
            }
        }
        NodeData::Element { ref name, .. } => {
            let tag = name.local.as_ref();

            match tag {
                "hr" => {
                    blocks.push(Block::HorizontalRule);
                }
                "img" => {
                    if let NodeData::Element { ref attrs, .. } = borrow.data {
                        let src = get_attr(attrs, "src").unwrap_or_default();
                        let alt = get_attr(attrs, "alt").unwrap_or_default();
                        let (width, height) = image_size(attrs);
                        blocks.push(Block::Image {
                            src,
                            alt,
                            width,
                            height,
                        });
                    }
                }
                "br" => {
                    // A <br> at block level creates an empty paragraph.
                    blocks.push(Block::empty_paragraph());
                }
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "td" | "th" | "pre" => {
                    parse_inline_element_to_blocks(tag, node, &borrow, blocks);
                }
                _ => {
                    // Container/structural elements and unknown tags.
                    // Some arms need to re-borrow node, so own the tag
                    // before dropping the borrow.
                    let tag = tag.to_owned();
                    drop(borrow);
                    parse_container_to_blocks(&tag, node, blocks);
                }
            }
        }
        // Comments, doctypes, PIs: skip
        _ => {}
    }
}

/// Handle inline/paragraph-like elements: `<p>`, headings, `<td>`/`<th>`, `<pre>`.
fn parse_inline_element_to_blocks(
    tag: &str,
    node: &Handle,
    borrow: &std::cell::Ref<'_, dom::Node>,
    blocks: &mut Vec<Block>,
) {
    let attrs = node_block_attrs(borrow);
    // The font, size and color a paragraph or a heading sets for its text.
    let base = node_run_format(tag, borrow);
    match tag {
        "p" => {
            if tree_has_img(&borrow.children) {
                image_or_inline_block(&borrow.children, base, blocks, |runs| Block::Paragraph {
                    attrs,
                    runs,
                });
            } else {
                let runs = collect_inline_children_with(&borrow.children, base);
                blocks.push(Block::Paragraph { attrs, runs });
            }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = match tag {
                "h1" => HeadingLevel::H1,
                "h2" => HeadingLevel::H2,
                // H4-H6 map to H3 per spec
                _ => HeadingLevel::H3,
            };
            let has_img = tree_has_img(&borrow.children);
            if has_img {
                image_or_inline_block(&borrow.children, base, blocks, |runs| Block::Heading {
                    level,
                    attrs,
                    runs,
                });
            } else {
                let runs = collect_inline_children_with(&borrow.children, base);
                blocks.push(Block::Heading { level, attrs, runs });
            }
        }
        "td" | "th" => {
            if tree_has_img(&borrow.children) {
                image_or_inline_block(&borrow.children, base, blocks, |runs| Block::Paragraph {
                    attrs,
                    runs,
                });
            } else {
                let runs = collect_element_runs(node, &borrow.children);
                if !runs_are_empty(&runs) {
                    blocks.push(Block::Paragraph { attrs, runs });
                }
            }
        }
        "pre" => {
            // Preserve text as-is (no whitespace collapsing).
            let mut text = String::new();
            collect_pre_text(node, &mut text);
            blocks.push(Block::paragraph(text));
        }
        _ => {}
    }
}

/// Handle container/structural elements: lists, blockquote, div, tables,
/// transparent wrappers, and unknown tags.
///
/// Called after the caller has dropped its borrow on `node`, so this
/// function re-borrows as needed.
fn parse_container_to_blocks(tag: &str, node: &Handle, blocks: &mut Vec<Block>) {
    match tag {
        "ul" | "ol" => {
            let ordered = tag == "ol";
            parse_list_to_items(node, ordered, 0, blocks);
        }
        "blockquote" => {
            let borrow = node.borrow();
            let mut inner_blocks = Vec::new();
            for child in &borrow.children {
                node_to_blocks(child, &mut inner_blocks);
            }
            if inner_blocks.is_empty() {
                inner_blocks.push(Block::empty_paragraph());
            }
            blocks.push(Block::BlockQuote {
                blocks: inner_blocks.into_iter().map(Arc::new).collect(),
            });
        }
        "div" => {
            let borrow = node.borrow();
            // <div> is a block container -- recurse into children.
            // If children contain only inline content, wrap in paragraph.
            let has_block_children = borrow.children.iter().any(|c| {
                let cb = c.borrow();
                if let NodeData::Element { ref name, .. } = cb.data {
                    is_block_element(name.local.as_ref())
                } else {
                    false
                }
            });

            if has_block_children {
                for child in &borrow.children {
                    node_to_blocks(child, blocks);
                }
            } else {
                // All inline children -- wrap in a paragraph.
                let attrs = node_block_attrs(&borrow);
                let runs = collect_element_runs(node, &borrow.children);
                blocks.push(Block::Paragraph { attrs, runs });
            }
        }
        "table" => {
            let table = parse_table(node);
            if !table.rows.is_empty() {
                blocks.push(Block::Table(Arc::new(table)));
            }
        }
        // Table parts met outside a table flatten to text paragraphs.
        "thead" | "tbody" | "tr" => {
            let borrow = node.borrow();
            for child in &borrow.children {
                node_to_blocks(child, blocks);
            }
        }
        // li outside a list context (shouldn't happen from our output)
        "li" => {
            let borrow = node.borrow();
            for child in &borrow.children {
                node_to_blocks(child, blocks);
            }
        }
        // Transparent wrappers from the html5ever fragment parser.
        "html" | "head" | "body" => {
            let borrow = node.borrow();
            for child in &borrow.children {
                node_to_blocks(child, blocks);
            }
        }
        _ => {
            // Unknown element. If block-level, recurse. If inline, wrap
            // in paragraph.
            if is_block_element(tag) {
                let borrow = node.borrow();
                for child in &borrow.children {
                    node_to_blocks(child, blocks);
                }
            } else {
                // Inline element at block level: collect as runs in a paragraph.
                let ctx = StyleContext::new();
                let mut runs = Vec::new();
                collect_inline_runs(node, &ctx, &mut runs);
                trim_runs(&mut runs);
                if runs.is_empty() {
                    runs.push(StyledRun::plain(String::new()));
                }
                if !runs_are_empty(&runs) {
                    blocks.push(Block::paragraph_of(runs));
                }
            }
        }
    }
}

// ── Tables ──────────────────────────────────────────────

/// Parse a `<table>` element: its rows, wherever a `<thead>`, `<tbody>` or
/// `<tfoot>` holds them, and their cells. Attributes are kept as written.
fn parse_table(node: &Handle) -> Table {
    let borrow = node.borrow();
    let mut table = Table {
        attrs: kept_attrs(&borrow),
        rows: Vec::new(),
    };
    collect_table_rows(&borrow.children, &mut table.rows);
    table
}

fn collect_table_rows(children: &[Handle], rows: &mut Vec<TableRow>) {
    for child in children {
        let node = child.borrow();
        let NodeData::Element { ref name, .. } = node.data else {
            continue;
        };
        match name.local.as_ref() {
            "tr" => {
                let cells: Vec<TableCell> =
                    node.children.iter().filter_map(parse_table_cell).collect();
                if !cells.is_empty() {
                    rows.push(TableRow {
                        attrs: kept_attrs(&node),
                        cells,
                    });
                }
            }
            "thead" | "tbody" | "tfoot" => collect_table_rows(&node.children, rows),
            _ => {}
        }
    }
}

fn parse_table_cell(handle: &Handle) -> Option<TableCell> {
    let node = handle.borrow();
    let NodeData::Element { ref name, .. } = node.data else {
        return None;
    };
    let header = match name.local.as_ref() {
        "td" => false,
        "th" => true,
        _ => return None,
    };
    let mut blocks = Vec::new();
    cell_blocks(&node.children, &mut blocks);
    if blocks.is_empty() {
        blocks.push(Block::empty_paragraph());
    }
    Some(TableCell {
        attrs: kept_attrs(&node),
        header,
        blocks: blocks.into_iter().map(Arc::new).collect(),
    })
}

/// The blocks of a cell: its block children as they are, and each stretch
/// of text and inline elements between them as a paragraph.
fn cell_blocks(children: &[Handle], blocks: &mut Vec<Block>) {
    let mut inline: Vec<Handle> = Vec::new();
    for child in children {
        let is_block = matches!(
            child.borrow().data,
            NodeData::Element { ref name, .. }
                if &*name.local != "img" && is_block_element(&name.local)
        );
        if is_block {
            flush_cell_inline(&mut inline, blocks);
            node_to_blocks(child, blocks);
        } else {
            inline.push(Rc::clone(child));
        }
    }
    flush_cell_inline(&mut inline, blocks);
}

fn flush_cell_inline(inline: &mut Vec<Handle>, blocks: &mut Vec<Block>) {
    if inline.is_empty() {
        return;
    }
    let runs = collect_inline_children(inline);
    inline.clear();
    if !runs_are_empty(&runs) {
        blocks.push(Block::paragraph_of(runs));
    }
}

/// The attributes of an element, to write back as they are. Event handlers
/// and names that are not plain attribute names are left out.
fn kept_attrs(node: &dom::Node) -> HtmlAttrs {
    let NodeData::Element { ref attrs, .. } = node.data else {
        return HtmlAttrs::default();
    };
    HtmlAttrs::new(
        attrs
            .iter()
            .filter_map(|attr| {
                let name: &str = &attr.name.local;
                let plain = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                let handler = name.to_ascii_lowercase().starts_with("on");
                (plain && !handler).then(|| (name.to_owned(), attr.value.to_string()))
            })
            .collect(),
    )
}

/// Parse a `<ul>`/`<ol>` element, flattening `<li>` children into
/// `Block::ListItem` blocks with the given indent level.
fn parse_list_to_items(
    list_node: &Handle,
    ordered: bool,
    indent_level: u8,
    blocks: &mut Vec<Block>,
) {
    let borrow = list_node.borrow();
    let mut found_any = false;
    for child in &borrow.children {
        let child_borrow = child.borrow();
        if let NodeData::Element { ref name, .. } = child_borrow.data
            && &*name.local == "li"
        {
            drop(child_borrow);
            parse_li_to_items(child, ordered, indent_level, blocks);
            found_any = true;
        }
        // Skip non-li children (whitespace text nodes, etc.)
    }
    if !found_any {
        blocks.push(Block::ListItem {
            ordered,
            indent_level,
            runs: vec![StyledRun::plain(String::new())],
        });
    }
}

/// Parse a single `<li>` element into one or more `Block::ListItem` blocks.
///
/// Inline content becomes a single `ListItem`. Nested `<ul>`/`<ol>` elements
/// recurse with `indent_level + 1`.
fn parse_li_to_items(li_node: &Handle, ordered: bool, indent_level: u8, blocks: &mut Vec<Block>) {
    let borrow = li_node.borrow();

    // Separate children into inline content and nested lists.
    // Inline content before the first nested list becomes this item's runs.
    // Nested lists recurse.
    let has_nested_list = borrow.children.iter().any(|c| {
        let cb = c.borrow();
        if let NodeData::Element { ref name, .. } = cb.data {
            let tag = &*name.local;
            return tag == "ul" || tag == "ol";
        }
        false
    });

    if !has_nested_list {
        // Simple case: all inline content.
        drop(borrow);
        let runs = collect_element_runs(li_node, &li_node.borrow().children);
        blocks.push(Block::ListItem {
            ordered,
            indent_level,
            runs,
        });
        return;
    }

    // Mixed content: collect inline runs before each nested list,
    // emit a ListItem for the inline parts, then recurse for nested lists.
    let mut inline_children: Vec<Handle> = Vec::new();
    let mut emitted_inline = false;

    for child in &borrow.children {
        let child_borrow = child.borrow();
        let is_nested_list = if let NodeData::Element { ref name, .. } = child_borrow.data {
            let tag = &*name.local;
            tag == "ul" || tag == "ol"
        } else {
            false
        };

        if is_nested_list {
            // Flush pending inline children as a ListItem. If the nested list is
            // the first meaningful content, keep an empty parent item.
            let runs = collect_inline_children(&inline_children);
            if !runs_are_empty(&runs) {
                blocks.push(Block::ListItem {
                    ordered,
                    indent_level,
                    runs,
                });
                emitted_inline = true;
            } else if !emitted_inline {
                blocks.push(Block::ListItem {
                    ordered,
                    indent_level,
                    runs: vec![StyledRun::plain(String::new())],
                });
                emitted_inline = true;
            }
            inline_children.clear();

            // Determine ordering of the nested list.
            let nested_ordered = if let NodeData::Element { ref name, .. } = child_borrow.data {
                &*name.local == "ol"
            } else {
                false
            };
            drop(child_borrow);
            parse_list_to_items(child, nested_ordered, indent_level + 1, blocks);
        } else {
            drop(child_borrow);
            inline_children.push(Rc::clone(child));
        }
    }

    // Emit trailing inline content too. The flat list-item model cannot keep
    // text after a nested list in the same item, so preserve it as another
    // item at the same indent instead of dropping it.
    if !inline_children.is_empty() {
        let runs = collect_inline_children(&inline_children);
        if !runs_are_empty(&runs) {
            blocks.push(Block::ListItem {
                ordered,
                indent_level,
                runs,
            });
        }
    } else if !emitted_inline {
        blocks.push(Block::ListItem {
            ordered,
            indent_level,
            runs: vec![StyledRun::plain(String::new())],
        });
    }
}

/// Collect inline runs from a list of child handles.
fn collect_inline_children(children: &[Handle]) -> Vec<StyledRun> {
    collect_inline_children_with(children, RunFormat::default())
}

/// Collect inline runs from a list of child handles, the text starting
/// from the color, font and size of `base`.
fn collect_inline_children_with(children: &[Handle], base: RunFormat) -> Vec<StyledRun> {
    let ctx = StyleContext {
        format: base,
        ..StyleContext::new()
    };
    let mut runs = Vec::new();
    for child in children {
        collect_inline_runs(child, &ctx, &mut runs);
    }
    trim_runs(&mut runs);
    if runs.is_empty() {
        runs.push(StyledRun::plain(String::new()));
    }
    runs
}

/// Check whether a node tree contains any `<img>` elements at any depth.
fn tree_has_img(children: &[Handle]) -> bool {
    children.iter().any(|c| {
        let cb = c.borrow();
        if let NodeData::Element { ref name, .. } = cb.data {
            if &*name.local == "img" {
                return true;
            }
            // Recurse into inline wrappers.
            if !is_block_element(name.local.as_ref()) {
                return tree_has_img(&cb.children);
            }
        }
        false
    })
}

/// Collect inline runs from an element's children.
fn collect_element_runs(_parent: &Handle, children: &[Handle]) -> Vec<StyledRun> {
    let ctx = StyleContext::new();
    let mut runs = Vec::new();
    for child in children {
        collect_inline_runs(child, &ctx, &mut runs);
    }
    trim_runs(&mut runs);
    if runs.is_empty() {
        runs.push(StyledRun::plain(String::new()));
    }
    runs
}

/// Collect text from a `<pre>` element without whitespace collapsing.
fn collect_pre_text(node: &Handle, buf: &mut String) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Text(ref text) => buf.push_str(text),
        NodeData::Element { .. } => {
            for child in &borrow.children {
                collect_pre_text(child, buf);
            }
        }
        _ => {}
    }
}

/// Trim leading whitespace from the first run and trailing whitespace from the
/// last run.
fn trim_runs(runs: &mut Vec<StyledRun>) {
    if let Some(first) = runs.first_mut() {
        let trimmed = first.text.trim_start().to_owned();
        first.text = trimmed;
    }
    if let Some(last) = runs.last_mut() {
        let trimmed = last.text.trim_end().to_owned();
        last.text = trimmed;
    }
    // Remove now-empty runs from the edges.
    while runs.len() > 1 && runs.first().is_some_and(|r| r.text.is_empty()) {
        runs.remove(0);
    }
    while runs.len() > 1 && runs.last().is_some_and(|r| r.text.is_empty()) {
        runs.pop();
    }
}

/// Check if a run list is effectively empty (all runs are empty or whitespace).
fn runs_are_empty(runs: &[StyledRun]) -> bool {
    runs.iter().all(|r| r.text.trim().is_empty())
}

// ── Public API ──────────────────────────────────────────

/// Parse HTML into a Document.
///
/// This parser only handles the editor's own output: `<p>`, `<h1>`-`<h6>`,
/// `<ul>`, `<ol>`, `<li>`, `<blockquote>`, `<hr>`, and inline formatting tags.
/// Unknown block elements become paragraphs; unknown inline elements pass
/// through content.
///
/// If the input is empty or unparseable, returns `Document::new()` (single
/// empty paragraph).
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn from_html(html: &str) -> Document {
    if html.trim().is_empty() {
        return Document::new();
    }

    let dom = dom::parse_html_fragment(html);
    let mut blocks = Vec::new();

    // The fragment parser wraps content in an <html> element. Walk into it.
    let root = dom.borrow();
    for child in &root.children {
        node_to_blocks(child, &mut blocks);
    }

    if blocks.is_empty() {
        return Document::new();
    }

    Document::from_blocks(blocks)
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, HeadingLevel, InlineStyle, StyledRun};

    #[test]
    fn table_round_trips_with_its_attributes_and_its_empty_cells() {
        use crate::html_serialize::to_html;
        let html = "<table border=\"0\" cellpadding=\"0\" style=\"width:520px\">\
            <tr><td style=\"padding:0cm 13.5pt 0cm 0.75pt;width:150pt\">\
            <p style=\"margin:0;text-align:center\">MonApp</p></td>\
            <td style=\"height:36pt\"></td></tr>\
            <tr><th colspan=\"2\"><p style=\"margin:0\">Support</p></th></tr></table>";
        let doc = from_html(html);
        assert_eq!(doc.block_count(), 1);
        let Some(Block::Table(table)) = doc.block(0) else {
            panic!("expected a table");
        };
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.attrs.get("cellpadding"), Some("0"));
        assert_eq!(table.rows[0].cells[0].attrs.style("width"), Some("150pt"));
        assert!(table.rows[0].cells[1].is_blank());
        assert!(table.rows[1].cells[0].header);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn text_written_straight_in_a_cell_becomes_a_paragraph_without_margins() {
        use crate::html_serialize::to_html;
        let doc = from_html(
            "<table><tbody><tr><td>plain <b>bold</b><p>second</p>tail</td></tr></tbody></table>",
        );
        assert_eq!(
            to_html(&doc),
            "<table><tr><td><p style=\"margin:0\">plain <strong>bold</strong></p>\
             <p style=\"margin:0\">second</p><p style=\"margin:0\">tail</p></td></tr></table>"
        );
    }

    #[test]
    fn nested_table_and_linked_image_in_a_cell_are_kept() {
        use crate::html_serialize::to_html;
        let html = "<table><tr><td><table><tr><td>\
            <p style=\"margin:0\"><a href=\"https://example.com\">\
            <img src=\"cid:logo\" width=\"31\" height=\"31\"></a> MonApp</p>\
            </td></tr></table></td></tr></table>";
        let doc = from_html(&html.replace("<td><table>", "<td><div><table>").replace(
            "</table></td></tr></table>",
            "</table></div></td></tr></table>",
        ));
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn event_handlers_are_not_kept_among_the_attributes_of_a_table() {
        use crate::html_serialize::to_html;
        let doc = from_html(
            "<table onclick=\"run()\" width=\"100\"><tr onmouseover=\"run()\">\
             <td ONLOAD=\"run()\" bgcolor=\"#ffffff\">x</td></tr></table>",
        );
        assert_eq!(
            to_html(&doc),
            "<table width=\"100\"><tr><td bgcolor=\"#ffffff\">\
             <p style=\"margin:0\">x</p></td></tr></table>"
        );
    }

    #[test]
    fn a_paragraph_gives_its_font_and_size_to_its_text_but_not_its_background() {
        let doc = from_html(
            "<p style=\"font-family:Georgia, serif; font-size:12pt; background-color:#ff0000\">\
             plain <span style=\"font-size:24pt\">large</span></p>",
        );
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].format.font, Some(FontFamily::GEORGIA));
        assert_eq!(runs[0].format.size, Some(16));
        assert_eq!(runs[0].format.background, None);
        // The span keeps the font of its paragraph and sets its own size.
        assert_eq!(runs[1].format.font, Some(FontFamily::GEORGIA));
        assert_eq!(runs[1].format.size, Some(32));
    }

    #[test]
    fn a_line_height_given_as_a_length_becomes_a_share_of_the_font_size() {
        let line_height = |style: &str| {
            from_html(&format!("<p style=\"{style}\">text</p>"))
                .block(0)
                .map(Block::attrs)
                .and_then(|attrs| attrs.line_height)
        };
        assert_eq!(line_height("line-height:24px; font-size:12pt"), Some(150));
        assert_eq!(line_height("font-size:12pt; line-height:18pt"), Some(150));
        // Without a size, the text is as large as HTML makes it: 16 pixels.
        assert_eq!(line_height("line-height:32px"), Some(200));
        assert_eq!(line_height("line-height:0px"), Some(80));
        assert_eq!(line_height("line-height:1.5"), Some(150));
        assert_eq!(line_height("line-height:normal"), None);
    }

    #[test]
    fn empty_string_produces_empty_document() {
        let doc = from_html("");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0), Some(&Block::empty_paragraph()));
    }

    #[test]
    fn whitespace_only_produces_empty_document() {
        let doc = from_html("   \n\t  ");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0), Some(&Block::empty_paragraph()));
    }

    #[test]
    fn simple_paragraph() {
        let doc = from_html("<p>hello</p>");
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("should have block");
        assert_eq!(
            *block,
            Block::Paragraph {
                attrs: Default::default(),
                runs: vec![StyledRun::plain("hello")]
            }
        );
    }

    #[test]
    fn paragraph_with_bold() {
        let doc = from_html("<p><strong>bold</strong> normal</p>");
        assert_eq!(doc.block_count(), 1);
        let runs = doc
            .block(0)
            .and_then(Block::runs)
            .expect("should have runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "bold");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[1].text, " normal");
        assert_eq!(runs[1].style, InlineStyle::empty());
    }

    #[test]
    fn heading_h1() {
        let doc = from_html("<h1>Title</h1>");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H1,
                runs: vec![StyledRun::plain("Title")]
            }
        );
    }

    #[test]
    fn heading_h2() {
        let doc = from_html("<h2>Subtitle</h2>");
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H2,
                runs: vec![StyledRun::plain("Subtitle")]
            }
        );
    }

    #[test]
    fn heading_h4_maps_to_h3() {
        let doc = from_html("<h4>Section</h4>");
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H3,
                runs: vec![StyledRun::plain("Section")]
            }
        );
    }

    #[test]
    fn heading_h6_maps_to_h3() {
        let doc = from_html("<h6>Deep</h6>");
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H3,
                runs: vec![StyledRun::plain("Deep")]
            }
        );
    }

    #[test]
    fn unordered_list() {
        let doc = from_html("<ul><li>one</li><li>two</li></ul>");
        assert_eq!(doc.block_count(), 2);
        if let Block::ListItem {
            ordered,
            runs,
            indent_level,
        } = doc.block(0).expect("block")
        {
            assert!(!ordered);
            assert_eq!(*indent_level, 0);
            assert_eq!(runs[0].text, "one");
        } else {
            panic!("expected ListItem");
        }
        if let Block::ListItem { ordered, runs, .. } = doc.block(1).expect("block") {
            assert!(!ordered);
            assert_eq!(runs[0].text, "two");
        } else {
            panic!("expected ListItem");
        }
    }

    #[test]
    fn ordered_list() {
        let doc = from_html("<ol><li>first</li></ol>");
        assert_eq!(doc.block_count(), 1);
        if let Block::ListItem { ordered, runs, .. } = doc.block(0).expect("block") {
            assert!(ordered);
            assert_eq!(runs[0].text, "first");
        } else {
            panic!("expected ListItem");
        }
    }

    #[test]
    fn blockquote() {
        let doc = from_html("<blockquote><p>quoted</p></blockquote>");
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        if let Block::BlockQuote { blocks } = block {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].flattened_text(), "quoted");
        } else {
            panic!("expected BlockQuote, got {block:?}");
        }
    }

    #[test]
    fn horizontal_rule() {
        let doc = from_html("<hr>");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(*doc.block(0).expect("block"), Block::HorizontalRule);
    }

    #[test]
    fn nested_styles_bold_italic() {
        let doc = from_html("<p><strong><em>bold italic</em></strong></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "bold italic");
        assert_eq!(runs[0].style, InlineStyle::BOLD | InlineStyle::ITALIC);
    }

    #[test]
    fn link() {
        let doc = from_html(r#"<p><a href="https://example.com">click</a></p>"#);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "click");
        assert_eq!(runs[0].link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn link_with_bold() {
        let doc =
            from_html(r#"<p><a href="https://example.com"><strong>bold link</strong></a></p>"#);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "bold link");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[0].link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn unknown_inline_tag_passes_through() {
        let doc = from_html("<p><span>text</span></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "text");
        assert_eq!(runs[0].style, InlineStyle::empty());
    }

    #[test]
    fn bare_text_without_tags() {
        let doc = from_html("hello world");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0).expect("block").flattened_text(), "hello world");
    }

    #[test]
    fn multiple_paragraphs() {
        let doc = from_html("<p>one</p><p>two</p><p>three</p>");
        assert_eq!(doc.block_count(), 3);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "one");
        assert_eq!(doc.block(1).expect("b").flattened_text(), "two");
        assert_eq!(doc.block(2).expect("b").flattened_text(), "three");
    }

    #[test]
    fn mixed_inline_styles() {
        let doc =
            from_html("<p>normal <b>bold</b> <i>italic</i> <u>underline</u> <s>strike</s></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].text, "normal ");
        assert_eq!(runs[0].style, InlineStyle::empty());
        assert_eq!(runs[1].text, "bold");
        assert_eq!(runs[1].style, InlineStyle::BOLD);
        // Space between bold and italic
        assert_eq!(runs[2].text, " ");
        assert_eq!(runs[3].text, "italic");
        assert_eq!(runs[3].style, InlineStyle::ITALIC);
        assert_eq!(runs[4].text, " ");
        assert_eq!(runs[5].text, "underline");
        assert_eq!(runs[5].style, InlineStyle::UNDERLINE);
        assert_eq!(runs[6].text, " ");
        assert_eq!(runs[7].text, "strike");
        assert_eq!(runs[7].style, InlineStyle::STRIKETHROUGH);
    }

    #[test]
    fn del_and_strike_tags() {
        let doc = from_html("<p><del>deleted</del> <strike>struck</strike></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].text, "deleted");
        assert_eq!(runs[0].style, InlineStyle::STRIKETHROUGH);
        assert_eq!(runs[2].text, "struck");
        assert_eq!(runs[2].style, InlineStyle::STRIKETHROUGH);
    }

    #[test]
    fn whitespace_collapsing() {
        let doc = from_html("<p>  hello   world  </p>");
        let text = doc.block(0).expect("block").flattened_text();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn nested_blockquote() {
        let doc =
            from_html("<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>");
        let block = doc.block(0).expect("block");
        if let Block::BlockQuote { blocks } = block {
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].flattened_text(), "outer");
            if let Block::BlockQuote {
                blocks: inner_blocks,
            } = blocks[1].as_ref()
            {
                assert_eq!(inner_blocks[0].flattened_text(), "inner");
            } else {
                panic!("expected nested BlockQuote");
            }
        } else {
            panic!("expected BlockQuote");
        }
    }

    #[test]
    fn hr_between_paragraphs() {
        let doc = from_html("<p>above</p><hr><p>below</p>");
        assert_eq!(doc.block_count(), 3);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "above");
        assert_eq!(*doc.block(1).expect("b"), Block::HorizontalRule);
        assert_eq!(doc.block(2).expect("b").flattened_text(), "below");
    }

    #[test]
    fn div_with_inline_content() {
        let doc = from_html("<div>hello</div>");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "hello");
    }

    #[test]
    fn div_with_block_children() {
        let doc = from_html("<div><p>first</p><p>second</p></div>");
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "first");
        assert_eq!(doc.block(1).expect("b").flattened_text(), "second");
    }

    #[test]
    fn html_entities_decoded() {
        let doc = from_html("<p>x &lt; y &amp; y &gt; z &quot;quoted&quot;</p>");
        assert_eq!(
            doc.block(0).expect("b").flattened_text(),
            "x < y & y > z \"quoted\""
        );
    }

    #[test]
    fn round_trip_simple() {
        use crate::html_serialize::to_html;
        let html = "<p>Hello, <strong>world</strong>!</p>";
        let doc = from_html(html);
        let output = to_html(&doc);
        assert_eq!(output, html);
    }

    #[test]
    fn round_trip_color_font_and_size() {
        use crate::html_serialize::to_html;
        let html = "<p>plain <span style=\"color:#cc0000;font-family:Georgia, serif;\
            font-size:18px\">loud</span> <span style=\"color:#0055aa\"><strong>blue</strong>\
            </span></p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);

        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(
            runs[1].format,
            RunFormat {
                color: Some(Rgb::new(0xcc, 0x00, 0x00)),
                background: None,
                font: Some(FontFamily::GEORGIA),
                size: Some(18),
                image: None,
            }
        );
        assert_eq!(runs[3].format.color, Some(Rgb::new(0x00, 0x55, 0xaa)));
        assert!(runs[3].style.contains(InlineStyle::BOLD));
    }

    #[test]
    fn font_tag_and_point_sizes_are_read() {
        let doc = from_html(
            "<p><font color=\"#00f\" face=\"Courier New\" size=\"5\">a</font>\
             <span style=\"font-size: 12pt; color: rgb(10, 20, 30)\">b</span></p>",
        );
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(
            runs[0].format,
            RunFormat {
                color: Some(Rgb::new(0x00, 0x00, 0xff)),
                background: None,
                font: Some(FontFamily::COURIER_NEW),
                size: Some(24),
                image: None,
            }
        );
        assert_eq!(runs[1].format.size, Some(16));
        assert_eq!(runs[1].format.color, Some(Rgb::new(10, 20, 30)));
    }

    #[test]
    fn formats_the_editor_cannot_hold_stay_automatic() {
        let doc = from_html(
            "<p><span style=\"color:red;font-family:sans-serif;font-size:1.2em\">a</span></p>",
        );
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert!(runs[0].format.is_plain());
    }

    #[test]
    fn any_named_family_is_kept_and_written_with_a_fallback() {
        use crate::html_serialize::to_html;
        let doc = from_html(
            "<p><span style=\"font-family: 'Avenir Next', Helvetica, sans-serif\">a</span>\
             <span style=\"font-family:comic sans ms\">b</span></p>",
        );
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(
            runs[0].format.font.map(FontFamily::name),
            Some("Avenir Next")
        );
        assert_eq!(
            runs[1].format.font.map(FontFamily::name),
            Some("Comic Sans MS"),
            "a listed family comes back under its listed name"
        );
        assert_eq!(
            to_html(&doc),
            "<p><span style=\"font-family:'Avenir Next', sans-serif\">a</span>\
             <span style=\"font-family:'Comic Sans MS', 'Comic Sans', cursive\">b</span></p>"
        );
    }

    #[test]
    fn family_names_that_would_break_the_style_attribute_are_dropped() {
        let long = "x".repeat(65);
        for name in ["a;color:red", "a<b", "", long.as_str()] {
            assert_eq!(FontFamily::new(name), None, "{name:?}");
        }
    }

    #[test]
    fn round_trip_alignment_indent_and_line_height() {
        use crate::html_serialize::to_html;
        let html = "<p style=\"text-align:center;margin-left:80px;line-height:150%\">a</p>\
            <h2 style=\"text-align:right\">b</h2><p style=\"text-align:justify\">c</p><p>d</p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
        assert_eq!(
            doc.block(0).map(Block::attrs),
            Some(BlockAttrs {
                alignment: TextAlignment::Center,
                indent_level: 2,
                line_height: Some(150),
            })
        );
    }

    #[test]
    fn block_attributes_are_read_from_what_other_clients_write() {
        let doc = from_html(
            "<p align=\"center\">a</p>\
             <div style=\"TEXT-ALIGN: right; line-height: 1.15; padding-left: 30px\">b</div>\
             <p style=\"line-height: normal; margin-left: 2em; text-align: middle\">c</p>",
        );
        let attrs = |index: usize| doc.block(index).map(Block::attrs);
        assert_eq!(attrs(0).map(|a| a.alignment), Some(TextAlignment::Center));
        assert_eq!(
            attrs(1),
            Some(BlockAttrs {
                alignment: TextAlignment::Right,
                indent_level: 1,
                line_height: Some(115),
            })
        );
        assert_eq!(attrs(2), Some(BlockAttrs::default()));
    }

    #[test]
    fn round_trip_highlight_and_extreme_sizes() {
        use crate::html_serialize::to_html;
        let html = "<p><span style=\"background-color:#ffee00;font-size:1px\">a</span>\
            <span style=\"font-size:400px\">b</span></p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);

        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].format.background, Some(Rgb::new(0xff, 0xee, 0x00)));

        let oversized = from_html("<p><span style=\"font-size:9000px\">a</span></p>");
        let runs = oversized.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].format.size, Some(400));
    }

    #[test]
    fn nested_formats_inherit_and_override() {
        let doc = from_html(
            "<p><span style=\"color:#112233;font-size:20px\">a\
             <span style=\"color:#445566\">b</span></span></p>",
        );
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].format.color, Some(Rgb::new(0x11, 0x22, 0x33)));
        assert_eq!(runs[1].format.color, Some(Rgb::new(0x44, 0x55, 0x66)));
        assert_eq!(runs[1].format.size, Some(20));
    }

    #[test]
    fn round_trip_heading() {
        use crate::html_serialize::to_html;
        let html = "<h1>Title</h1>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_list() {
        use crate::html_serialize::to_html;
        // Parse a list, serialize it, parse again, serialize again - should be stable.
        let html = "<ul><li>one</li><li>two</li></ul>";
        let doc = from_html(html);
        let output = to_html(&doc);
        assert_eq!(output, html);
    }

    #[test]
    fn round_trip_blockquote() {
        use crate::html_serialize::to_html;
        let html = "<blockquote><p>quoted</p></blockquote>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_hr() {
        use crate::html_serialize::to_html;
        let html = "<p>above</p><hr><p>below</p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_br() {
        use crate::html_serialize::to_html;
        let html = "<p>line one<br>line two</p>";
        let doc = from_html(html);
        assert_eq!(
            doc.block(0).expect("block").flattened_text(),
            "line one\nline two"
        );
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_br_inside_styled_signature_line() {
        use crate::html_serialize::to_html;
        // The signature shape that motivated the fix: name + title separated
        // by <br> inside one paragraph.
        let html = "<p><strong>Alice Smith</strong><br>Engineering Lead</p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_all_styles() {
        use crate::html_serialize::to_html;
        let html = "<p><a href=\"https://example.com\"><strong><em><u><s>styled</s></u></em></strong></a></p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_mixed_document() {
        use crate::html_serialize::to_html;
        let html = "<h1><strong>Welcome</strong></h1>\
                     <p>Some intro text.</p>\
                     <ul><li>Point A</li><li>Point B</li></ul>\
                     <hr>\
                     <blockquote><p>A wise quote.</p></blockquote>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn nested_list_round_trip() {
        use crate::html_serialize::to_html;
        let html = "<ol><li>outer item<ul><li>nested-a</li><li>nested-b</li></ul></li><li>second outer</li></ol>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn unicode_content() {
        let doc = from_html("<p>Héllo wörld \u{1f30d}</p>");
        assert_eq!(
            doc.block(0).expect("b").flattened_text(),
            "Héllo wörld \u{1f30d}"
        );
    }

    #[test]
    fn empty_paragraph_tag() {
        let doc = from_html("<p></p>");
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        assert_eq!(
            *block,
            Block::Paragraph {
                attrs: Default::default(),
                runs: vec![StyledRun::plain("")]
            }
        );
    }

    #[test]
    fn image_block_parsed() {
        let doc = from_html(r#"<img src="https://example.com/img.png" alt="A photo">"#);
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        if let Block::Image {
            src,
            alt,
            width,
            height,
        } = block
        {
            assert_eq!(src, "https://example.com/img.png");
            assert_eq!(alt, "A photo");
            assert_eq!(*width, None);
            assert_eq!(*height, None);
        } else {
            panic!("expected Image block, got {block:?}");
        }
    }

    #[test]
    fn image_block_with_dimensions() {
        let doc = from_html(r#"<img src="cid:abc" alt="logo" width="100" height="50">"#);
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        if let Block::Image {
            src,
            alt,
            width,
            height,
        } = block
        {
            assert_eq!(src, "cid:abc");
            assert_eq!(alt, "logo");
            assert_eq!(*width, Some(100));
            assert_eq!(*height, Some(50));
        } else {
            panic!("expected Image block, got {block:?}");
        }
    }

    #[test]
    fn image_inside_paragraph_stays_in_its_line() {
        use crate::html_serialize::to_html;
        let html = r#"<p>before<img src="test.png" alt="img" width="12" height="12">after</p>"#;
        let doc = from_html(html);
        assert_eq!(doc.block_count(), 1);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].text, INLINE_IMAGE_CHAR.to_string());
        let image = runs[1].format.image.expect("image").data();
        assert_eq!(
            (
                image.src.as_str(),
                image.alt.as_str(),
                image.width,
                image.height
            ),
            ("test.png", "img", Some(12), Some(12))
        );
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn an_image_alone_in_a_link_stays_in_the_line_to_keep_the_link() {
        use crate::html_serialize::to_html;
        let html = r#"<p><a href="https://example.com"><img src="cid:logo"></a></p>"#;
        let doc = from_html(html);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].format.image.is_some());
        assert_eq!(runs[0].link.as_deref(), Some("https://example.com"));
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn image_size_is_read_from_the_style_when_the_attributes_are_missing() {
        let doc = from_html(
            r#"<p>x<img src="cid:a" style="margin: 0; WIDTH: 200px; height:80.4px"></p>"#,
        );
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        let image = runs[1].format.image.expect("image").data();
        assert_eq!((image.width, image.height), (Some(200), Some(80)));

        // Lengths that are not pixels are left to the image's own size.
        let doc = from_html(r#"<p>x<img src="cid:b" style="width:0.33in"></p>"#);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        let image = runs[1].format.image.expect("image").data();
        assert_eq!((image.width, image.height), (None, None));
    }

    #[test]
    fn round_trip_image() {
        use crate::html_serialize::to_html;
        let html =
            r#"<img src="https://example.com/img.png" alt="A photo" width="100" height="50">"#;
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_image_minimal() {
        use crate::html_serialize::to_html;
        let html = r#"<img src="cid:abc">"#;
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn linked_image_in_a_heading_keeps_its_place_and_its_link() {
        use crate::html_serialize::to_html;
        let html =
            r#"<h1>before<a href="https://x.com"><img src="logo.png" alt="logo"></a>after</h1>"#;
        let doc = from_html(html);
        assert_eq!(doc.block_count(), 1);
        assert!(matches!(doc.block(0), Some(Block::Heading { .. })));
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 3);
        assert!(runs[1].format.image.is_some());
        assert_eq!(runs[1].link.as_deref(), Some("https://x.com"));
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn image_inside_strong_in_paragraph() {
        // <img> wrapped in <strong> inside a paragraph.
        let doc = from_html(r#"<p><strong><img src="pic.jpg" alt="pic"></strong></p>"#);
        let has_image =
            (0..doc.block_count()).any(|i| matches!(doc.block(i), Some(Block::Image { .. })));
        assert!(has_image, "expected an Image block");
    }

    #[test]
    fn list_item_with_nested_blocks() {
        let doc = from_html("<ul><li><p>text</p><ul><li>nested</li></ul></li></ul>");
        // Flattened model: first item at indent 0, nested item at indent 1.
        assert_eq!(doc.block_count(), 2);
        if let Block::ListItem {
            indent_level, runs, ..
        } = doc.block(0).expect("block")
        {
            assert_eq!(*indent_level, 0);
            assert_eq!(runs[0].text, "text");
        } else {
            panic!("expected ListItem at indent 0");
        }
        if let Block::ListItem {
            indent_level, runs, ..
        } = doc.block(1).expect("block")
        {
            assert_eq!(*indent_level, 1);
            assert_eq!(runs[0].text, "nested");
        } else {
            panic!("expected ListItem at indent 1");
        }
    }

    #[test]
    fn list_item_preserves_text_after_nested_list() {
        let doc = from_html("<ul><li>before<ul><li>nested</li></ul>after</li></ul>");
        assert_eq!(doc.block_count(), 3);
        assert_eq!(doc.block(0).expect("block").flattened_text(), "before");
        assert_eq!(doc.block(1).expect("block").flattened_text(), "nested");
        assert_eq!(doc.block(2).expect("block").flattened_text(), "after");
    }

    #[test]
    fn list_item_preserves_text_between_nested_lists() {
        let doc = from_html(
            "<ul><li>before<ul><li>nested-a</li></ul>middle<ol><li>nested-b</li></ol>after</li></ul>",
        );
        assert_eq!(doc.block_count(), 5);
        assert_eq!(doc.block(0).expect("block").flattened_text(), "before");
        assert_eq!(doc.block(1).expect("block").flattened_text(), "nested-a");
        assert_eq!(doc.block(2).expect("block").flattened_text(), "middle");
        assert_eq!(doc.block(3).expect("block").flattened_text(), "nested-b");
        assert_eq!(doc.block(4).expect("block").flattened_text(), "after");
    }

    #[test]
    fn html_parse_nested_list_indent_levels() {
        let doc =
            from_html("<ul><li>a</li><li>b<ol><li>b1</li><li>b2</li></ol></li><li>c</li></ul>");
        // Should produce: a(0), b(0), b1(1), b2(1), c(0)
        assert_eq!(doc.block_count(), 5);
        let expected = [
            ("a", 0u8, false),
            ("b", 0, false),
            ("b1", 1, true),
            ("b2", 1, true),
            ("c", 0, false),
        ];
        for (i, (text, indent, ordered)) in expected.iter().enumerate() {
            if let Block::ListItem {
                indent_level,
                runs,
                ordered: o,
            } = doc.block(i).expect("block")
            {
                assert_eq!(runs[0].text, *text, "block {i} text");
                assert_eq!(*indent_level, *indent, "block {i} indent");
                assert_eq!(*o, *ordered, "block {i} ordered");
            } else {
                panic!("block {i}: expected ListItem");
            }
        }
    }
}
