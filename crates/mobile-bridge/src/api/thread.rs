//! One conversation, opened for reading: every message with its HTML made
//! safe for the phone's web view, and the conversation's attachments.
//!
//! The reads use the desktop's own thread query and content stores. Each
//! message body goes through Courly's HTML sanitizer, inline `cid:` images
//! are embedded from the inline image store, and remote images follow the
//! user's "block remote images" setting unless the reader asked to load
//! them for this conversation.

use std::collections::HashMap;
use std::path::Path;

use base64::Engine as _;
use common::html_sanitizer::{RemoteImagePolicy, sanitize_html_body_with_image_policy};
use db::db::ReadConn;
use db::db::queries_extra::{get_thread_detail, query_inline_cid_hashes};
use rusqlite::OptionalExtension;
use service_api::{MarkChatReadAck, RequestParams};

/// One conversation as the reading screen shows it.
pub struct ThreadView {
    pub subject: String,
    /// Oldest first, the order a phone reads a conversation in.
    pub messages: Vec<MessageView>,
    pub attachments: Vec<AttachmentView>,
    /// True when at least one remote image or style was held back, so the
    /// screen can offer to load them.
    pub remote_images_blocked: bool,
}

/// One message of a conversation.
pub struct MessageView {
    pub id: String,
    /// The sender's name, else their address.
    pub sender: String,
    pub sender_address: String,
    /// Display names of the To and Cc recipients, in order.
    pub recipients: Vec<String>,
    pub date_ms: i64,
    pub is_read: bool,
    pub is_own_message: bool,
    /// A one-line preview for the collapsed form of the message.
    pub preview: String,
    /// A complete HTML document, safe to hand to the web view.
    pub html: String,
}

/// One attachment of the conversation (inline images excluded).
pub struct AttachmentView {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: Option<i64>,
}

/// Content Security Policy of every message document: no script, no frame,
/// no form, and network images only when the reader allows them.
fn content_security_policy(allow_remote: bool) -> &'static str {
    if allow_remote {
        "default-src 'none'; img-src data: https: http:; style-src 'unsafe-inline' https:; font-src data: https:"
    } else {
        "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:"
    }
}

pub(crate) fn open_read_only(path: &Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("open {}: {e}", path.display()))
}

/// The user's "block remote images" setting; blocked unless they turned
/// it off, as on the desktop.
fn remote_images_blocked_by_setting(conn: &rusqlite::Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'block_remote_images'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("read block_remote_images: {e}"))?;
    Ok(value.as_deref() != Some("false"))
}

/// Open one conversation for reading. `load_remote_images` overrides the
/// blocking setting for this conversation only.
pub fn thread_detail(
    data_dir: String,
    account_id: String,
    thread_id: String,
    load_remote_images: bool,
) -> Result<ThreadView, String> {
    let dir = Path::new(&data_dir);
    let conn = open_read_only(&dir.join(db::db::DB_FILE_NAME))?;
    let bodies = open_read_only(&dir.join("bodies.db"))?;
    let read = ReadConn::from_raw(&conn);

    let detail = get_thread_detail(&read, &bodies, &account_id, &thread_id)?;
    let cid_hashes = query_inline_cid_hashes(&read, &account_id, &thread_id)?;
    let inline_images = if cid_hashes.is_empty() {
        HashMap::new()
    } else {
        let images = open_read_only(&dir.join("inline_images.db"))?;
        rtsk::inline_image_store::InlineImageStoreReadState::get_batch_sync(&images, &cid_hashes)?
    };

    let allow_remote = load_remote_images || !remote_images_blocked_by_setting(&conn)?;
    let policy = if allow_remote {
        RemoteImagePolicy::AllowRemote
    } else {
        RemoteImagePolicy::StripRemote
    };

    let mut remote_images_blocked = false;
    let mut messages: Vec<MessageView> = detail
        .messages
        .into_iter()
        .map(|message| {
            let body = match message
                .body_html
                .as_deref()
                .filter(|h| !h.trim().is_empty())
            {
                Some(html) => {
                    let safe = sanitize_html_body_with_image_policy(html, policy);
                    if !allow_remote
                        && safe
                            != sanitize_html_body_with_image_policy(
                                html,
                                RemoteImagePolicy::AllowRemote,
                            )
                    {
                        remote_images_blocked = true;
                    }
                    embed_inline_images(&safe, &inline_images)
                }
                None => plain_text_body(message.body_text.as_deref().unwrap_or_default()),
            };
            let mut recipients = display_names(message.to_addresses.as_deref());
            recipients.extend(display_names(message.cc_addresses.as_deref()));
            let sender_address = message.from_address.unwrap_or_default();
            MessageView {
                sender: message
                    .from_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| sender_address.clone()),
                sender_address,
                recipients,
                date_ms: message.date,
                is_read: message.is_read,
                is_own_message: message.is_own_message,
                preview: message.collapsed_summary.unwrap_or_default(),
                html: html_document(&body, content_security_policy(allow_remote)),
                id: message.id,
            }
        })
        .collect();
    messages.sort_by_key(|message| message.date_ms);

    Ok(ThreadView {
        subject: detail.subject.unwrap_or_default(),
        messages,
        attachments: detail
            .attachments
            .into_iter()
            .map(|attachment| AttachmentView {
                filename: attachment.filename.unwrap_or_default(),
                mime_type: attachment.mime_type.unwrap_or_default(),
                size: attachment.size,
                id: attachment.id,
                message_id: attachment.message_id,
            })
            .collect(),
        remote_images_blocked,
    })
}

