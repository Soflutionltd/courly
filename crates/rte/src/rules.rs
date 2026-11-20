//! Heuristic rules engine: chain of responsibility for insert/delete/format behavior.
//!
//! Each rule is a function `(doc, selection, action, pending_style) → Option<Vec<EditOp>>`.
//! Rules are tried in priority order; first `Some` wins. The top-level `resolve()`
//! function dispatches to per-action resolvers.
//!
//! This is where most of the editor's user-facing behavior lives. Getting these
//! rules right is more important than getting the data structures right - users
//! notice when Enter doesn't do what they expect.

use crate::document::{
    Block, BlockAttrs, BlockKind, DocPosition, DocSelection, Document, FontFamily, FormatChange,
    HtmlAttrs, InlineStyle, LINE_HEIGHT_RANGE, MAX_INDENT_LEVEL, Rgb, RunFormat, StyledRun, Table,
    TableChange, TextAlignment, block_with_runs_like, isolate_runs, text_len,
};
use crate::operations::{DeletedContent, EditOp};
use std::sync::Arc;

// ── Public API ──────────────────────────────────────────

/// A high-level user editing action (before rule resolution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    /// Insert a string at the current position.
    InsertText(String),
    /// Delete backward (Backspace).
    DeleteBackward,
    /// Delete forward (Delete key).
    DeleteForward,
    /// Delete the current selection.
    DeleteSelection,
    /// Split the current block (Enter).
    SplitBlock,
    /// Toggle an inline style.
    ToggleInlineStyle(InlineStyle),
    /// Set the block type at the cursor position (toolbar action).
    SetBlockType(BlockKind),
    /// Set the color, font or size of the selection (toolbar action).
    SetFormat(FormatChange),
    /// Strip bold, italic, underline, strikethrough, color, highlight, font
    /// and size from the selection (toolbar action). Links are kept.
    ClearFormatting,
    /// Align every paragraph and heading the selection touches.
    SetAlignment(TextAlignment),
    /// Set the line height, in percent of the font size, of every paragraph
    /// and heading the selection touches. `None` is the editor's own.
    SetLineHeight(Option<u16>),
    /// Indent every paragraph, heading and list item the selection touches
    /// one level deeper.
    IncreaseIndent,
    /// The reverse of [`EditAction::IncreaseIndent`].
    DecreaseIndent,
    /// Put an image in the document: in place of the image that is
    /// selected, after the block holding the caret otherwise.
    InsertImage {
        src: String,
        alt: String,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// Give the image at `block_index` a size in pixels.
    ResizeImage {
        block_index: usize,
        width: u32,
        height: u32,
    },
    /// Put an empty table of `rows` by `columns` cells after the block
    /// holding the caret.
    InsertTable { rows: usize, columns: usize },
    /// Change the rows or columns of the table whose cell holds the caret.
    /// The editor state carries it out, on the blocks around that table.
    Table(TableChange),
}

/// The most rows or columns a table can be inserted with.
pub const MAX_NEW_TABLE_SIDE: usize = 20;

/// An empty table as the editor inserts it: as wide as the text, thin
/// lines around its cells.
fn new_table(rows: usize, columns: usize) -> Table {
    let attrs = |pairs: &[(&str, &str)]| {
        HtmlAttrs::new(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        )
    };
    let mut table = Table::grid(
        rows.clamp(1, MAX_NEW_TABLE_SIDE),
        columns.clamp(1, MAX_NEW_TABLE_SIDE),
    );
    table.attrs = attrs(&[
        ("cellspacing", "0"),
        ("cellpadding", "0"),
        ("style", "border-collapse:collapse;width:100%"),
    ]);
    for row in &mut table.rows {
        for cell in &mut row.cells {
            cell.attrs = attrs(&[(
                "style",
                "border:1px solid #c8c8c8;padding:6px;vertical-align:top",
            )]);
        }
    }
    table
}

/// Color, font and size chosen at a collapsed caret, waiting for the text
/// typed next. Each attribute is `None` until chosen; a chosen attribute may
/// itself be `None`, for automatic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingFormat {
    color: Option<Option<Rgb>>,
    background: Option<Option<Rgb>>,
    font: Option<Option<FontFamily>>,
    size: Option<Option<u16>>,
}

impl PendingFormat {
    /// Whether nothing was chosen.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Record a choice, replacing an earlier one for the same attribute.
    pub fn set(&mut self, change: FormatChange) {
        match change {
            FormatChange::Color(color) => self.color = Some(color),
            FormatChange::Background(background) => self.background = Some(background),
            FormatChange::Font(font) => self.font = Some(font),
            FormatChange::Size(size) => self.size = Some(size),
        }
    }

    /// `format` with the chosen attributes laid over it.
    #[must_use]
    pub fn over(self, mut format: RunFormat) -> RunFormat {
        if let Some(color) = self.color {
            format.color = color;
        }
        if let Some(background) = self.background {
            format.background = background;
        }
        if let Some(font) = self.font {
            format.font = font;
        }
        if let Some(size) = self.size {
            format = format.with(FormatChange::Size(size));
        }
        format
    }
}

/// Resolve a high-level user action into concrete `EditOp`s using the rules chain.
///
/// The caller is responsible for applying the returned ops to the document and
/// running normalization afterward.
///
/// For `ToggleInlineStyle` at a collapsed caret, returns an empty `Vec` to signal
/// "toggle pending style" - the caller (editor state) handles pending style.
pub fn resolve(
    doc: &Document,
    selection: DocSelection,
    action: EditAction,
    pending_style: InlineStyle,
) -> Vec<EditOp> {
    match action {
        EditAction::InsertText(text) => resolve_insert(doc, selection, &text, pending_style),
        EditAction::DeleteBackward => resolve_delete_backward(doc, selection),
        EditAction::DeleteForward => resolve_delete_forward(doc, selection),
        EditAction::DeleteSelection => resolve_delete_selection(doc, selection),
        EditAction::SplitBlock => resolve_split_block(doc, selection),
        EditAction::ToggleInlineStyle(style) => resolve_toggle_style(doc, selection, style),
        EditAction::SetBlockType(kind) => resolve_set_block_type(doc, selection, kind),
        EditAction::SetFormat(change) => {
            // A collapsed caret has no text to format: the empty result
            // tells the editor state to keep the choice for the next typing.
            if selection.is_collapsed() {
                Vec::new()
            } else {
                format_range_ops(doc, selection.start(), selection.end(), |format| {
                    format.with(change)
                })
            }
        }
        EditAction::ClearFormatting => {
            // An image is content, not formatting: it stays.
            rewrite_range_ops(doc, selection.start(), selection.end(), |run| {
                run.style = InlineStyle::empty();
                run.format = RunFormat {
                    image: run.format.image,
                    ..RunFormat::default()
                };
            })
        }
        EditAction::SetAlignment(alignment) => block_attrs_ops(doc, selection, |block, attrs| {
            if !matches!(block, Block::ListItem { .. }) {
                attrs.alignment = alignment;
            }
        }),
        EditAction::SetLineHeight(line_height) => {
            block_attrs_ops(doc, selection, |block, attrs| {
                if !matches!(block, Block::ListItem { .. }) {
                    attrs.line_height = line_height.map(|percent| {
                        percent.clamp(*LINE_HEIGHT_RANGE.start(), *LINE_HEIGHT_RANGE.end())
                    });
                }
            })
        }
        EditAction::IncreaseIndent => block_attrs_ops(doc, selection, |_, attrs| {
            attrs.indent_level = attrs.indent_level.saturating_add(1).min(MAX_INDENT_LEVEL);
        }),
        EditAction::DecreaseIndent => block_attrs_ops(doc, selection, |_, attrs| {
            attrs.indent_level = attrs.indent_level.saturating_sub(1);
        }),
        EditAction::InsertImage {
            src,
            alt,
            width,
            height,
        } => resolve_insert_image(
            doc,
            selection,
            Block::Image {
                src,
                alt,
                width,
                height,
            },
        ),
        EditAction::InsertTable { rows, columns } => insert_block_ops(
            doc,
            selection,
            Block::Table(Arc::new(new_table(rows, columns))),
        ),
        EditAction::Table(_) => Vec::new(),
        EditAction::ResizeImage {
            block_index,
            width,
            height,
        } => resolve_resize_image(doc, block_index, width, height),
    }
}

