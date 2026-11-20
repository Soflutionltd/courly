//! Document → HTML serialization.
//!
//! Recursive walk of the document tree. Consistent nesting order for inline
//! styles: `<a><strong><em><u><s>text</s></u></em></strong></a>`
//!
//! Consecutive `ListItem` blocks with matching `ordered` flags are grouped
//! into a single `<ul>`/`<ol>`, with nesting determined by `indent_level`.

use crate::document::{
    Block, BlockAttrs, Document, HtmlAttrs, InlineStyle, RunFormat, StyledRun, TextAlignment,
};
use std::sync::Arc;

/// Serialize a document to HTML.
pub fn to_html(doc: &Document) -> String {
    let mut buf = String::new();
    let mut i = 0;
    while i < doc.blocks.len() {
        let Some(block) = doc.block(i) else {
            i += 1;
            continue;
        };
        if let Block::ListItem {
            ordered,
            indent_level,
            ..
        } = block
        {
            i = serialize_list_group(&doc.blocks, i, *ordered, *indent_level, &mut buf);
        } else {
            serialize_block(block, &mut buf);
            i += 1;
        }
    }
    buf
}

/// Serialize a group of consecutive `ListItem` blocks starting at `start`,
/// wrapping them in the appropriate `<ul>`/`<ol>` tags. Returns the index
/// of the first block after the group.
fn serialize_list_group(
    blocks: &[Arc<Block>],
    start: usize,
    ordered: bool,
    base_indent: u8,
    buf: &mut String,
) -> usize {
    let tag = if ordered { "ol" } else { "ul" };
    buf.push('<');
    buf.push_str(tag);
    buf.push('>');

    let mut i = start;
    while i < blocks.len() {
        let block = blocks[i].as_ref();
        match block {
            Block::ListItem {
                ordered: item_ordered,
                indent_level,
                runs,
            } if *item_ordered == ordered && *indent_level == base_indent => {
                buf.push_str("<li>");
                serialize_runs(runs, buf);

                // Check if the next block is a deeper-indented list item.
                if i + 1 < blocks.len()
                    && let Block::ListItem {
                        indent_level: next_indent,
                        ordered: next_ordered,
                        ..
                    } = blocks[i + 1].as_ref()
                    && *next_indent > base_indent
                {
                    i = serialize_list_group(blocks, i + 1, *next_ordered, *next_indent, buf);
                    buf.push_str("</li>");
                    continue;
                }

                buf.push_str("</li>");
                i += 1;
            }
            // A list item at a shallower indent or different ordered flag
            // ends this group.
            _ => break,
        }
    }

    buf.push_str("</");
    buf.push_str(tag);
    buf.push('>');
    i
}

fn serialize_block(block: &Block, buf: &mut String) {
    serialize_block_in(block, false, buf);
}

/// Serialize a block. `tight` is set inside a table cell: a paragraph is
/// then written without its outer margins, which a mail client would add
/// to the height of the cell.
fn serialize_block_in(block: &Block, tight: bool, buf: &mut String) {
    match block {
        Block::Paragraph { attrs, runs } => {
            buf.push_str("<p");
            push_block_attrs(*attrs, tight, buf);
            buf.push('>');
            serialize_runs(runs, buf);
            buf.push_str("</p>");
        }
        Block::Heading { level, attrs, runs } => {
            let n = level.as_u8();
            buf.push_str("<h");
            buf.push(char::from(b'0' + n));
            push_block_attrs(*attrs, tight, buf);
            buf.push('>');
            serialize_runs(runs, buf);
            buf.push_str("</h");
            buf.push(char::from(b'0' + n));
            buf.push('>');
        }
        Block::ListItem { .. } => {
            // ListItem blocks are handled by serialize_list_group via to_html.
            // If we reach here (e.g. from serialize_child_blocks in a
            // blockquote), wrap in a minimal list.
            //
            // This shouldn't normally happen in our own output, but handle
            // gracefully.
            if let Block::ListItem { ordered, runs, .. } = block {
                let tag = if *ordered { "ol" } else { "ul" };
                buf.push('<');
                buf.push_str(tag);
                buf.push_str("><li>");
                serialize_runs(runs, buf);
                buf.push_str("</li></");
                buf.push_str(tag);
                buf.push('>');
            }
        }
        Block::BlockQuote { blocks } => {
            buf.push_str("<blockquote>");
            serialize_child_blocks(blocks, tight, buf);
            buf.push_str("</blockquote>");
        }
        Block::HorizontalRule => {
            buf.push_str("<hr>");
        }
        Block::Image {
            src,
            alt,
            width,
            height,
        } => push_image_tag(src, alt, *width, *height, buf),
        Block::Table(table) => {
            buf.push_str("<table");
            push_html_attrs(&table.attrs, buf);
            buf.push('>');
            for row in &table.rows {
                buf.push_str("<tr");
                push_html_attrs(&row.attrs, buf);
                buf.push('>');
                for cell in &row.cells {
                    let tag = if cell.header { "th" } else { "td" };
                    buf.push('<');
                    buf.push_str(tag);
                    push_html_attrs(&cell.attrs, buf);
                    buf.push('>');
                    // A cell left empty stays empty: a spacer cell would
                    // grow by the height of a paragraph.
                    if !cell.is_blank() {
                        serialize_child_blocks(&cell.blocks, true, buf);
                    }
                    buf.push_str("</");
                    buf.push_str(tag);
                    buf.push('>');
                }
                buf.push_str("</tr>");
            }
            buf.push_str("</table>");
        }
    }
}