/// Mark the conversation read, locally at once and at the provider through
/// the engine's action journal, like opening it on the desktop.
pub fn mark_thread_read(account_id: String, thread_id: String) -> Result<(), String> {
    let _ack: MarkChatReadAck = super::engine::call(RequestParams::ActionMarkThreadRead {
        account_id,
        thread_id,
    })?;
    Ok(())
}

/// Replace each `cid:` reference with the image bytes as a `data:` URI.
/// Runs after sanitizing, which drops `data:` images found in the mail
/// itself.
pub(crate) fn embed_inline_images(html: &str, images: &HashMap<String, Vec<u8>>) -> String {
    let mut out = html.to_owned();
    for (cid, bytes) in images {
        let cid = cid.trim_start_matches('<').trim_end_matches('>');
        let data_uri = format!(
            "data:{};base64,{}",
            image_mime_type(bytes),
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        out = out.replace(&format!("cid:{cid}"), &data_uri);
    }
    out
}

/// The image type from its first bytes; the store keeps bytes only.
pub(crate) fn image_mime_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "image/webp",
        [b'<', ..] => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// A plain-text message, escaped and kept with its line breaks.
pub(crate) fn plain_text_body(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<div style=\"white-space:pre-wrap;word-wrap:break-word\">{escaped}</div>")
}

/// Wrap a sanitized body in a document sized for the phone.
pub(crate) fn html_document(body: &str, csp: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<style>html,body{{margin:0;padding:0;background:#fff;color:#1f1f1f;\
font:15px -apple-system,system-ui,sans-serif;-webkit-text-size-adjust:100%;}}\
body{{padding:4px 16px 16px;overflow-wrap:break-word;}}\
img{{max-width:100%;height:auto;}}table{{max-width:100%;}}</style></head>\
<body>{body}</body></html>"
    )
}

/// Display names from a raw address header such as
/// `"Doe, Jane" <jane@example.com>, bob@example.com`.
fn display_names(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in raw.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' | ';' if !quoted => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
        .iter()
        .filter_map(|part| {
            let part = part.trim();
            let name = match part.split_once('<') {
                Some((name, address)) => {
                    let name = name.trim().trim_matches('"').trim();
                    if name.is_empty() {
                        address.trim_end_matches('>').trim()
                    } else {
                        name
                    }
                }
                None => part,
            };
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients_keep_quoted_commas_and_fall_back_to_the_address() {
        assert_eq!(
            display_names(Some(
                "\"Doe, Jane\" <jane@example.com>, <bob@example.com>; carl@example.com"
            )),
            vec!["Doe, Jane", "bob@example.com", "carl@example.com"]
        );
        assert!(display_names(None).is_empty());
    }

    #[test]
    fn inline_images_become_data_uris() {
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        let images = HashMap::from([("<logo@mail>".to_owned(), png)]);
        let html = embed_inline_images("<img src=\"cid:logo@mail\">", &images);
        assert!(html.contains("src=\"data:image/png;base64,"), "{html}");
        assert!(!html.contains("cid:"));
    }

    #[test]
    fn plain_text_is_escaped() {
        let body = plain_text_body("a < b & c");
        assert!(body.contains("a &lt; b &amp; c"));
    }

    #[test]
    fn blocked_documents_forbid_network_images_and_scripts() {
        let csp = content_security_policy(false);
        assert!(csp.contains("img-src data:;"));
        assert!(csp.starts_with("default-src 'none'"));
        assert!(content_security_policy(true).contains("img-src data: https:"));
    }

    #[test]
    fn an_unknown_thread_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().into_owned();
        super::super::database::open_courly_database(path.clone()).expect("migrate");
        rtsk::body_store::open_body_store_connection(dir.path()).expect("body store");
        assert!(thread_detail(path, "acct".into(), "missing".into(), false).is_err());
    }
}