/// The index of the image block a selection covers exactly, if it does.
pub fn selected_image(doc: &Document, selection: DocSelection) -> Option<usize> {
    let start = selection.start();
    let end = selection.end();
    let covers_one_block =
        start.block_index == end.block_index && start.offset == 0 && end.offset == 1;
    (covers_one_block && matches!(doc.block(start.block_index), Some(Block::Image { .. })))
        .then_some(start.block_index)
}

/// A selected image is replaced. Otherwise the image goes after the block
/// holding the end of the selection.
fn resolve_insert_image(doc: &Document, selection: DocSelection, image: Block) -> Vec<EditOp> {
    if let Some(index) = selected_image(doc, selection)
        && let Some(old) = doc.block(index)
    {
        return vec![EditOp::ReplaceBlock {
            index,
            old: old.clone(),
            new: image,
        }];
    }
    insert_block_ops(doc, selection, image)
}

/// Put `block` after the block holding the end of the selection, followed
/// by an empty paragraph when it would be the last block, so that typing
/// can go on under it.
fn insert_block_ops(doc: &Document, selection: DocSelection, block: Block) -> Vec<EditOp> {
    let index = (selection.end().block_index + 1).min(doc.block_count());
    let mut ops = vec![EditOp::InsertBlock { index, block }];
    if index == doc.block_count() {
        ops.push(EditOp::InsertBlock {
            index: index + 1,
            block: Block::empty_paragraph(),
        });
    }
    ops
}

fn resolve_resize_image(
    doc: &Document,
    block_index: usize,
    width: u32,
    height: u32,
) -> Vec<EditOp> {
    let Some(old @ Block::Image { src, alt, .. }) = doc.block(block_index) else {
        return Vec::new();
    };
    let new = Block::Image {
        src: src.clone(),
        alt: alt.clone(),
        width: Some(width),
        height: Some(height),
    };
    if new == *old {
        return Vec::new();
    }
    vec![EditOp::ReplaceBlock {
        index: block_index,
        old: old.clone(),
        new,
    }]
}

/// Ops that pass the attributes of every paragraph, heading and list item
/// the selection touches through `change`. A list item holds an indent
/// only, so `change` is told which block it is looking at.
fn block_attrs_ops(
    doc: &Document,
    selection: DocSelection,
    change: impl Fn(&Block, &mut BlockAttrs),
) -> Vec<EditOp> {
    let mut ops = Vec::new();
    for block_index in selection.start().block_index..=selection.end().block_index {
        let Some(block) = doc
            .block(block_index)
            .filter(|block| block.is_inline_block())
        else {
            continue;
        };
        let old = block.attrs();
        let mut new = old;
        change(block, &mut new);
        if new != old {
            ops.push(EditOp::SetBlockAttrs {
                block_index,
                old,
                new,
            });
        }
    }
    ops
}

/// Ops that rewrite the format of every run in `[start..end)` through
/// `change`.
pub fn format_range_ops(
    doc: &Document,
    start: DocPosition,
    end: DocPosition,
    change: impl Fn(RunFormat) -> RunFormat,
) -> Vec<EditOp> {
    rewrite_range_ops(doc, start, end, |run| run.format = change(run.format))
}

/// Ops that pass every run in `[start..end)` through `rewrite`, which may
/// change a run's style and format but not its text. Each touched block is
/// replaced whole, which carries its own undo and leaves positions where
/// they are.
fn rewrite_range_ops(
    doc: &Document,
    start: DocPosition,
    end: DocPosition,
    rewrite: impl Fn(&mut StyledRun),
) -> Vec<EditOp> {
    let mut ops = Vec::new();
    for index in start.block_index..=end.block_index {
        let Some(block) = doc.block(index).filter(|block| block.is_inline_block()) else {
            continue;
        };
        let block_start = if index == start.block_index {
            start.offset
        } else {
            0
        };
        let block_end = if index == end.block_index {
            end.offset
        } else {
            block.char_len()
        };
        if block_start >= block_end {
            continue;
        }

        let mut new = block.clone();
        let Some(runs) = new.runs_mut() else {
            continue;
        };
        let range = isolate_runs(runs, block_start, block_end);
        let mut changed = false;
        for run in &mut runs[range] {
            let before = (run.style, run.format);
            rewrite(run);
            changed |= before != (run.style, run.format);
        }
        if changed {
            ops.push(EditOp::ReplaceBlock {
                index,
                old: block.clone(),
                new,
            });
        }
    }
    ops
}

// ── Insert ──────────────────────────────────────────────

/// Resolve an insert-text action.
///
/// Rules (in priority order):
/// 1. If selection is non-collapsed, delete the selection first, then insert at start.
/// 2. Use pending style if non-empty; otherwise inherit style from cursor position.
/// 3. Link boundary exclusivity: typing at the edge of a link does NOT extend the link.
fn resolve_insert(
    doc: &Document,
    selection: DocSelection,
    text: &str,
    pending_style: InlineStyle,
) -> Vec<EditOp> {
    let mut ops = Vec::new();

    // Rule: insert replaces selection. The style decisions must account for
    // the deletion: the desired style comes from the first selected character
    // (the text being replaced), while the inherited style must be predicted
    // against the post-delete document, because the deletion can remove the
    // very runs the insertion offset would otherwise inherit from.
    let scratch: Document;
    let (target_doc, insert_pos, style) = if selection.is_collapsed() {
        let pos = selection.focus;
        let style = if pending_style.is_empty() {
            resolve_style_at(doc, pos)
        } else {
            pending_style
        };
        (doc, pos, style)
    } else {
        let pos = post_delete_position(doc, selection);
        let style = if pending_style.is_empty() {
            style_at_position(doc, pos)
        } else {
            pending_style
        };
        let delete_ops = build_delete_selection(doc, selection);
        let mut post_delete = doc.clone();
        for op in &delete_ops {
            op.apply(&mut post_delete);
        }
        ops.extend(delete_ops);
        scratch = post_delete;
        (&scratch, pos, style)
    };

    // The InsertText op inserts text into the run at the insertion point,
    // inheriting that run's style. If the desired style differs (e.g., pending
    // bold into plain text), we emit ToggleInlineStyle ops after the insert to
    // fix up the style bits on the newly inserted range.
    let run_style = run_style_at(target_doc, insert_pos);

    ops.push(EditOp::InsertText {
        position: insert_pos,
        text: text.to_owned(),
    });

    // Emit ToggleInlineStyle for each bit that differs between the desired
    // style and the inherited run style. The toggle range covers exactly
    // the inserted text.
    let diff = style.symmetric_difference(run_style);
    if !diff.is_empty() {
        let char_count = text_len(text);
        let insert_end = DocPosition::new(insert_pos.block_index, insert_pos.offset + char_count);

        for bit in diff.iter() {
            ops.push(EditOp::ToggleInlineStyle {
                start: insert_pos,
                end: insert_end,
                style_bit: bit,
            });
        }
    }

    ops
}

