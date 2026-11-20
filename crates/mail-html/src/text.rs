//! The text of a laid out email with the position of every character, so
//! the UI can select and copy text from what is otherwise a picture.

use blitz_dom::{BaseDocument, Node};

/// One character (grapheme cluster) of a [`TextLine`].
#[derive(Debug, Clone, PartialEq)]
pub struct TextCell {
    /// Byte range of the character inside the line's `text`.
    pub start: usize,
    pub end: usize,
    /// Left edge and width in display px.
    pub x: f32,
    pub width: f32,
}

/// One line box of text. Lines come in document order.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// Lines wrapped from the same block of text share this number; a new
    /// number means the text starts on a new line when copied.
    pub block: usize,
    /// Top edge and height in display px.
    pub y: f32,
    pub height: f32,
    pub text: String,
    /// The characters of `text`, ordered by `start`.
    pub cells: Vec<TextCell>,
}

/// Every line of text of a resolved document, in document order.
pub(crate) fn collect_text(doc: &BaseDocument, zoom: f32) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut block = 0;
    let mut pending = vec![doc.root_element().id];
    while let Some(id) = pending.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.flags.is_inline_root() && push_block(node, block, zoom, &mut lines) {
            block += 1;
        }
        if let Some(children) = node.layout_children.borrow().as_ref() {
            pending.extend(children.iter().rev().copied());
        }
    }
    lines
}

/// Append the lines of one inline root. Returns whether it had any text.
fn push_block(node: &Node, block: usize, zoom: f32, lines: &mut Vec<TextLine>) -> bool {
    let Some(inline) = node
        .element_data()
        .and_then(|element| element.inline_layout_data.as_deref())
    else {
        return false;
    };
    let layout = &inline.layout;
    let scale = layout.scale();
    if scale <= 0.0 {
        return false;
    }
    // Parley positions are relative to the content box of the inline root.
    let box_layout = node.final_layout();
    let origin = node.absolute_position(
        box_layout.padding.left + box_layout.border.left,
        box_layout.padding.top + box_layout.border.top + inline.block_offset,
    );

    let first = lines.len();
    for line in layout.lines() {
        let range = line.text_range();
        let Some(text) = inline.text.get(range.clone()) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let mut cells: Vec<TextCell> = line
            .runs()
            .flat_map(|run| run.clusters().collect::<Vec<_>>())
            .filter_map(|cluster| {
                let cluster_range = cluster.text_range();
                if cluster_range.start < range.start || cluster_range.end > range.end {
                    return None;
                }
                let start = cluster_range.start - range.start;
                let end = cluster_range.end - range.start;
                if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                    return None;
                }
                Some(TextCell {
                    start,
                    end,
                    x: (origin.x + cluster.visual_offset()? / scale) * zoom,
                    width: cluster.advance() / scale * zoom,
                })
            })
            .collect();
        if cells.is_empty() {
            continue;
        }
        cells.sort_by_key(|cell| cell.start);
        let metrics = line.metrics();
        lines.push(TextLine {
            block,
            y: (origin.y + metrics.block_min_coord / scale) * zoom,
            height: (metrics.block_max_coord - metrics.block_min_coord) / scale * zoom,
            text: text.to_string(),
            cells,
        });
    }
    lines.len() > first
}