/// Write attributes kept from the HTML that was read.
fn push_html_attrs(attrs: &HtmlAttrs, buf: &mut String) {
    for (name, value) in attrs.pairs() {
        buf.push(' ');
        buf.push_str(name);
        buf.push_str("=\"");
        html_escape_into(value, buf);
        buf.push('"');
    }
}

/// Write an `<img>` tag. The size goes in the `width` and `height`
/// attributes, which every mail client reads.
fn push_image_tag(src: &str, alt: &str, width: Option<u32>, height: Option<u32>, buf: &mut String) {
    buf.push_str("<img src=\"");
    html_escape_into(src, buf);
    buf.push('"');
    if !alt.is_empty() {
        buf.push_str(" alt=\"");
        html_escape_into(alt, buf);
        buf.push('"');
    }
    if let Some(w) = width {
        buf.push_str(" width=\"");
        buf.push_str(&w.to_string());
        buf.push('"');
    }
    if let Some(h) = height {
        buf.push_str(" height=\"");
        buf.push_str(&h.to_string());
        buf.push('"');
    }
    buf.push('>');
}

fn serialize_child_blocks(blocks: &[Arc<Block>], tight: bool, buf: &mut String) {
    let mut i = 0;
    while i < blocks.len() {
        let block = blocks[i].as_ref();
        if let Block::ListItem {
            ordered,
            indent_level,
            ..
        } = block
        {
            i = serialize_list_group(blocks, i, *ordered, *indent_level, buf);
        } else {
            serialize_block_in(block, tight, buf);
            i += 1;
        }
    }
}

fn serialize_runs(runs: &[StyledRun], buf: &mut String) {
    for run in runs {
        if run.is_empty() {
            continue;
        }
        serialize_run(run, buf);
    }
}

fn serialize_run(run: &StyledRun, buf: &mut String) {
    // An image in the line: the tag itself, inside its link if it has one.
    if let Some(image) = run.format.image {
        let data = image.data();
        if let Some(href) = &run.link {
            buf.push_str("<a href=\"");
            html_escape_into(href, buf);
            buf.push_str("\">");
        }
        push_image_tag(&data.src, &data.alt, data.width, data.height, buf);
        if run.link.is_some() {
            buf.push_str("</a>");
        }
        return;
    }

    let has_link = run.link.is_some();
    let has_bold = run.style.contains(InlineStyle::BOLD);
    let has_italic = run.style.contains(InlineStyle::ITALIC);
    let has_underline = run.style.contains(InlineStyle::UNDERLINE);
    let has_strikethrough = run.style.contains(InlineStyle::STRIKETHROUGH);

    // Open tags: link → format → bold → italic → underline → strikethrough.
    // The format span sits inside the link so its color wins over the
    // reader's link color.
    if let Some(href) = &run.link {
        buf.push_str("<a href=\"");
        html_escape_into(href, buf);
        buf.push_str("\">");
    }
    let has_format = !run.format.is_plain();
    if has_format {
        buf.push_str("<span style=\"");
        push_format_css(run.format, buf);
        buf.push_str("\">");
    }
    if has_bold {
        buf.push_str("<strong>");
    }
    if has_italic {
        buf.push_str("<em>");
    }
    if has_underline {
        buf.push_str("<u>");
    }
    if has_strikethrough {
        buf.push_str("<s>");
    }

    // Text content (escaped). Newlines in run text come from parsed `<br>`
    // elements (see html_parse); emit them back as `<br>` so soft line
    // breaks survive the round trip instead of collapsing to whitespace.
    let mut segments = run.text.split('\n');
    if let Some(first) = segments.next() {
        html_escape_into(first, buf);
    }
    for segment in segments {
        buf.push_str("<br>");
        html_escape_into(segment, buf);
    }

    // Close tags: strikethrough → underline → italic → bold → link
    if has_strikethrough {
        buf.push_str("</s>");
    }
    if has_underline {
        buf.push_str("</u>");
    }
    if has_italic {
        buf.push_str("</em>");
    }
    if has_bold {
        buf.push_str("</strong>");
    }
    if has_format {
        buf.push_str("</span>");
    }
    if has_link {
        buf.push_str("</a>");
    }
}