/// Resolve the inline style at a cursor position, with link boundary exclusivity.
///
/// - If the cursor is strictly inside a run, use that run's style.
/// - If the cursor is at a run boundary, use the style of the run to the left
///   (the "preceding character" heuristic - standard across all editors).
/// - Link boundary exclusivity: if the cursor is at the start or end of a link run,
///   do NOT inherit the link's style. Only typing strictly inside a link extends it.
///   (This function only returns InlineStyle, not link info, so link exclusivity is
///   handled separately by the caller.)
fn resolve_style_at(doc: &Document, pos: DocPosition) -> InlineStyle {
    let Some(block) = doc.block(pos.block_index) else {
        return InlineStyle::empty();
    };
    let Some(runs) = block.runs() else {
        return InlineStyle::empty();
    };

    if runs.is_empty() {
        return InlineStyle::empty();
    }

    // Find which run the cursor is in.
    let Some((run_idx, offset_in_run)) = block.resolve_offset(pos.offset) else {
        // Past end - use last run's style.
        return runs.last().map_or(InlineStyle::empty(), |r| r.style);
    };

    let run = &runs[run_idx];

    // If cursor is at the start of a run and there's a previous run, prefer the
    // previous run's style (the "left affinity" / preceding character heuristic).
    if offset_in_run == 0 && run_idx > 0 {
        let prev_run = &runs[run_idx - 1];
        // Link boundary exclusivity: if the previous run is a link and we're at its
        // end (i.e., the cursor is right after the link), don't inherit the link style.
        // For InlineStyle (non-link formatting), always inherit from the left.
        if prev_run.link.is_some() {
            // At the end of a link - use the current run's style (the one after the link).
            return run.style;
        }
        return prev_run.style;
    }

    // Link boundary exclusivity: if at the start of a link run (offset_in_run == 0),
    // don't inherit the link's style bits. But we already handled offset_in_run == 0
    // above (either run_idx == 0 or we used the previous run). So if we're here with
    // offset_in_run == 0 and run_idx == 0, we're at the very start of the block.
    // If this first run is a link, the cursor is at the link boundary - don't inherit link.
    // For InlineStyle bits (bold/italic/etc), still inherit them.

    run.style
}

/// Style of the character at `pos` (right affinity). Used when replacing a
/// selection: the typed text takes on the style of the first character it
/// replaces, matching mainstream editor behavior. Falls back to the last
/// run's style past the end of the block.
fn style_at_position(doc: &Document, pos: DocPosition) -> InlineStyle {
    let Some(runs) = doc.block(pos.block_index).and_then(Block::runs) else {
        return InlineStyle::empty();
    };
    let mut char_pos = 0;
    for run in runs {
        let run_len = run.char_len();
        if pos.offset < char_pos + run_len {
            return run.style;
        }
        char_pos += run_len;
    }
    runs.last().map_or(InlineStyle::empty(), |r| r.style)
}

/// The caret position after deleting `selection`. Normally the selection
/// start; when the selection starts at the trailing edge of an atomic block
/// (offset past the atom), the atom itself is not part of the selection, so
/// the deletion - and the caret - begin at the start of the next block.
pub(crate) fn post_delete_position(doc: &Document, selection: DocSelection) -> DocPosition {
    let start = selection.start();
    if start.offset > 0
        && start.block_index + 1 < doc.block_count()
        && doc
            .block(start.block_index)
            .is_some_and(Block::is_atomic_block)
    {
        return DocPosition::new(start.block_index + 1, 0);
    }
    start
}

/// Get the style of the run at a given position - the style that `InsertText`
/// will inherit when inserting at this offset.
///
/// This mirrors the logic in `insert_text_into_runs` (operations.rs): the text
/// is inserted into whichever run contains the offset, so the inherited style
/// is that run's style.
fn run_style_at(doc: &Document, pos: DocPosition) -> InlineStyle {
    let Some(block) = doc.block(pos.block_index) else {
        return InlineStyle::empty();
    };
    let Some(runs) = block.runs() else {
        return InlineStyle::empty();
    };

    if runs.is_empty() {
        return InlineStyle::empty();
    }

    // Walk runs the same way insert_text_into_runs does.
    let mut char_pos = 0;
    for run in runs {
        let run_len = run.char_len();
        if pos.offset >= char_pos && pos.offset <= char_pos + run_len {
            return run.style;
        }
        char_pos += run_len;
    }

    // Past the end - last run's style.
    runs.last().map_or(InlineStyle::empty(), |r| r.style)
}

// ── Delete backward (Backspace) ─────────────────────────

/// Resolve a backspace action.
///
/// Rules (in priority order):
/// 1. If selection is non-collapsed, delete the selection.
/// 2. At offset 0 of block 0: no-op.
/// 3. At offset 0 of any other block: merge with previous block.
/// 4. Otherwise: delete the character before the cursor.
fn resolve_delete_backward(doc: &Document, selection: DocSelection) -> Vec<EditOp> {
    // Rule 1: delete selection if non-collapsed.
    if !selection.is_collapsed() {
        return build_delete_selection(doc, selection);
    }

    let pos = selection.focus;

    // Rule 2: at the very start of the document, no-op.
    if pos.block_index == 0 && pos.offset == 0 {
        return vec![];
    }

    // Rule 3: at offset 0 of a non-first block - handle image blocks.
    if pos.offset == 0 {
        // If the previous block is atomic, delete (remove) it instead of merging.
        if let Some(prev) = doc.block(pos.block_index - 1)
            && prev.is_atomic_block()
        {
            return vec![EditOp::RemoveBlock {
                index: pos.block_index - 1,
                saved: prev.clone(),
            }];
        }
        return resolve_merge_backward(doc, pos.block_index);
    }

    // Rule 4: delete the character before the cursor.
    if block_is_atomic_at(doc, pos.block_index) {
        return vec![remove_or_clear_block(doc, pos.block_index)];
    }
    resolve_delete_char_backward(doc, pos)
}

/// Merge the block at `block_index` into the previous block.
fn resolve_merge_backward(doc: &Document, block_index: usize) -> Vec<EditOp> {
    if block_index == 0 {
        return vec![];
    }

    let Some(current_block) = doc.block(block_index) else {
        return vec![];
    };
    let Some(prev_block) = doc.block(block_index - 1) else {
        return vec![];
    };

    // Only merge inline blocks. If either is a container/HR, skip (no-op for now).
    if !current_block.is_inline_block() || !prev_block.is_inline_block() {
        return vec![];
    }

    let merge_offset = prev_block.char_len();

    vec![EditOp::MergeBlocks {
        block_index,
        saved: current_block.clone(),
        merge_offset,
    }]
}

/// Delete a single character before the cursor (within the same block).
fn resolve_delete_char_backward(doc: &Document, pos: DocPosition) -> Vec<EditOp> {
    let Some(block) = doc.block(pos.block_index) else {
        return vec![];
    };
    let Some(runs) = block.runs() else {
        return vec![];
    };

    let char_before_offset = pos.offset.saturating_sub(1);
    if char_before_offset >= block.char_len() {
        return vec![];
    }

    // Build deleted content: extract the runs covering the deleted character.
    let deleted_runs = extract_deleted_runs(runs, char_before_offset, pos.offset);

    let start = DocPosition::new(pos.block_index, char_before_offset);
    let end = pos;

    vec![EditOp::DeleteRange {
        start,
        end,
        deleted: DeletedContent {
            blocks: vec![block_with_runs_like(block, deleted_runs)],
            end_tail: None,
        },
    }]
}

// ── Delete forward (Delete key) ─────────────────────────

/// Resolve a delete-forward action.
///
/// Rules (in priority order):
/// 1. If selection is non-collapsed, delete the selection.
/// 2. At the end of the last block: no-op.
/// 3. At the end of any block: merge with next block.
/// 4. Otherwise: delete the character after the cursor.
fn resolve_delete_forward(doc: &Document, selection: DocSelection) -> Vec<EditOp> {
    // Rule 1: delete selection if non-collapsed.
    if !selection.is_collapsed() {
        return build_delete_selection(doc, selection);
    }

    let pos = selection.focus;

    let Some(block) = doc.block(pos.block_index) else {
        return vec![];
    };

    let at_block_end = pos.offset >= block.char_len();
    let is_last_block = pos.block_index >= doc.block_count().saturating_sub(1);

    // Rule 2: at the end of the last block, no-op.
    if at_block_end && is_last_block {
        return vec![];
    }

    // Rule 3: at the end of a non-last block - handle image blocks.
    if at_block_end {
        // If the next block is atomic, delete (remove) it instead of merging.
        let next_idx = pos.block_index + 1;
        if let Some(next) = doc.block(next_idx)
            && next.is_atomic_block()
        {
            return vec![EditOp::RemoveBlock {
                index: next_idx,
                saved: next.clone(),
            }];
        }
        return resolve_merge_forward(doc, pos.block_index);
    }

    // Rule 4: delete the character after the cursor.
    if block_is_atomic_at(doc, pos.block_index) && pos.offset == 0 {
        return vec![remove_or_clear_block(doc, pos.block_index)];
    }
    resolve_delete_char_forward(doc, pos)
}

