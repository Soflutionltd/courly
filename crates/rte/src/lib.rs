//! Rich text editor for email composition in iced.
//!
//! Built from scratch - no existing rich text editor exists for iced. Design
//! informed by deep study of ProseMirror, Slate.js, Quill, and fleather.
//!
//! # Crate structure
//!
//! **Pure Rust** (no iced dependency): `document`, `operations`, `normalize`,
//! `rules`, `undo`, `html_serialize`, `html_parse`, `image`. Unit-testable
//! without a GUI.
//!
//! **Feature-gated** (`widget` feature, default on): `widget` module depends
//! on iced for rendering, input handling, and cursor management.

pub mod compose;
pub mod document;
pub mod html_parse;
pub mod html_serialize;
pub mod image;
pub mod normalize;
pub mod operations;
pub mod rules;
pub mod table_box;
pub mod undo;

#[cfg(feature = "widget")]
pub mod widget;

pub use document::{
    Block, BlockAttrs, BlockKind, CellStep, DocPosition, DocSelection, DocSlice, Document,
    FontFamily, FormatChange, HeadingLevel, HtmlAttrs, InlineImage, InlineStyle, LINE_HEIGHT_RANGE,
    MAX_INDENT_LEVEL, RUN_SIZE_RANGE, Rgb, RunFormat, StyledRun, Table, TableCell, TableChange,
    TableRow, TextAlignment, clamp_run_size,
};
pub use normalize::normalize;
pub use operations::{EditOp, PosMap};
pub use rules::{EditAction, PendingFormat};
pub use undo::UndoStack;

#[cfg(feature = "widget")]
pub use widget::{Action, EditorState, RichTextEditor, rich_text_editor};