/// A block's alignment, indent and line height as a `style` attribute, or
/// nothing when they are all at their defaults.
fn push_block_attrs(attrs: BlockAttrs, tight: bool, buf: &mut String) {
    if attrs.is_plain() && !tight {
        return;
    }
    let mut declarations: Vec<String> = Vec::new();
    if tight {
        declarations.push("margin:0".to_owned());
    }
    if attrs.alignment != TextAlignment::default() {
        declarations.push(format!("text-align:{}", attrs.alignment.css()));
    }
    if attrs.indent_level > 0 {
        let pixels = u32::from(attrs.indent_level) * crate::html_parse::INDENT_STEP_PX;
        declarations.push(format!("margin-left:{pixels}px"));
    }
    if let Some(percent) = attrs.line_height {
        declarations.push(format!("line-height:{percent}%"));
    }
    buf.push_str(" style=\"");
    buf.push_str(&declarations.join(";"));
    buf.push('"');
}

/// The CSS declarations of a run format, for a `style` attribute. Family
/// names are quoted with single quotes, so the value needs no escaping.
fn push_format_css(format: RunFormat, buf: &mut String) {
    let mut declarations: Vec<String> = Vec::new();
    if let Some(color) = format.color {
        declarations.push(format!("color:{}", color.to_hex()));
    }
    if let Some(background) = format.background {
        declarations.push(format!("background-color:{}", background.to_hex()));
    }
    if let Some(font) = format.font {
        declarations.push(format!("font-family:{}", font.css()));
    }
    if let Some(size) = format.size {
        declarations.push(format!("font-size:{size}px"));
    }
    buf.push_str(&declarations.join(";"));
}