/// Merge the next block into the block at `block_index`.
fn resolve_merge_forward(doc: &Document, block_index: usize) -> Vec<EditOp> {
    let next_index = block_index + 1;
    if next_index >= doc.block_count() {
        return vec![];
    }

    let Some(next_block) = doc.block(next_index) else {
        return vec![];
    };
    let Some(current_block) = doc.block(block_index) else {
        return vec![];
    };

    if !next_block.is_inline_block() || !current_block.is_inline_block() {
        return vec![];
    }

    let merge_offset = current_block.char_len();

    vec![EditOp::MergeBlocks {
        block_index: next_index,
        saved: next_block.clone(),
        merge_offset,
    }]
}

/// Delete a single character after the cursor (within the same block).
fn resolve_delete_char_forward(doc: &Document, pos: DocPosition) -> Vec<EditOp> {
    let Some(block) = doc.block(pos.block_index) else {
        return vec![];
    };
    let Some(runs) = block.runs() else {
        return vec![];
    };

    if pos.offset >= block.char_len() {
        return vec![];
    }

    let end_offset = pos.offset + 1;

    let deleted_runs = extract_deleted_runs(runs, pos.offset, end_offset);

    vec![EditOp::DeleteRange {
        start: pos,
        end: DocPosition::new(pos.block_index, end_offset),
        deleted: DeletedContent {
            blocks: vec![block_with_runs_like(block, deleted_runs)],
            end_tail: None,
        },
    }]
}

// ── Delete selection ────────────────────────────────────

/// Resolve an explicit delete-selection action.
fn resolve_delete_selection(doc: &Document, selection: DocSelection) -> Vec<EditOp> {
    if selection.is_collapsed() {
        return vec![];
    }
    build_delete_selection(doc, selection)
}

/// Build the EditOps to delete a non-collapsed selection.
fn build_delete_selection(doc: &Document, selection: DocSelection) -> Vec<EditOp> {
    // A selection anchored at the trailing edge of an atomic block does not
    // include the atom; start the deletion at the next block so the atom
    // survives (and so undo doesn't have to reconstruct a block it never
    // captured).
    let start = post_delete_position(doc, selection);
    let end = selection.end();

    if start >= end {
        return vec![];
    }

    // Build the DeletedContent by extracting from the document.
    let deleted = build_deleted_content(doc, start, end);

    vec![EditOp::DeleteRange {
        start,
        end,
        deleted,
    }]
}

/// Build `DeletedContent` capturing the content in `[start..end)`.
fn build_deleted_content(doc: &Document, start: DocPosition, end: DocPosition) -> DeletedContent {
    if start.block_index == end.block_index {
        // Single-block deletion.
        let block = match doc.block(start.block_index) {
            Some(b) => b,
            None => {
                return DeletedContent {
                    blocks: vec![Block::empty_paragraph()],
                    end_tail: None,
                };
            }
        };

        if let Some(runs) = block.runs() {
            let deleted_runs = extract_deleted_runs(runs, start.offset, end.offset);

            DeletedContent {
                blocks: vec![block_with_runs_like(block, deleted_runs)],
                end_tail: None,
            }
        } else if start.offset == 0 && end.offset >= block.char_len() {
            DeletedContent {
                blocks: vec![block.clone()],
                end_tail: None,
            }
        } else {
            DeletedContent {
                blocks: vec![Block::empty_paragraph()],
                end_tail: None,
            }
        }
    } else {
        // Cross-block deletion.
        let mut blocks = Vec::new();
        let mut end_tail = None;

        // Tail of start block (from start.offset to end of block).
        if let Some(start_block) = doc.block(start.block_index) {
            if let Some(runs) = start_block.runs() {
                let tail_runs = extract_deleted_runs(runs, start.offset, start_block.char_len());
                blocks.push(block_with_runs_like(start_block, tail_runs));
            } else {
                // Non-inline block (Image, HR, BlockQuote) - capture whole block.
                if start.offset == 0 {
                    blocks.push(start_block.clone());
                }
            }
        }

        // Middle blocks (fully deleted).
        for i in (start.block_index + 1)..end.block_index {
            if let Some(block) = doc.block(i) {
                blocks.push(block.clone());
            }
        }

        // Head of end block (from 0 to end.offset).
        if let Some(end_block) = doc.block(end.block_index) {
            if let Some(runs) = end_block.runs() {
                let head_runs = extract_deleted_runs(runs, 0, end.offset);
                blocks.push(block_with_runs_like(end_block, head_runs));
                let tail_runs = extract_deleted_runs(runs, end.offset, end_block.char_len());
                if !runs_are_effectively_empty(&tail_runs) {
                    end_tail = Some(block_with_runs_like(end_block, tail_runs));
                }
            } else {
                // Non-inline block - capture whole block.
                if end.offset > 0 {
                    blocks.push(end_block.clone());
                }
            }
        }

        if blocks.is_empty() {
            blocks.push(Block::empty_paragraph());
        }
        DeletedContent { blocks, end_tail }
    }
}

// ── Split block (Enter) ────────────────────────────────

/// Resolve a split-block action.
///
/// Rules (in priority order):
/// 1. If selection is non-collapsed, delete the selection first.
/// 2. Auto-exit block: pressing Enter on an empty block at the end of a list/blockquote
///    exits the block (converts it to a plain paragraph).
/// 3. Heading reset on split at end: splitting at the END of a heading creates the
///    heading + a new paragraph (not a second heading).
/// 4. Normal split: split the block, preserving block type.
fn resolve_split_block(doc: &Document, selection: DocSelection) -> Vec<EditOp> {
    let mut ops = Vec::new();

    // Rule 1: delete selection first.
    let split_pos = if !selection.is_collapsed() {
        let pos = post_delete_position(doc, selection);
        let delete_ops = build_delete_selection(doc, selection);
        ops.extend(delete_ops);
        pos
    } else {
        selection.focus
    };

    let Some(block) = doc.block(split_pos.block_index) else {
        return ops;
    };

    // Rule: Image blocks are atomic - Enter inserts a new paragraph after.
    if matches!(block, Block::Image { .. }) {
        ops.push(EditOp::InsertBlock {
            index: split_pos.block_index + 1,
            block: Block::empty_paragraph(),
        });
        return ops;
    }

    // Rule 2: auto-exit list item. Enter on an empty ListItem converts it
    // to a plain Paragraph (the "double-Enter to exit list" UX pattern).
    if let Block::ListItem { runs, .. } = block {
        let is_empty = runs.iter().all(|r| r.text.trim().is_empty());
        if is_empty {
            ops.push(EditOp::SetBlockType {
                block_index: split_pos.block_index,
                old: block.kind(),
                new: BlockKind::Paragraph,
            });
            return ops;
        }
    }

    // Rule 3: heading reset on split at end.
    if let Block::Heading { level, .. } = block
        && split_pos.offset >= block.char_len()
    {
        // Splitting at the end of a heading: the heading stays, new block is paragraph.
        // We do this by splitting (which creates two headings) and then changing the
        // second heading to a paragraph.
        ops.push(EditOp::SplitBlock {
            position: split_pos,
        });
        ops.push(EditOp::SetBlockType {
            block_index: split_pos.block_index + 1,
            old: BlockKind::Heading(*level),
            new: BlockKind::Paragraph,
        });
        return ops;
    }

    // Rule 4: normal split, preserving block type.
    ops.push(EditOp::SplitBlock {
        position: split_pos,
    });
    ops
}

// ── Toggle inline style ─────────────────────────────────

/// Resolve a toggle-style action.
///
/// Rules:
/// 1. With selection: produce `ToggleInlineStyle` op.
/// 2. Without selection (collapsed caret) inside a link: format the whole contiguous link.
/// 3. Without selection, not inside a link: return empty Vec to signal "toggle pending style."
fn resolve_toggle_style(
    doc: &Document,
    selection: DocSelection,
    style: InlineStyle,
) -> Vec<EditOp> {
    if selection.is_collapsed() {
        // Rule 2: if cursor is inside a link, format the whole link span.
        if let Some((link_start, link_end)) = find_link_boundaries(doc, selection.focus) {
            return vec![EditOp::ToggleInlineStyle {
                start: link_start,
                end: link_end,
                style_bit: style,
            }];
        }

        // Rule 3: empty Vec signals "toggle pending style" to the caller.
        return vec![];
    }

    let start = selection.start();
    let end = selection.end();

    // Don't produce an op if start == end (shouldn't happen since we checked is_collapsed,
    // but be safe).
    if start == end {
        return vec![];
    }

    vec![EditOp::ToggleInlineStyle {
        start,
        end,
        style_bit: style,
    }]
}

