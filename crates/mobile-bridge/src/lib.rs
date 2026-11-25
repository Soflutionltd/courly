//! Rust side of the Courly mobile app. Dart reaches the shared Courly crates
//! through the functions in `api`; `frb_generated` is produced by
//! `flutter_rust_bridge_codegen generate` and is never edited by hand.

pub mod api;
mod frb_generated;
