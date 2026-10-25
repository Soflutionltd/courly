//! IMAP-specific RFC822 MIME projection used only by the Bifrost consumer.
//!
//! This deliberately lives in service rather than common: IMAP MIME section
//! numbering is a protocol wire detail and the consumer is its sole user.

use std::collections::HashMap;

use common::attachment_dedup::{
    dedup_by_key, prefer_missing_clone, prefer_missing_take, prefer_non_placeholder_filename,
};
use mail_parser::MessageParser;
use xxhash_rust::xxh3::xxh3_64;

#[derive(Debug, Clone)]
pub(crate) struct ParsedImapMessage {
    pub(crate) date: i64,
    pub(crate) attachments: Vec<ParsedImapAttachment>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedImapAttachment {
    pub(crate) part_id: String,
    pub(crate) filename: String,
    pub(crate) mime_type: String,
    pub(crate) size: u32,
    pub(crate) content_id: Option<String>,
    pub(crate) is_inline: bool,
    pub(crate) content_hash: Option<db::blob_hash::BlobHash>,
    pub(crate) inline_data: Option<Vec<u8>>,
}

/// Parse the portion of an IMAP RFC822 message which is not represented by
/// Bifrost's structured hydration: wire date fallback and MIME attachments.
///
/// `internal_date` is the server INTERNALDATE in SECONDS, used only when the
/// Date header is absent or unparseable. `messages.date` is Unix MILLIseconds
/// across every provider so the retention / eviction queries can compare
/// against one unit, hence the multiply below: without it an IMAP-only
/// mailbox produces seconds-scale dates, the eviction sweep's
/// `m.date >= window_start * 1000` never matches, and every IMAP attachment
/// becomes tombstone-eligible on the next sweep.
pub(crate) fn parse_message(
    parser: &MessageParser,
    raw: &[u8],
    internal_date: Option<i64>,
) -> Result<ParsedImapMessage, String> {
    let message = parser.parse(raw).ok_or("Failed to parse MIME message")?;
    let date = message
        .date()
        .map(|date| date.to_timestamp())
        .or(internal_date)
        .unwrap_or(0)
        .saturating_mul(1000);
    let section_map = build_imap_section_map(&message);
    // RFC 8621 `attachments` leaves out inline media the body displays, but
    // those parts are exactly the `cid:` images that need an attachment row
    // and an inline-image-store entry, so body media is taken as well.
    let attachments = message
        .root()
        .parts()
        .filter(|part| part.is_attachment() || part.role() == mail_parser::PartRole::MediaBody)
        .filter_map(|attachment| {
            let section = section_map.get(&attachment.id())?.clone();
            let mime_type = attachment
                .content_type()
                .map(|content_type| {
                    let subtype = content_type.subtype().unwrap_or("octet-stream");
                    format!("{}/{subtype}", content_type.ctype())
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let contents = attachment.decoded();
            let contents = contents.as_ref();
            let is_inline = attachment
                .content_disposition()
                .is_some_and(|disposition| disposition.is_inline());
            #[allow(clippy::cast_possible_truncation)]
            let size = attachment.decoded_len() as u32;
            let inline_data = if is_inline
                && (size as usize) <= store::inline_image_store::MAX_INLINE_SIZE
                && mime_type.starts_with("image/")
            {
                Some(contents.to_vec())
            } else {
                None
            };
            Some((
                xxh3_64(contents),
                ParsedImapAttachment {
                    part_id: section,
                    filename: attachment
                        .attachment_name()
                        .unwrap_or("attachment")
                        .to_string(),
                    mime_type,
                    size,
                    content_id: attachment.content_id().map(ToString::to_string),
                    is_inline,
                    content_hash: is_inline.then(|| db::blob_hash::BlobHash::hash(contents)),
                    inline_data,
                },
            ))
        })
        .collect::<Vec<_>>();
    let attachments = dedup_by_key(
        attachments,
        |(hash, _)| *hash,
        |existing, mut new| {
            let existing_placeholder = existing.1.filename == "attachment";
            let new_placeholder = new.1.filename == "attachment";
            prefer_non_placeholder_filename(
                &mut existing.1.filename,
                &new.1.filename,
                existing_placeholder,
                new_placeholder,
            );
            prefer_missing_clone(&mut existing.1.content_id, &new.1.content_id);
            if new.1.is_inline {
                existing.1.is_inline = true;
            }
            prefer_missing_take(&mut existing.1.inline_data, &mut new.1.inline_data);
        },
    )
    .into_iter()
    .map(|(_, attachment)| attachment)
    .collect();

    Ok(ParsedImapMessage { date, attachments })
}

/// Build the IMAP section-number mapping used both for persisted attachment
/// ids and raw-RFC822 attachment-byte extraction.
///
/// Keys are mail-parser part ids. Only the root message's multipart tree is
/// walked; a nested `message/rfc822` part maps as a single leaf.
pub(crate) fn build_imap_section_map(
    message: &mail_parser::Message,
) -> HashMap<mail_parser::PartId, String> {
    fn walk(
        part: mail_parser::MessagePart<'_>,
        prefix: &str,
        map: &mut HashMap<mail_parser::PartId, String>,
    ) {
        if part.is_multipart() {
            for (index, child) in part.children().enumerate() {
                let section = if prefix.is_empty() {
                    (index + 1).to_string()
                } else {
                    format!("{prefix}.{}", index + 1)
                };
                walk(child, &section, map);
            }
        } else {
            map.insert(
                part.id(),
                if prefix.is_empty() {
                    "1".to_string()
                } else {
                    prefix.to_string()
                },
            );
        }
    }

    let mut map = HashMap::new();
    walk(message.root_part(), "", &mut map);
    map
}

/// Extract a decoded leaf MIME part by its IMAP section number.
pub(crate) fn extract_attachment_from_rfc822(raw: &[u8], part_id: &str) -> Result<Vec<u8>, String> {
    let message = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| "Failed to parse MIME message".to_string())?;
    let section_map = build_imap_section_map(&message);
    let part_index = section_map
        .iter()
        .find(|(_, section)| section.as_str() == part_id)
        .map(|(&index, _)| index)
        .ok_or_else(|| format!("Section {part_id} not found in message"))?;
    let part = message
        .part(part_index)
        .ok_or_else(|| format!("Part index {part_index} out of range"))?;
    match part.kind() {
        mail_parser::PartKind::Binary | mail_parser::PartKind::InlineBinary => {
            Ok(part.decoded().into_owned())
        }
        mail_parser::PartKind::Text | mail_parser::PartKind::Html => Ok(part
            .text()
            .map(|text| text.into_owned().into_bytes())
            .unwrap_or_default()),
        mail_parser::PartKind::Message(nested) => Ok(nested.raw().to_vec()),
        mail_parser::PartKind::Multipart => Err(format!(
            "Part {part_id} is a multipart container, not a leaf part"
        )),
    }
}
