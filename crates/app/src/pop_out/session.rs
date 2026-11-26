//! Session state persistence for pop-out windows.
//!
//! Saves the full window session (main window + pop-outs) to `session.json`
//! on app close, and restores it on launch. Best-effort - if a message was
//! deleted, the window opens with an error banner.

use crate::window_state::WindowState;
use serde::{Deserialize, Serialize};

/// Full session state, saved on app close.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// Main window geometry and layout state.
    pub main_window: WindowState,

    /// Open message view pop-out windows.
    #[serde(default)]
    pub message_views: Vec<MessageViewSessionEntry>,

    /// Open compose pop-out windows. Each carries the `draft_id` of its
    /// `local_drafts` row, which is the canonical store for compose state -
    /// the entry only needs to remember which draft to reopen and where.
    #[serde(default)]
    pub compose_windows: Vec<ComposeSessionEntry>,

    /// Calendar pop-out window, if one was open. The calendar's view and
    /// date persist via the main DB, so the entry only needs geometry.
    #[serde(default)]
    pub calendar_window: Option<CalendarSessionEntry>,

    /// What the main window was showing, so a relaunch (an update above
    /// all) comes back to the same place.
    #[serde(default)]
    pub main_view: Option<MainViewEntry>,
}

/// Where the reader was in the main window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainViewEntry {
    pub scope: rtsk::scope::ViewScope,
    pub selection: types::SidebarSelection,
    /// The open conversation, as (account id, thread id).
    #[serde(default)]
    pub open_thread: Option<(String, String)>,
    /// How far down the list and the open conversation were scrolled.
    #[serde(default)]
    pub thread_list_offset: f32,
    #[serde(default)]
    pub reading_offset: f32,
    /// The message at the top of the conversation (its index) and how far
    /// down into it the view was: the place, whatever the heights above.
    #[serde(default)]
    pub reading_anchor: Option<(usize, f32)>,
    /// The search the list was showing.
    #[serde(default)]
    pub search: Option<SearchEntry>,
    /// The message being written in place of the reading pane.
    #[serde(default)]
    pub inline_compose: Option<InlineComposeEntry>,
}

/// A search in progress: the typed words and the person picked as a pill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntry {
    pub query: String,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_name: Option<String>,
}

/// The message being written in the main window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineComposeEntry {
    pub draft_id: String,
    #[serde(default)]
    pub detail: ComposeDetail,
}

/// What a draft row does not hold about a message being written: where the
/// caret was, and the files attached so far.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeDetail {
    #[serde(default)]
    pub caret: Option<CaretEntry>,
    #[serde(default)]
    pub attachments: Vec<AttachmentEntry>,
    /// The name each recipient was shown with, by address: a draft keeps
    /// the addresses alone.
    #[serde(default)]
    pub recipient_names: Vec<(String, String)>,
}

/// A selection in the body: both ends as (block index, offset in the block).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CaretEntry {
    pub anchor: (usize, usize),
    pub focus: (usize, usize),
    /// The body had the keyboard: typing goes on where it stopped.
    #[serde(default)]
    pub focused: bool,
}

/// A file attached to a message being written. Its bytes are kept in
/// `compose-attachments/<file>` of the data directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentEntry {
    pub name: String,
    pub mime_type: String,
    pub file: String,
    /// A session saved before this field existed attached every file.
    #[serde(default)]
    pub delivery: service_api::AttachmentDelivery,
}

/// Minimal data needed to restore a message view window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageViewSessionEntry {
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Minimal data needed to restore a compose pop-out. The compose body,
/// recipients, subject, signature, and reply context all live on the
/// `local_drafts` row - the entry just records which draft to reopen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeSessionEntry {
    pub draft_id: String,
    #[serde(default)]
    pub detail: ComposeDetail,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Geometry needed to restore the calendar pop-out window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSessionEntry {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

impl SessionState {
    /// Load from disk. Falls back gracefully:
    /// 1. Try `session.json`.
    /// 2. Fall back to `window.json` (migrating from old format).
    /// 3. Use defaults.
    pub fn load(data_dir: &std::path::Path) -> Self {
        // Try session.json first
        let session_path = data_dir.join("session.json");
        if let Ok(bytes) = std::fs::read(&session_path)
            && let Ok(session) = serde_json::from_slice::<Self>(&bytes)
        {
            return session;
        }

        // Fall back to window.json (old format migration)
        let window = WindowState::load(data_dir);
        Self {
            main_window: window,
            message_views: Vec::new(),
            compose_windows: Vec::new(),
            calendar_window: None,
            main_view: None,
        }
    }
}

const ATTACHMENTS_DIR: &str = "compose-attachments";

/// Keep the bytes of `attachments` on disk, one file per distinct content,
/// and return what the session records to find them again. A file that
/// cannot be written is left out: the message reopens without it rather
/// than with a wrong one.
pub fn store_attachments(
    data_dir: &std::path::Path,
    attachments: &[crate::pop_out::compose::ComposeAttachment],
) -> Vec<AttachmentEntry> {
    if attachments.is_empty() {
        return Vec::new();
    }
    let dir = data_dir.join(ATTACHMENTS_DIR);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!("session: cannot create {}: {error}", dir.display());
        return Vec::new();
    }
    attachments
        .iter()
        .filter_map(|attachment| {
            let file = blake3::hash(&attachment.data).to_hex().to_string();
            let path = dir.join(&file);
            if !path.is_file() {
                let temporary = dir.join(format!("{file}.part"));
                let written = std::fs::write(&temporary, attachment.data.as_slice())
                    .and_then(|()| std::fs::rename(&temporary, &path));
                if let Err(error) = written {
                    log::warn!("session: cannot keep an attachment: {error}");
                    return None;
                }
            }
            Some(AttachmentEntry {
                name: attachment.name.clone(),
                mime_type: attachment.mime_type.clone(),
                file,
                delivery: attachment.delivery,
            })
        })
        .collect()
}

