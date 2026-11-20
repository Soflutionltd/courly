//! Raw-RFC822 re-parse for JMAP hydration fidelity (B3a-cut-jmap 4.2).
//!
//! The conduit structured `Message` is lossy relative to the fields the
//! legacy JMAP `upsert_messages` / `upsert_attachments` path persisted: it
//! drops the `Message-ID` header, the List-Unsubscribe(-Post) headers, the
//! MDN request bit, the full (space-joined) In-Reply-To id list, the
//! `Authentication-Results` header, and every per-attachment detail the
//! structured `BlobHandle` cannot carry (the part name, the `Content-ID`,
//! and the inline disposition). Those all live in the verbatim MIME the
//! server assembled, so the consumer re-parses the `open_raw_rfc822` octets
//! and layers the recovered fields back on, matching the legacy output.
//!
//! This module is the single re-parse path shared by the production
//! consumer (`crates/service/.../hydrate.rs`) and the byte-identical golden
//! equality test, so the two cannot drift: both feed identical raw MIME
//! through the identical extraction here.

use mail_parser::{MessageParser, MimeHeaders};

use crate::email_parsing::format_address_list;
use crate::text::snippet_from_text_body;

/// One attachment recovered from the raw MIME, carrying the fields the
/// structured `BlobHandle` cannot: the part `name`, the `Content-ID`, and
/// the inline disposition (`Content-Disposition: inline`). Matched to a
/// conduit blob handle by ordinal position in the attachment list.
#[derive(Debug, Clone, Default)]
pub struct Rfc822Attachment {
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    /// Decoded MIME-part text for calendar parts. Kept only for iMIP UID
    /// extraction during hydration; ordinary attachments do not retain bytes.
    pub calendar_payload: Option<String>,
}

/// The RFC822-derived field set the legacy JMAP persist wrote that the
/// conduit structured `Message` does not expose. The consumer merges this
/// onto the structured membership/flag/importance state.
#[derive(Debug, Clone, Default)]
pub struct Rfc822Parsed {
    /// `Message-ID` header (legacy `email.message_id().join(" ")`).
    pub message_id_header: Option<String>,
    /// `In-Reply-To` header, ALL ids space-joined (legacy joined every id;
    /// the structured `Message::in_reply_to` keeps only the first).
    pub in_reply_to_header: Option<String>,
    /// `References` header, all ids space-joined.
    pub references_header: Option<String>,
    /// `List-Unsubscribe` header text.
    pub list_unsubscribe: Option<String>,
    /// `List-Unsubscribe-Post` header text.
    pub list_unsubscribe_post: Option<String>,
    /// `Authentication-Results` header text.
    pub auth_results: Option<String>,
    /// Whether `Disposition-Notification-To` is present (MDN requested).
    pub mdn_requested: bool,
    /// Plain-text body of the first non-AMP text part.
    pub body_text: Option<String>,
    /// HTML body of the first non-AMP text/html part.
    pub body_html: Option<String>,
    /// Per-attachment detail keyed by attachment ordinal.
    pub attachments: Vec<Rfc822Attachment>,
}