// ── Set block type (toolbar) ─────────────────────────────

/// Resolve a `SetBlockType` action from the toolbar.
///
/// If the cursor is inside a block of a different type, emits a `SetBlockType`
/// op. If the block is already the target type, toggles back to Paragraph.
fn resolve_set_block_type(doc: &Document, selection: DocSelection, kind: BlockKind) -> Vec<EditOp> {
    let block_index = selection.focus.block_index;
    let Some(block) = doc.block(block_index) else {
        return vec![];
    };

    let current_kind = block.kind();

    // Toggle: if already the target kind, revert to Paragraph.
    let new_kind = if current_kind == kind {
        BlockKind::Paragraph
    } else {
        kind
    };

    // Don't emit a no-op.
    if current_kind == new_kind {
        return vec![];
    }

    // Only allow conversion of inline blocks (Paragraph, Heading, ListItem).
    // Don't convert HorizontalRule, Image, or BlockQuote content blocks.
    if matches!(
        current_kind,
        BlockKind::HorizontalRule | BlockKind::Image | BlockKind::Table
    ) {
        return vec![];
    }

    // Don't allow creating block types that have no addressable inline runs
    // (HR, Image, BlockQuote). The editor's flat `DocPosition` can't reach
    // content inside a BlockQuote, so wrapping a paragraph in one would
    // immediately strand the cursor on an unwritable block.
    if matches!(
        new_kind,
        BlockKind::HorizontalRule | BlockKind::Image | BlockKind::BlockQuote | BlockKind::Table
    ) {
        return vec![];
    }

    let set_type = EditOp::SetBlockType {
        block_index,
        old: current_kind,
        new: new_kind,
    };

    // A paragraph and a heading hand their attributes to each other. A list
    // item cannot hold them, so they are reset first, as an op of its own:
    // undoing the pair then brings them back.
    let attrs = block.attrs();
    let becomes_list_item = matches!(new_kind, BlockKind::ListItem { .. });
    let is_list_item = matches!(current_kind, BlockKind::ListItem { .. });
    if becomes_list_item && !is_list_item && !attrs.is_plain() {
        return vec![
            EditOp::SetBlockAttrs {
                block_index,
                old: attrs,
                new: BlockAttrs::default(),
            },
            set_type,
        ];
    }
    vec![set_type]
}

// ── Helpers ─────────────────────────────────────────────

/// Find the boundaries of the contiguous link span containing `pos`.
///
/// If the cursor is inside a run that has a link, this walks backward and forward
/// through adjacent runs with the same link href to find the full contiguous span.
/// Returns `Some((start, end))` as `DocPosition`s, or `None` if the cursor is not
/// inside a link.
fn find_link_boundaries(doc: &Document, pos: DocPosition) -> Option<(DocPosition, DocPosition)> {
    let block = doc.block(pos.block_index)?;
    let runs = block.runs()?;

    if runs.is_empty() {
        return None;
    }

    let (run_idx, offset_in_run) = block.resolve_offset(pos.offset)?;
    let run = &runs[run_idx];

    // If we're at a run boundary (offset_in_run == 0) and the current run
    // has no link, check the previous run. resolve_offset advances to the
    // next run at internal boundaries, so a caret at the end of a link
    // followed by plain text resolves to the plain run. We want to still
    // recognize the caret as "at the link boundary" for formatting purposes.
    let href = match run.link.as_ref() {
        Some(h) => h,
        None if offset_in_run == 0 && run_idx > 0 => runs[run_idx - 1].link.as_ref()?,
        None => return None,
    };

    // If we fell back to the previous run, adjust run_idx for the walk.
    let anchor_run_idx = if run.link.as_ref() == Some(href) {
        run_idx
    } else {
        run_idx - 1
    };

    // Walk backward to find the first run with the same link href.
    let mut start_run_idx = anchor_run_idx;
    while start_run_idx > 0 {
        if runs[start_run_idx - 1].link.as_ref() == Some(href) {
            start_run_idx -= 1;
        } else {
            break;
        }
    }

    // Walk forward to find the last run with the same link href.
    let mut end_run_idx = anchor_run_idx;
    while end_run_idx + 1 < runs.len() {
        if runs[end_run_idx + 1].link.as_ref() == Some(href) {
            end_run_idx += 1;
        } else {
            break;
        }
    }

    // Compute the start offset: sum of char_len for all runs before start_run_idx.
    let start_offset: usize = runs[..start_run_idx].iter().map(StyledRun::char_len).sum();

    // Compute the end offset: start_offset + sum of char_len for runs in the link span.
    let end_offset: usize = start_offset
        + runs[start_run_idx..=end_run_idx]
            .iter()
            .map(StyledRun::char_len)
            .sum::<usize>();

    Some((
        DocPosition::new(pos.block_index, start_offset),
        DocPosition::new(pos.block_index, end_offset),
    ))
}

fn block_is_atomic_at(doc: &Document, block_index: usize) -> bool {
    doc.block(block_index).is_some_and(Block::is_atomic_block)
}

fn remove_or_clear_block(doc: &Document, block_index: usize) -> EditOp {
    let block = doc
        .block(block_index)
        .cloned()
        .unwrap_or_else(Block::empty_paragraph);
    if doc.block_count() <= 1 {
        EditOp::ReplaceBlock {
            index: block_index,
            old: block,
            new: Block::empty_paragraph(),
        }
    } else {
        EditOp::RemoveBlock {
            index: block_index,
            saved: block,
        }
    }
}

fn runs_are_effectively_empty(runs: &[StyledRun]) -> bool {
    runs.iter().all(StyledRun::is_empty)
}