/// HTML-escape `&`, `<`, `>`, and `"` into the buffer.
fn html_escape_into(text: &str, buf: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => buf.push_str("&amp;"),
            '<' => buf.push_str("&lt;"),
            '>' => buf.push_str("&gt;"),
            '"' => buf.push_str("&quot;"),
            _ => buf.push(ch),
        }
    }
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{HeadingLevel, InlineStyle};
    use std::sync::Arc;

    #[test]
    fn empty_document() {
        let doc = Document::new();
        assert_eq!(to_html(&doc), "<p></p>");
    }

    #[test]
    fn plain_paragraph() {
        let doc = Document::from_blocks(vec![Block::paragraph("Hello, world!")]);
        assert_eq!(to_html(&doc), "<p>Hello, world!</p>");
    }

    #[test]
    fn bold_italic_run() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled(
                "text",
                InlineStyle::BOLD | InlineStyle::ITALIC,
            )],
        }]);
        assert_eq!(to_html(&doc), "<p><strong><em>text</em></strong></p>");
    }

    #[test]
    fn all_inline_styles() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun {
                text: "styled".into(),
                style: InlineStyle::BOLD
                    | InlineStyle::ITALIC
                    | InlineStyle::UNDERLINE
                    | InlineStyle::STRIKETHROUGH,
                format: RunFormat::default(),
                link: Some("https://example.com".into()),
            }],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p><a href=\"https://example.com\"><strong><em><u><s>styled</s></u></em></strong></a></p>"
        );
    }

    #[test]
    fn multiple_runs_different_styles() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("normal "),
                StyledRun::styled("bold", InlineStyle::BOLD),
                StyledRun::plain(" and "),
                StyledRun::styled("italic", InlineStyle::ITALIC),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p>normal <strong>bold</strong> and <em>italic</em></p>"
        );
    }

    #[test]
    fn heading_levels() {
        let doc = Document::from_blocks(vec![
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H1,
                runs: vec![StyledRun::plain("Title")],
            },
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H2,
                runs: vec![StyledRun::plain("Subtitle")],
            },
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H3,
                runs: vec![StyledRun::plain("Section")],
            },
        ]);
        assert_eq!(
            to_html(&doc),
            "<h1>Title</h1><h2>Subtitle</h2><h3>Section</h3>"
        );
    }

    #[test]
    fn unordered_list() {
        let doc = Document::from_blocks(vec![
            Block::list_item("Alpha", false),
            Block::list_item("Beta", false),
            Block::list_item("Gamma", false),
        ]);
        assert_eq!(
            to_html(&doc),
            "<ul><li>Alpha</li><li>Beta</li><li>Gamma</li></ul>"
        );
    }

    #[test]
    fn ordered_list() {
        let doc = Document::from_blocks(vec![
            Block::list_item("First", true),
            Block::list_item("Second", true),
        ]);
        assert_eq!(to_html(&doc), "<ol><li>First</li><li>Second</li></ol>");
    }

    #[test]
    fn nested_list() {
        let doc = Document::from_blocks(vec![
            Block::list_item("outer item", true),
            Block::list_item_with_indent("nested-a", false, 1),
            Block::list_item_with_indent("nested-b", false, 1),
            Block::list_item("second outer", true),
        ]);
        assert_eq!(
            to_html(&doc),
            "<ol><li>outer item<ul><li>nested-a</li><li>nested-b</li></ul></li><li>second outer</li></ol>"
        );
    }

    #[test]
    fn blockquote_with_paragraphs() {
        let doc = Document::from_blocks(vec![Block::BlockQuote {
            blocks: vec![
                Arc::new(Block::paragraph("Line one")),
                Arc::new(Block::paragraph("Line two")),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<blockquote><p>Line one</p><p>Line two</p></blockquote>"
        );
    }

    #[test]
    fn horizontal_rule() {
        let doc = Document::from_blocks(vec![
            Block::paragraph("Above"),
            Block::HorizontalRule,
            Block::paragraph("Below"),
        ]);
        assert_eq!(to_html(&doc), "<p>Above</p><hr><p>Below</p>");
    }

    #[test]
    fn link_with_styles() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::linked(
                "click here",
                InlineStyle::BOLD,
                "https://example.com",
            )],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p><a href=\"https://example.com\"><strong>click here</strong></a></p>"
        );
    }

    #[test]
    fn html_escaping_in_text() {
        let doc = Document::from_blocks(vec![Block::paragraph("x < y & y > z \"quoted\"")]);
        assert_eq!(
            to_html(&doc),
            "<p>x &lt; y &amp; y &gt; z &quot;quoted&quot;</p>"
        );
    }

    #[test]
    fn html_escaping_in_link_href() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::linked(
                "link",
                InlineStyle::empty(),
                "https://example.com/search?q=a&b=c\"d",
            )],
        }]);
        assert_eq!(
            to_html(&doc),
            "<p><a href=\"https://example.com/search?q=a&amp;b=c&quot;d\">link</a></p>"
        );
    }

    #[test]
    fn empty_runs_skipped() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain(""),
                StyledRun::plain("visible"),
                StyledRun::styled("", InlineStyle::BOLD),
            ],
        }]);
        assert_eq!(to_html(&doc), "<p>visible</p>");
    }

    #[test]
    fn mixed_document() {
        let doc = Document::from_blocks(vec![
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H1,
                runs: vec![StyledRun::styled("Welcome", InlineStyle::BOLD)],
            },
            Block::paragraph("Some intro text."),
            Block::list_item("Point A", false),
            Block::list_item("Point B", false),
            Block::HorizontalRule,
            Block::BlockQuote {
                blocks: vec![Arc::new(Block::paragraph("A wise quote."))],
            },
        ]);
        assert_eq!(
            to_html(&doc),
            "<h1><strong>Welcome</strong></h1>\
             <p>Some intro text.</p>\
             <ul><li>Point A</li><li>Point B</li></ul>\
             <hr>\
             <blockquote><p>A wise quote.</p></blockquote>"
        );
    }

    #[test]
    fn underline_and_strikethrough() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled(
                "deleted",
                InlineStyle::UNDERLINE | InlineStyle::STRIKETHROUGH,
            )],
        }]);
        assert_eq!(to_html(&doc), "<p><u><s>deleted</s></u></p>");
    }

    #[test]
    fn blockquote_with_list_items_groups_correctly() {
        let doc = Document::from_blocks(vec![Block::BlockQuote {
            blocks: vec![
                Arc::new(Block::list_item("item one", false)),
                Arc::new(Block::list_item("item two", false)),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<blockquote><ul><li>item one</li><li>item two</li></ul></blockquote>"
        );
    }

    #[test]
    fn blockquote_nested_in_blockquote() {
        let doc = Document::from_blocks(vec![Block::BlockQuote {
            blocks: vec![
                Arc::new(Block::paragraph("Outer")),
                Arc::new(Block::BlockQuote {
                    blocks: vec![Arc::new(Block::paragraph("Inner"))],
                }),
            ],
        }]);
        assert_eq!(
            to_html(&doc),
            "<blockquote><p>Outer</p><blockquote><p>Inner</p></blockquote></blockquote>"
        );
    }

    #[test]
    fn unicode_content() {
        let doc = Document::from_blocks(vec![Block::paragraph("Héllo wörld \u{1F30D}")]);
        assert_eq!(to_html(&doc), "<p>Héllo wörld \u{1F30D}</p>");
    }

    #[test]
    fn newline_in_run_serializes_as_br() {
        let doc = Document::from_blocks(vec![Block::paragraph("line one\nline two")]);
        assert_eq!(to_html(&doc), "<p>line one<br>line two</p>");
    }

    #[test]
    fn newline_inside_styled_run_serializes_as_br() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("a\nb", InlineStyle::BOLD)],
        }]);
        assert_eq!(to_html(&doc), "<p><strong>a<br>b</strong></p>");
    }

    #[test]
    fn image_block_with_all_attributes() {
        let doc = Document::from_blocks(vec![Block::Image {
            src: "https://example.com/img.png".into(),
            alt: "A photo".into(),
            width: Some(100),
            height: Some(50),
        }]);
        assert_eq!(
            to_html(&doc),
            "<img src=\"https://example.com/img.png\" alt=\"A photo\" width=\"100\" height=\"50\">"
        );
    }

    #[test]
    fn image_block_without_optional_attributes() {
        let doc = Document::from_blocks(vec![Block::Image {
            src: "cid:abc123".into(),
            alt: String::new(),
            width: None,
            height: None,
        }]);
        assert_eq!(to_html(&doc), "<img src=\"cid:abc123\">");
    }

    #[test]
    fn image_block_escapes_src_and_alt() {
        let doc = Document::from_blocks(vec![Block::Image {
            src: "https://example.com/img?a=1&b=2".into(),
            alt: "A \"quoted\" image".into(),
            width: None,
            height: None,
        }]);
        assert_eq!(
            to_html(&doc),
            "<img src=\"https://example.com/img?a=1&amp;b=2\" alt=\"A &quot;quoted&quot; image\">"
        );
    }

    #[test]
    fn list_item_has_runs_and_char_len() {
        let item = Block::list_item("hello", false);
        assert!(item.runs().is_some());
        assert_eq!(item.char_len(), 5);
        assert!(item.is_inline_block());
        assert!(!item.is_container());
    }

    #[test]
    fn consecutive_list_items_produce_ul_wrapper() {
        let doc = Document::from_blocks(vec![
            Block::list_item("one", false),
            Block::list_item("two", false),
        ]);
        assert_eq!(to_html(&doc), "<ul><li>one</li><li>two</li></ul>");
    }

    #[test]
    fn mixed_indent_levels_produce_nested_lists() {
        let doc = Document::from_blocks(vec![
            Block::list_item("top", true),
            Block::list_item_with_indent("nested", true, 1),
            Block::list_item("top2", true),
        ]);
        assert_eq!(
            to_html(&doc),
            "<ol><li>top<ol><li>nested</li></ol></li><li>top2</li></ol>"
        );
    }
}