/// Re-parse the verbatim RFC822 octets the server assembled and recover the
/// fields the structured `Message` dropped. Returns `Ok(parsed)`; an
/// unparseable message yields `Err` so the caller can fall back to the
/// structured-only row (degraded, never dropped - B3-spec 4.1.3).
pub fn parse_rfc822(parser: &MessageParser, raw: &[u8]) -> Result<Rfc822Parsed, String> {
    let message = parser
        .parse(raw)
        .ok_or("Failed to parse JMAP RFC822 message")?;

    // Message-ID: legacy joined all ids with a space.
    let message_id_header = join_text_ids(message.header(mail_parser::HeaderName::MessageId));

    // In-Reply-To: legacy `email.in_reply_to().join(" ")` - ALL ids, not
    // just the first. The structured `Message::in_reply_to` keeps only the
    // first, which is the divergence this re-parse repairs.
    let in_reply_to_header = join_text_ids(message.header(mail_parser::HeaderName::InReplyTo));

    let references_header = join_text_ids(message.header(mail_parser::HeaderName::References));

    // List-Unsubscribe / -Post and Authentication-Results are URI- and
    // structured-token headers mail-parser does not always classify as a
    // plain `Text` value (the angle-bracket URI list parses to a non-text
    // variant), so read them verbatim from the raw header block. This also
    // keeps the stored value byte-identical to what the server sent.
    let list_unsubscribe = raw_header(raw, "list-unsubscribe");
    let list_unsubscribe_post = raw_header(raw, "list-unsubscribe-post");
    let auth_results = raw_header(raw, "authentication-results");
    let mdn_requested = raw_header(raw, "disposition-notification-to").is_some();

    // Body: legacy preferred the FIRST non-AMP part, not an all-parts join.
    // mail-parser's body_text(0)/body_html(0) already select the first
    // text/plain and text/html part; guard the HTML selection against AMP.
    let body_text = message.body_text(0).map(|text| text.to_string());
    let body_html = message.body_html(0).and_then(|html| {
        if let Some(&part_idx) = message.html_body.first()
            && let Some(part) = message.parts.get(part_idx as usize)
            && let Some(ct) = part.content_type()
            && let Some(subtype) = ct.subtype()
            && crate::email_parsing::is_amp_content_type(&format!("{}/{subtype}", ct.ctype()))
        {
            return None;
        }
        Some(html.to_string())
    });
    // Some senders write markup in their plain text part, a whole HTML page
    // at times. That part is what search indexes and the list previews, so
    // its tags and style rules matched searches ("meta", "table") the mail
    // says nothing about. The text is then taken from the HTML part.
    let body_text = match (body_text, body_html.as_deref()) {
        (Some(text), Some(html)) if holds_markup(&text) => {
            Some(mail_parser::decoders::html::html_to_text(html))
        }
        (text, _) => text,
    };

    let attachments = message
        .attachments
        .iter()
        .filter_map(|&part_idx| {
            let part = message.parts.get(part_idx as usize)?;
            let mime_type = part
                .content_type()
                .map(|ct| {
                    let ctype = ct.ctype();
                    let subtype = ct.subtype().unwrap_or("octet-stream");
                    // Preserve the iMIP `method=` parameter so the consumer's
                    // `extract_imip_method` recovers the legacy
                    // `meeting_invite_method` (REQUEST/REPLY/CANCEL). The bare
                    // `type/subtype` join would drop it.
                    match ct.attribute("method") {
                        Some(method) => format!("{ctype}/{subtype}; method={method}"),
                        None => format!("{ctype}/{subtype}"),
                    }
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let is_inline = part
                .content_disposition()
                .is_some_and(mail_parser::ContentType::is_inline);
            let calendar_payload = crate::email_parsing::is_calendar_content_type(&mime_type)
                .then(|| String::from_utf8_lossy(part.contents()).into_owned());
            Some(Rfc822Attachment {
                filename: part.attachment_name().unwrap_or("attachment").to_string(),
                mime_type,
                size: i64::try_from(part.len()).unwrap_or(i64::MAX),
                content_id: part.content_id().map(ToString::to_string),
                is_inline,
                calendar_payload,
            })
        })
        .collect();

    Ok(Rfc822Parsed {
        message_id_header,
        in_reply_to_header,
        references_header,
        list_unsubscribe,
        list_unsubscribe_post,
        auth_results,
        mdn_requested,
        body_text,
        body_html,
        attachments,
    })
}

/// Whether `text`, given as plain text, is written with HTML tags.
fn holds_markup(text: &str) -> bool {
    const TAGS: [&str; 12] = [
        "!doctype", "html", "head", "body", "meta", "style", "table", "div", "span", "td", "p",
        "br",
    ];
    text.match_indices('<').any(|(at, _)| {
        let rest = &text[at + 1..];
        TAGS.iter().any(|tag| {
            rest.get(..tag.len())
                .is_some_and(|name| name.eq_ignore_ascii_case(tag))
                && rest[tag.len()..]
                    .chars()
                    .next()
                    .is_some_and(|after| after == '>' || after == '/' || after.is_whitespace())
        })
    })
}

/// Derive the snippet the legacy path stored. Legacy used the JMAP server's
/// `email.preview()`; conduit's structured `Message` does not surface a
/// preview, so the consumer derives it from the re-parsed text body exactly
/// as the IMAP raw-MIME path does (`snippet_from_text_body`, 200 graphemes).
#[must_use]
pub fn snippet_from_body(body_text: Option<&str>) -> String {
    body_text
        .map(|text| snippet_from_text_body(text, 200))
        .unwrap_or_default()
}

/// Format a mail-parser address field as the comma-joined "Name <email>"
/// string the DB stores.
#[must_use]
pub fn format_addr_field(addr: Option<&mail_parser::Address>) -> Option<String> {
    let addr = addr?;
    format_address_list(addr.iter().map(|a| {
        (
            a.name.as_ref().map(ToString::to_string),
            a.address
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
    }))
}

fn join_text_ids(hv: Option<&mail_parser::HeaderValue>) -> Option<String> {
    match hv {
        Some(mail_parser::HeaderValue::Text(t)) => Some(t.to_string()),
        Some(mail_parser::HeaderValue::TextList(list)) if !list.is_empty() => {
            Some(list.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(" "))
        }
        _ => None,
    }
}

/// Read a header verbatim from the raw RFC822 header block (the bytes before
/// the first blank line), case-insensitively, unfolding continuation lines.
/// Returns the first matching header's value with surrounding whitespace
/// trimmed, or `None` if absent.
fn raw_header(raw: &[u8], name_lower: &str) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let header_block = text
        .split_once("\r\n\r\n")
        .or_else(|| text.split_once("\n\n"))
        .map_or(text.as_ref(), |(headers, _)| headers);

    let mut lines = header_block.lines().peekable();
    while let Some(line) = lines.next() {
        // A continuation line belongs to the header above it. A DKIM
        // signature folds its `h=` list, so a line such as
        // `\tList-Unsubscribe:Date:From` is not a List-Unsubscribe header.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        if !field.trim().eq_ignore_ascii_case(name_lower) {
            continue;
        }
        let mut value = value.trim().to_string();
        // Unfold continuation lines (those beginning with whitespace).
        while let Some(next) = lines.peek() {
            if next.starts_with(' ') || next.starts_with('\t') {
                value.push(' ');
                value.push_str(next.trim());
                lines.next();
            } else {
                break;
            }
        }
        return Some(value);
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{parse_rfc822, snippet_from_body};
    use mail_parser::MessageParser;

    const RAW: &[u8] = b"Message-ID: <root@example.test>\r\n\
In-Reply-To: <a@example.test> <b@example.test>\r\n\
References: <a@example.test> <b@example.test>\r\n\
List-Unsubscribe: <https://example.test/u>\r\n\
List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\
Disposition-Notification-To: sender@example.test\r\n\
Authentication-Results: example.test; spf=pass\r\n\
Subject: Re: hello\r\n\
From: Alice <alice@example.test>\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hello body text.";

    #[test]
    fn recovers_dropped_headers() {
        let parsed = parse_rfc822(&MessageParser::default(), RAW).unwrap();
        assert_eq!(
            parsed.message_id_header.as_deref(),
            Some("root@example.test")
        );
        // In-Reply-To joins ALL ids, not just the first.
        assert_eq!(
            parsed.in_reply_to_header.as_deref(),
            Some("a@example.test b@example.test")
        );
        assert!(parsed.list_unsubscribe.is_some());
        assert!(parsed.list_unsubscribe_post.is_some());
        assert!(parsed.auth_results.is_some());
        assert!(parsed.mdn_requested);
        assert_eq!(parsed.body_text.as_deref(), Some("Hello body text."));
        assert_eq!(
            snippet_from_body(parsed.body_text.as_deref()),
            "Hello body text."
        );
    }

    #[test]
    fn folded_dkim_tag_list_is_not_taken_for_a_header() {
        let raw: &[u8] = b"DKIM-Signature: v=1; a=rsa-sha256; d=example.test;\r\n\
\th=Subject:From:To:\r\n\
\tList-Unsubscribe:Date; b=abc\r\n\
List-Unsubscribe: <https://example.test/u>\r\n\
Subject: hi\r\n\
\r\n\
Body.";
        let parsed = parse_rfc822(&MessageParser::default(), raw).unwrap();
        assert_eq!(
            parsed.list_unsubscribe.as_deref(),
            Some("<https://example.test/u>")
        );
    }

    /// A newsletter with no plain text part: its text is what a reader sees
    /// of the HTML, none of the markup around it. A search for "meta" or
    /// "style" must not find it through its tags.
    #[test]
    fn the_text_of_an_html_only_message_holds_no_markup() {
        let raw: &[u8] = b"Subject: receipt\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><head><!--[if !mso]><!--> <meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\"> <!--<![endif]-->\r\n\
<meta name=\"viewport\" content=\"width=device-width\">\r\n\
<title>Receipt</title>\r\n\
<style>a, a:link { color:#007eff; }</style></head>\r\n\
<body><!-- hidden note --><p>Your receipt &amp; total: 12&nbsp;EUR</p>\r\n\
<div style=\"display:none\">x</div><p>Thanks</p></body></html>";
        let parsed = parse_rfc822(&MessageParser::default(), raw).unwrap();
        let text = parsed.body_text.unwrap();
        for markup in ["meta", "viewport", "color", "hidden note", "<", "Receipt<"] {
            assert!(!text.contains(markup), "{markup:?} left in {text:?}");
        }
        assert!(text.contains("Your receipt & total: 12"), "{text:?}");
        assert!(text.contains("Thanks"), "{text:?}");
    }

    /// What Apple, Meta and AWS notices send: a plain text alternative that
    /// is itself an HTML page.
    #[test]
    fn markup_sent_as_plain_text_is_replaced_by_the_text_of_the_html() {
        let raw: &[u8] = b"Subject: notice\r\n\
Content-Type: multipart/alternative; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
<!DOCTYPE html><html><head><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">\r\n\
<style>a, a:link { color:#007eff; }</style></head><body><p>Setup done</p></body></html>\r\n\
--b\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><head><meta name=\"viewport\" content=\"width=device-width\">\r\n\
<style>a { color:#007eff; }</style></head><body><p>Setup done</p></body></html>\r\n\
--b--\r\n";
        let parsed = parse_rfc822(&MessageParser::default(), raw).unwrap();
        let text = parsed.body_text.unwrap();
        for markup in ["meta", "viewport", "color", "<"] {
            assert!(!text.contains(markup), "{markup:?} left in {text:?}");
        }
        assert!(text.contains("Setup done"), "{text:?}");
    }

    #[test]
    fn plain_text_that_only_uses_angle_brackets_is_kept() {
        let raw: &[u8] = b"Subject: hi\r\n\
Content-Type: multipart/alternative; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain\r\n\
\r\n\
Write to Ann <ann@example.test>, price < 5 and > 2, see <https://example.test/path>.\r\n\
--b\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>Other words</p>\r\n\
--b--\r\n";
        let parsed = parse_rfc822(&MessageParser::default(), raw).unwrap();
        assert!(parsed.body_text.unwrap().starts_with("Write to Ann <ann@"));
    }

    #[test]
    fn malformed_does_not_panic() {
        // A header-less body still parses; an empty input parses to an empty
        // message rather than erroring, so assert the call is total.
        let parsed = parse_rfc822(&MessageParser::default(), b"").unwrap_or_default();
        assert!(parsed.message_id_header.is_none());
    }
}