/// Extract runs covering `[start_offset..end_offset)` from a run list.
/// Returns at least one run (possibly empty) for valid DeletedContent.
fn extract_deleted_runs(
    runs: &[StyledRun],
    start_offset: usize,
    end_offset: usize,
) -> Vec<StyledRun> {
    if start_offset >= end_offset {
        return vec![StyledRun::plain(String::new())];
    }

    let mut result = Vec::new();
    let mut pos = 0;

    for run in runs {
        let run_len = run.char_len();
        let run_start = pos;
        let run_end = pos + run_len;

        // Skip runs entirely before the range.
        if run_end <= start_offset {
            pos = run_end;
            continue;
        }
        // Stop at runs entirely after the range.
        if run_start >= end_offset {
            break;
        }

        // Compute the overlap.
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

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        Block, DocPosition, DocSelection, Document, HeadingLevel, InlineStyle, StyledRun,
    };
    use crate::operations::EditOp;

    /// Helper: apply a sequence of ops to a document.
    fn apply_ops(doc: &mut Document, ops: &[EditOp]) {
        for op in ops {
            op.apply(doc);
        }
    }

    /// Helper: get flattened text of a block.
    fn block_text(doc: &Document, idx: usize) -> String {
        doc.block(idx)
            .map_or_else(String::new, Block::flattened_text)
    }

    // ── Insert text ─────────────────────────────────────

    #[test]
    fn insert_text_at_start() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 0));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("X".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "Xhello");
    }

    #[test]
    fn insert_text_at_middle() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hllo")]);
        let sel = DocSelection::caret(DocPosition::new(0, 1));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("e".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn insert_text_at_end() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("!".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello!");
    }

    #[test]
    fn insert_text_into_styled_run() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("hllo", InlineStyle::BOLD)],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 1));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("e".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
    }

    #[test]
    fn insert_replacing_selection() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("!".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello!");
    }

    #[test]
    fn insert_replacing_cross_block_selection() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::range(DocPosition::new(0, 3), DocPosition::new(1, 2));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("X".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "helXrld");
    }

    #[test]
    fn insert_multichar() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("ac")]);
        let sel = DocSelection::caret(DocPosition::new(0, 1));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("bb".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "abbc");
    }

    // ── Backspace (delete backward) ─────────────────────

    #[test]
    fn backspace_at_document_start_is_noop() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        assert!(ops.is_empty());
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn backspace_within_block_deletes_char() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "helo");
    }

    #[test]
    fn backspace_at_end_of_block_deletes_last_char() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hi")]);
        let sel = DocSelection::caret(DocPosition::new(0, 2));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "h");
    }

    #[test]
    fn backspace_at_block_start_merges_with_previous() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::caret(DocPosition::new(1, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "helloworld");
    }

    #[test]
    fn backspace_merge_preserves_first_block_type() {
        let mut doc = Document::from_blocks(vec![
            Block::Heading {
                attrs: Default::default(),
                level: HeadingLevel::H1,
                runs: vec![StyledRun::plain("Title")],
            },
            Block::paragraph("rest"),
        ]);
        let sel = DocSelection::caret(DocPosition::new(1, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert_eq!(block_text(&doc, 0), "Titlerest");
    }

    #[test]
    fn backspace_with_selection_deletes_selection() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn backspace_captures_correct_deleted_content() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        assert_eq!(ops.len(), 1);
        if let EditOp::DeleteRange {
            start,
            end,
            deleted,
        } = &ops[0]
        {
            assert_eq!(start.offset, 2);
            assert_eq!(end.offset, 3);
            let deleted_text = deleted.blocks[0].flattened_text();
            assert_eq!(deleted_text, "l");
        } else {
            panic!("expected DeleteRange");
        }
    }

    #[test]
    fn backspace_captures_styled_deleted_content() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("ab"),
                StyledRun::styled("cd", InlineStyle::BOLD),
            ],
        }]);
        // Cursor at offset 3 (inside the bold run, after 'c').
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        assert_eq!(ops.len(), 1);
        if let EditOp::DeleteRange { deleted, .. } = &ops[0] {
            let deleted_runs = deleted.blocks[0].runs().expect("runs");
            assert_eq!(deleted_runs[0].text, "c");
            assert_eq!(deleted_runs[0].style, InlineStyle::BOLD);
        } else {
            panic!("expected DeleteRange");
        }
    }

    // ── Delete forward ──────────────────────────────────

    #[test]
    fn delete_forward_at_document_end_is_noop() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        assert!(ops.is_empty());
    }

    #[test]
    fn delete_forward_within_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 1));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hllo");
    }

    #[test]
    fn delete_forward_at_block_end_merges() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "helloworld");
    }

    #[test]
    fn delete_forward_at_last_block_end_is_noop() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::caret(DocPosition::new(1, 5));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        assert!(ops.is_empty());
    }

    #[test]
    fn delete_forward_with_selection_deletes_selection() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), " world");
    }

    // ── Image embed protection ──────────────────────────

    #[test]
    fn backspace_before_image_removes_image() {
        // Block 0: paragraph, Block 1: image.
        // Cursor at start of block 1 → backspace should remove the image.
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::Image {
                src: "img.png".into(),
                alt: String::new(),
                width: None,
                height: None,
            },
            Block::paragraph("world"),
        ]);
        // Cursor at block 2, offset 0 → previous block is Image.
        let sel = DocSelection::caret(DocPosition::new(2, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), "world");
    }

    #[test]
    fn delete_forward_after_image_removes_image() {
        // Block 0: paragraph, Block 1: image, Block 2: paragraph.
        // Cursor at end of block 0 → delete forward should remove the image.
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::Image {
                src: "img.png".into(),
                alt: String::new(),
                width: None,
                height: None,
            },
            Block::paragraph("world"),
        ]);
        // Cursor at end of block 0 → next block is Image.
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), "world");
    }

    // ── Delete selection ────────────────────────────────

    #[test]
    fn delete_from_atom_trailing_edge_spares_atom() {
        // Selection starts at (image, 1) - the trailing edge of the atom -
        // so the image is NOT part of the selection and must survive.
        let image = Block::Image {
            src: "cid:1".into(),
            alt: "logo".into(),
            width: None,
            height: None,
        };
        let mut doc = Document::from_blocks(vec![image, Block::paragraph("hello")]);
        let sel = DocSelection::range(DocPosition::new(0, 1), DocPosition::new(1, 3));
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        apply_ops(&mut doc, &ops);

        assert!(matches!(doc.block(0), Some(Block::Image { .. })));
        assert_eq!(block_text(&doc, 1), "lo");
    }

    #[test]
    fn delete_from_atom_trailing_edge_cross_block_undo_restores() {
        let image = Block::Image {
            src: "cid:1".into(),
            alt: "logo".into(),
            width: None,
            height: None,
        };
        let original = Document::from_blocks(vec![
            image,
            Block::paragraph("aaa"),
            Block::paragraph("bbb"),
        ]);
        let mut doc = original.clone();
        let sel = DocSelection::range(DocPosition::new(0, 1), DocPosition::new(2, 2));
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        apply_ops(&mut doc, &ops);

        assert!(matches!(doc.block(0), Some(Block::Image { .. })));
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 1), "b");

        for op in ops.iter().rev() {
            op.invert().apply(&mut doc);
        }
        crate::normalize::normalize(&mut doc);
        assert_eq!(doc, original);
    }

    #[test]
    fn delete_selection_collapsed_to_atom_trailing_edge_is_noop() {
        // (atom, 1) .. (next, 0) selects nothing once normalized.
        let image = Block::Image {
            src: "cid:1".into(),
            alt: String::new(),
            width: None,
            height: None,
        };
        let doc = Document::from_blocks(vec![image, Block::paragraph("hello")]);
        let sel = DocSelection::range(DocPosition::new(0, 1), DocPosition::new(1, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        assert!(ops.is_empty());
    }

    #[test]
    fn insert_replacing_styled_selection_keeps_style() {
        // Replacing a fully-bold selection with typed text: the replacement
        // inherits the deleted text's bold, even though the deletion removes
        // the bold runs the insert would otherwise inherit from.
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("abc", InlineStyle::BOLD)],
        }]);
        let sel = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 3));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("x".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);

        assert_eq!(block_text(&doc, 0), "x");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        for run in runs {
            if !run.is_empty() {
                assert!(
                    run.style.contains(InlineStyle::BOLD),
                    "replacement run {:?} should be bold",
                    run.text
                );
            }
        }
    }

    #[test]
    fn insert_replacing_mixed_selection_takes_first_char_style() {
        // Selection starts inside the italic run: the replacement takes the
        // first selected character's style (italic), not the style of the
        // character before the selection (plain).
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("ab"),
                StyledRun::styled("cde", InlineStyle::ITALIC),
            ],
        }]);
        let sel = DocSelection::range(DocPosition::new(0, 2), DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("x".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);

        assert_eq!(block_text(&doc, 0), "abx");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        let mut pos = 0;
        for run in runs {
            let rlen = run.char_len();
            if pos <= 2 && pos + rlen > 2 {
                assert!(
                    run.style.contains(InlineStyle::ITALIC),
                    "replacement run {:?} should take the first selected char's italic",
                    run.text
                );
            }
            pos += rlen;
        }
    }

    #[test]
    fn delete_selection_collapsed_is_noop() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 2));
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        assert!(ops.is_empty());
    }

    #[test]
    fn delete_selection_single_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn delete_selection_cross_block() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::range(DocPosition::new(0, 3), DocPosition::new(1, 2));
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "helrld");
    }

    // ── Split block (Enter) ─────────────────────────────

    #[test]
    fn split_block_mid_text() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), " world");
    }

    #[test]
    fn split_block_at_start() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 0));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "");
        assert_eq!(block_text(&doc, 1), "hello");
    }

    #[test]
    fn split_paragraph_at_end() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), "");
        // Both should be paragraphs.
        assert!(matches!(doc.block(0), Some(Block::Paragraph { .. })));
        assert!(matches!(doc.block(1), Some(Block::Paragraph { .. })));
    }

    #[test]
    fn split_heading_mid_text_preserves_heading() {
        let mut doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H2,
            runs: vec![StyledRun::plain("My Title")],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H2,
                ..
            })
        ));
        assert!(matches!(
            doc.block(1),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H2,
                ..
            })
        ));
        assert_eq!(block_text(&doc, 0), "My ");
        assert_eq!(block_text(&doc, 1), "Title");
    }

    #[test]
    fn split_heading_at_end_resets_to_paragraph() {
        let mut doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H1,
            runs: vec![StyledRun::plain("Title")],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        // First block stays as heading.
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H1,
                ..
            })
        ));
        // Second block becomes paragraph.
        assert!(matches!(doc.block(1), Some(Block::Paragraph { .. })));
        assert_eq!(block_text(&doc, 0), "Title");
        assert_eq!(block_text(&doc, 1), "");
    }

    #[test]
    fn split_with_selection_deletes_first() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(0, 11));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), "");
    }

    // ── Toggle inline style ─────────────────────────────

    #[test]
    fn toggle_bold_with_selection_produces_op() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], EditOp::ToggleInlineStyle { .. }));
        if let EditOp::ToggleInlineStyle {
            start,
            end,
            style_bit,
        } = &ops[0]
        {
            assert_eq!(start.offset, 0);
            assert_eq!(end.offset, 5);
            assert_eq!(*style_bit, InlineStyle::BOLD);
        }
    }

    #[test]
    fn toggle_bold_at_caret_returns_empty_vec() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        assert!(ops.is_empty(), "empty Vec signals 'toggle pending style'");
    }

    #[test]
    fn toggle_italic_with_cross_block_selection() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::range(DocPosition::new(0, 2), DocPosition::new(1, 3));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::ITALIC),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        if let EditOp::ToggleInlineStyle {
            start,
            end,
            style_bit,
        } = &ops[0]
        {
            assert_eq!(*start, DocPosition::new(0, 2));
            assert_eq!(*end, DocPosition::new(1, 3));
            assert_eq!(*style_bit, InlineStyle::ITALIC);
        }
    }

    #[test]
    fn toggle_bold_with_selection_applies_correctly() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        let sel = DocSelection::range(DocPosition::new(0, 0), DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        // After toggle, chars 0-5 should be bold.
        let mut pos = 0;
        for run in runs {
            let rlen = run.char_len();
            let rend = pos + rlen;
            if rlen > 0 && pos < 5 && rend > 0 {
                assert!(run.style.contains(InlineStyle::BOLD));
            }
            pos = rend;
        }
    }

    // ── Style resolution ────────────────────────────────

    #[test]
    fn resolve_style_at_inside_bold_run() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("hello", InlineStyle::BOLD)],
        }]);
        let style = resolve_style_at(&doc, DocPosition::new(0, 2));
        assert_eq!(style, InlineStyle::BOLD);
    }

    #[test]
    fn resolve_style_at_boundary_uses_left_affinity() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::styled("hello", InlineStyle::BOLD),
                StyledRun::plain(" world"),
            ],
        }]);
        // At offset 5 (boundary between bold "hello" and plain " world").
        let style = resolve_style_at(&doc, DocPosition::new(0, 5));
        assert_eq!(style, InlineStyle::BOLD);
    }

    #[test]
    fn resolve_style_at_start_of_block() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("hello", InlineStyle::ITALIC)],
        }]);
        let style = resolve_style_at(&doc, DocPosition::new(0, 0));
        assert_eq!(style, InlineStyle::ITALIC);
    }

    #[test]
    fn resolve_style_at_link_boundary_excludes_link() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::linked("click", InlineStyle::empty(), "https://example.com"),
                StyledRun::plain(" here"),
            ],
        }]);
        // At offset 5 (right after the link "click").
        // Should NOT inherit link style - use the plain run's style.
        let style = resolve_style_at(&doc, DocPosition::new(0, 5));
        assert_eq!(style, InlineStyle::empty());
    }

    // ── Document minimum ────────────────────────────────

    #[test]
    fn backspace_single_char_document_leaves_empty_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("a")]);
        let sel = DocSelection::caret(DocPosition::new(0, 1));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "");
    }

    #[test]
    fn delete_forward_single_char_document_leaves_empty_block() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("a")]);
        let sel = DocSelection::caret(DocPosition::new(0, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "");
    }

    // ── Backward selection direction ────────────────────

    #[test]
    fn backward_selection_handled_correctly() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello world")]);
        // Backward selection: focus before anchor.
        let sel = DocSelection::range(
            DocPosition::new(0, 11), // anchor
            DocPosition::new(0, 5),  // focus
        );
        let ops = resolve(&doc, sel, EditAction::DeleteSelection, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    // ── Undo round-trips ────────────────────────────────

    #[test]
    fn backspace_char_undoes_correctly() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "helo");

        // Undo.
        for op in ops.iter().rev() {
            op.invert().apply(&mut doc);
        }
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn backspace_merge_undoes_correctly() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::paragraph("world")]);
        let sel = DocSelection::caret(DocPosition::new(1, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "helloworld");

        // Undo.
        for op in ops.iter().rev() {
            op.invert().apply(&mut doc);
        }
        assert_eq!(doc.block_count(), 2);
        assert_eq!(block_text(&doc, 0), "hello");
        assert_eq!(block_text(&doc, 1), "world");
    }

    #[test]
    fn split_heading_at_end_undoes_correctly() {
        let mut doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H1,
            runs: vec![StyledRun::plain("Title")],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);

        // Undo: apply ops in reverse order, each inverted.
        for op in ops.iter().rev() {
            op.invert().apply(&mut doc);
        }
        assert_eq!(doc.block_count(), 1);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert_eq!(block_text(&doc, 0), "Title");
    }

    // ── Edge cases ──────────────────────────────────────

    #[test]
    fn insert_into_empty_document() {
        let mut doc = Document::new();
        let sel = DocSelection::caret(DocPosition::zero());
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("hello".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn split_empty_block() {
        let mut doc = Document::new();
        let sel = DocSelection::caret(DocPosition::zero());
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 2);
    }

    #[test]
    fn backspace_at_start_of_second_empty_block() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::empty_paragraph()]);
        let sel = DocSelection::caret(DocPosition::new(1, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn delete_forward_at_end_of_first_block_with_empty_second() {
        let mut doc =
            Document::from_blocks(vec![Block::paragraph("hello"), Block::empty_paragraph()]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(doc.block_count(), 1);
        assert_eq!(block_text(&doc, 0), "hello");
    }

    #[test]
    fn delete_forward_captures_correct_deleted_content() {
        let doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 0));
        let ops = resolve(&doc, sel, EditAction::DeleteForward, InlineStyle::empty());
        assert_eq!(ops.len(), 1);
        if let EditOp::DeleteRange {
            start,
            end,
            deleted,
        } = &ops[0]
        {
            assert_eq!(start.offset, 0);
            assert_eq!(end.offset, 1);
            assert_eq!(deleted.blocks[0].flattened_text(), "h");
        } else {
            panic!("expected DeleteRange");
        }
    }

    #[test]
    fn insert_unicode_text() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("caf")]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("\u{00e9}".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "caf\u{00e9}");
    }

    #[test]
    fn backspace_unicode_char() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("caf\u{00e9}")]);
        let sel = DocSelection::caret(DocPosition::new(0, 4));
        let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "caf");
    }

    #[test]
    fn heading_h3_reset_on_split_at_end() {
        let mut doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H3,
            runs: vec![StyledRun::plain("Subheading")],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 10));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H3,
                ..
            })
        ));
        assert!(matches!(doc.block(1), Some(Block::Paragraph { .. })));
    }

    #[test]
    fn split_heading_mid_text_both_are_headings() {
        let mut doc = Document::from_blocks(vec![Block::Heading {
            attrs: Default::default(),
            level: HeadingLevel::H1,
            runs: vec![StyledRun::plain("Hello World")],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(&doc, sel, EditAction::SplitBlock, InlineStyle::empty());
        apply_ops(&mut doc, &ops);
        // Both should be H1.
        assert!(matches!(
            doc.block(0),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H1,
                ..
            })
        ));
        assert!(matches!(
            doc.block(1),
            Some(Block::Heading {
                attrs: _,
                level: HeadingLevel::H1,
                ..
            })
        ));
    }

    #[test]
    fn multiple_consecutive_backspaces() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        // Delete from end, one at a time.
        for expected_len in (0..5).rev() {
            let len = doc.block(0).map(Block::char_len).unwrap_or(0);
            let sel = DocSelection::caret(DocPosition::new(0, len));
            let ops = resolve(&doc, sel, EditAction::DeleteBackward, InlineStyle::empty());
            apply_ops(&mut doc, &ops);
            assert_eq!(block_text(&doc, 0).chars().count(), expected_len);
        }
        assert_eq!(block_text(&doc, 0), "");
    }

    #[test]
    fn multiple_consecutive_inserts() {
        let mut doc = Document::new();
        let text = "hello";
        for (i, ch) in text.chars().enumerate() {
            let sel = DocSelection::caret(DocPosition::new(0, i));
            let ops = resolve(
                &doc,
                sel,
                EditAction::InsertText(ch.to_string()),
                InlineStyle::empty(),
            );
            apply_ops(&mut doc, &ops);
        }
        assert_eq!(block_text(&doc, 0), "hello");
    }

    // ── Pending style insertion ──────────────────────────

    #[test]
    fn insert_with_pending_bold_into_plain_text() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("x".into()),
            InlineStyle::BOLD,
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hellox");
        // The 'x' should be bold.
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        // Find the run containing 'x' (last char).
        let mut pos = 0;
        for run in runs {
            let rlen = run.char_len();
            let rend = pos + rlen;
            if rend > 5 && pos <= 5 {
                // This run covers offset 5 (the 'x').
                assert!(
                    run.style.contains(InlineStyle::BOLD),
                    "inserted 'x' should be bold, but run '{}' has style {:?}",
                    run.text,
                    run.style,
                );
            }
            pos = rend;
        }
    }

    #[test]
    fn insert_with_pending_italic_into_bold_text() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("hello", InlineStyle::BOLD)],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("x".into()),
            InlineStyle::BOLD | InlineStyle::ITALIC,
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hellox");
        // The 'x' should be bold + italic.
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        let mut pos = 0;
        for run in runs {
            let rlen = run.char_len();
            let rend = pos + rlen;
            if rend > 5 && pos <= 5 {
                assert!(
                    run.style.contains(InlineStyle::BOLD | InlineStyle::ITALIC),
                    "inserted 'x' should be bold+italic, but run '{}' has style {:?}",
                    run.text,
                    run.style,
                );
            }
            pos = rend;
        }
    }

    #[test]
    fn insert_without_pending_style_inherits_run_style() {
        let mut doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::styled("hello", InlineStyle::BOLD)],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("X".into()),
            InlineStyle::empty(),
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "helXlo");
        // The 'X' should inherit bold from the run.
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1, "should still be a single bold run");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[0].text, "helXlo");
    }

    #[test]
    fn insert_pending_bold_undoes_correctly() {
        let mut doc = Document::from_blocks(vec![Block::paragraph("hello")]);
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::InsertText("x".into()),
            InlineStyle::BOLD,
        );
        apply_ops(&mut doc, &ops);
        assert_eq!(block_text(&doc, 0), "hellox");

        // Undo: apply ops in reverse order, each inverted.
        for op in ops.iter().rev() {
            op.invert().apply(&mut doc);
        }
        assert_eq!(block_text(&doc, 0), "hello");
        // Original text should be plain.
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        for run in runs {
            assert!(
                !run.style.contains(InlineStyle::BOLD),
                "after undo, no run should be bold",
            );
        }
    }

    // ── Link formatting at caret ────────────────────────

    #[test]
    fn toggle_italic_inside_bold_link_formats_whole_link() {
        // Cursor inside a bold link: toggle italic should format the whole link.
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("before "),
                StyledRun::linked("click here", InlineStyle::BOLD, "https://example.com"),
                StyledRun::plain(" after"),
            ],
        }]);
        // Cursor at offset 10 (inside "click here", which starts at offset 7).
        let sel = DocSelection::caret(DocPosition::new(0, 10));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::ITALIC),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        if let EditOp::ToggleInlineStyle {
            start,
            end,
            style_bit,
        } = &ops[0]
        {
            assert_eq!(*start, DocPosition::new(0, 7));
            assert_eq!(*end, DocPosition::new(0, 17));
            assert_eq!(*style_bit, InlineStyle::ITALIC);
        } else {
            panic!("expected ToggleInlineStyle op");
        }
    }

    #[test]
    fn toggle_bold_inside_plain_link_formats_whole_link() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![StyledRun::linked(
                "my link",
                InlineStyle::empty(),
                "https://example.com",
            )],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 3));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        if let EditOp::ToggleInlineStyle {
            start,
            end,
            style_bit,
        } = &ops[0]
        {
            assert_eq!(*start, DocPosition::new(0, 0));
            assert_eq!(*end, DocPosition::new(0, 7));
            assert_eq!(*style_bit, InlineStyle::BOLD);
        } else {
            panic!("expected ToggleInlineStyle op");
        }
    }

    #[test]
    fn toggle_bold_not_inside_link_returns_empty() {
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("plain text"),
                StyledRun::linked("link", InlineStyle::empty(), "https://example.com"),
            ],
        }]);
        // Cursor at offset 5, inside the plain text (not the link).
        let sel = DocSelection::caret(DocPosition::new(0, 5));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        assert!(ops.is_empty(), "should return empty vec for pending style");
    }

    #[test]
    fn toggle_at_link_edge_formats_whole_link() {
        // Cursor at offset 0 of a link that starts at offset 0.
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::linked("link text", InlineStyle::empty(), "https://example.com"),
                StyledRun::plain(" after"),
            ],
        }]);
        let sel = DocSelection::caret(DocPosition::new(0, 0));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        if let EditOp::ToggleInlineStyle { start, end, .. } = &ops[0] {
            assert_eq!(*start, DocPosition::new(0, 0));
            assert_eq!(*end, DocPosition::new(0, 9));
        } else {
            panic!("expected ToggleInlineStyle op");
        }
    }

    #[test]
    fn toggle_at_link_end_boundary_formats_whole_link() {
        // Cursor at the END of a link (right after link, before plain text).
        // resolve_offset puts this in the plain run, but find_link_boundaries
        // should fall back to the previous run and recognize the link.
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::linked("link text", InlineStyle::empty(), "https://example.com"),
                StyledRun::plain(" after"),
            ],
        }]);
        // Cursor at offset 9 = right after "link text", at boundary with " after"
        let sel = DocSelection::caret(DocPosition::new(0, 9));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::BOLD),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        if let EditOp::ToggleInlineStyle { start, end, .. } = &ops[0] {
            assert_eq!(*start, DocPosition::new(0, 0));
            assert_eq!(*end, DocPosition::new(0, 9));
        } else {
            panic!("expected ToggleInlineStyle op, got: {ops:?}");
        }
    }

    #[test]
    fn toggle_spans_multiple_adjacent_link_runs_same_href() {
        // Multiple adjacent runs with the same link href but different styles.
        let doc = Document::from_blocks(vec![Block::Paragraph {
            attrs: Default::default(),
            runs: vec![
                StyledRun::plain("pre "),
                StyledRun::linked("bold", InlineStyle::BOLD, "https://example.com"),
                StyledRun::linked(" part", InlineStyle::empty(), "https://example.com"),
                StyledRun::plain(" post"),
            ],
        }]);
        // Cursor at offset 6 (inside "bold", which starts at offset 4).
        let sel = DocSelection::caret(DocPosition::new(0, 6));
        let ops = resolve(
            &doc,
            sel,
            EditAction::ToggleInlineStyle(InlineStyle::ITALIC),
            InlineStyle::empty(),
        );
        assert_eq!(ops.len(), 1);
        if let EditOp::ToggleInlineStyle {
            start,
            end,
            style_bit,
        } = &ops[0]
        {
            // Should span the whole contiguous link: "bold part" = offsets 4..13.
            assert_eq!(*start, DocPosition::new(0, 4));
            assert_eq!(*end, DocPosition::new(0, 13));
            assert_eq!(*style_bit, InlineStyle::ITALIC);
        } else {
            panic!("expected ToggleInlineStyle op");
        }
    }
}