/// Read back the attachments a session recorded. One whose file is gone or
/// no longer holds the recorded content is left out.
pub fn load_attachments(
    data_dir: &std::path::Path,
    entries: &[AttachmentEntry],
) -> Vec<crate::pop_out::compose::ComposeAttachment> {
    let dir = data_dir.join(ATTACHMENTS_DIR);
    entries
        .iter()
        .filter(|entry| is_content_name(&entry.file))
        .filter_map(|entry| {
            let data = std::fs::read(dir.join(&entry.file)).ok()?;
            (blake3::hash(&data).to_hex().as_str() == entry.file).then(|| {
                crate::pop_out::compose::ComposeAttachment {
                    name: entry.name.clone(),
                    mime_type: entry.mime_type.clone(),
                    data: std::sync::Arc::new(data),
                    delivery: entry.delivery,
                }
            })
        })
        .collect()
}

/// Remove the kept files no open message refers to any more.
pub fn prune_attachments(data_dir: &std::path::Path, kept: &std::collections::HashSet<String>) {
    let dir = data_dir.join(ATTACHMENTS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_kept = name.to_str().is_some_and(|name| kept.contains(name));
        if !is_kept && let Err(error) = std::fs::remove_file(entry.path()) {
            log::warn!("session: cannot remove {}: {error}", entry.path().display());
        }
    }
}

/// A file name this module wrote: the hex of a hash, never a path.
fn is_content_name(name: &str) -> bool {
    name.len() == blake3::OUT_LEN * 2 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pop_out::compose::ComposeAttachment;

    fn attachment(name: &str, data: &[u8]) -> ComposeAttachment {
        ComposeAttachment {
            name: name.to_string(),
            mime_type: "application/pdf".to_string(),
            data: std::sync::Arc::new(data.to_vec()),
            delivery: service_api::AttachmentDelivery::Attached,
        }
    }

    #[test]
    fn attachments_come_back_as_they_were_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let kept = store_attachments(
            dir.path(),
            &[
                attachment("a.pdf", b"first"),
                attachment("b.pdf", b"second"),
            ],
        );
        let back = load_attachments(dir.path(), &kept);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "a.pdf");
        assert_eq!(back[0].data.as_slice(), b"first");
        assert_eq!(back[1].name, "b.pdf");
        assert_eq!(back[1].data.as_slice(), b"second");
    }

    #[test]
    fn a_file_changed_on_disk_is_not_attached() {
        let dir = tempfile::tempdir().expect("tempdir");
        let kept = store_attachments(dir.path(), &[attachment("a.pdf", b"first")]);
        std::fs::write(
            dir.path().join(ATTACHMENTS_DIR).join(&kept[0].file),
            b"other",
        )
        .expect("overwrite");
        assert!(load_attachments(dir.path(), &kept).is_empty());
    }

    #[test]
    fn a_recorded_name_cannot_leave_the_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("secret"), b"x").expect("write");
        let entry = AttachmentEntry {
            name: "a.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            file: "../secret".to_string(),
            delivery: service_api::AttachmentDelivery::Attached,
        };
        assert!(load_attachments(dir.path(), &[entry]).is_empty());
    }

    #[test]
    fn files_no_message_refers_to_are_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let kept = store_attachments(
            dir.path(),
            &[
                attachment("a.pdf", b"first"),
                attachment("b.pdf", b"second"),
            ],
        );
        let still: std::collections::HashSet<String> = [kept[0].file.clone()].into();
        prune_attachments(dir.path(), &still);
        assert_eq!(load_attachments(dir.path(), &kept).len(), 1);
    }

    #[test]
    fn a_session_saved_before_the_main_view_was_recorded_still_loads() {
        let old = r#"{"main_window":{"width":1280.0,"height":800.0,"x":null,"y":null,
            "maximized":false},"compose_windows":[{"draft_id":"d1","width":600.0,
            "height":500.0,"x":null,"y":null}]}"#;
        let session: SessionState = serde_json::from_str(old).expect("old session");
        assert!(session.main_view.is_none());
        assert!(session.compose_windows[0].detail.attachments.is_empty());
    }
}
