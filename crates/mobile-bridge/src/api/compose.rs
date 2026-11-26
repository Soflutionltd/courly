//! Writing a mail from the phone: a new message, a reply or a reply to all,
//! sent through the engine's `action.send`, the journaled job the desktop
//! uses, so a mail written offline leaves once the network is back.
//!
//! A reply follows the desktop's rules: it leaves from the account that
//! received the message, goes to the sender (or, for our own message, to its
//! recipients), and quotes the original under an Outlook-style header, with
//! the inline images it shows attached so the recipient sees them too.

use std::collections::HashMap;
use std::path::Path;

use common::html_sanitizer::sanitize_html_body;
use db::db::ReadConn;
use db::db::queries_extra::{
    ThreadDetailMessage, get_all_accounts_sync, get_thread_detail, html_to_plain_text,
    query_inline_cid_hashes,
};
use rtsk::seen_addresses::parse::parse_address_list;
use service_api::{
    PlanId, RequestParams, SendAck, SendAttachmentSource, SendIntent, SendWireAttachment,
    SendWireMessage, SendWireRequest,
};

use super::thread::{embed_inline_images, image_mime_type, open_read_only, plain_text_body};

/// What the compose screen is writing.
pub enum ComposeMode {
    New,
    Reply,
    ReplyAll,
}

/// The original a reply is written on.
pub struct ReplySource {
    pub thread_id: String,
    pub message_id: String,
    /// The reader's offset from UTC at the original's date, in minutes, so
    /// the quote header shows the time the reader saw.
    pub utc_offset_minutes: i32,
}

/// A compose screen filled in before the user types.
pub struct ComposeDraft {
    pub account_id: String,
    pub from_email: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    /// The quoted original as a full document for the preview under the
    /// editor; empty for a new message.
    pub quote_preview_html: String,
}

/// A mail ready to leave.
pub struct OutgoingMail {
    pub mode: ComposeMode,
    pub account_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    /// Set for a reply.
    pub reply_source: Option<ReplySource>,
}

/// The quoted original of a reply.
struct Quote {
    html: String,
    text: String,
    /// Inline images the quoted body shows, by content id.
    images: Vec<(String, Vec<u8>)>,
}

struct Original {
    message: ThreadDetailMessage,
    quote: Quote,
    own_emails: Vec<String>,
}

fn load_original(
    data_dir: &Path,
    account_id: &str,
    source: &ReplySource,
) -> Result<Original, String> {
    let conn = open_read_only(&data_dir.join(db::db::DB_FILE_NAME))?;
    let bodies = open_read_only(&data_dir.join("bodies.db"))?;
    let read = ReadConn::from_raw(&conn);
    let own_emails = get_all_accounts_sync(&read)?
        .into_iter()
        .map(|account| account.email.to_lowercase())
        .collect();
    let detail = get_thread_detail(&read, &bodies, account_id, &source.thread_id)?;
    let message = detail
        .messages
        .into_iter()
        .find(|message| message.id == source.message_id)
        .ok_or_else(|| format!("message {} is not in this conversation", source.message_id))?;

    let cid_hashes = query_inline_cid_hashes(&read, account_id, &source.thread_id)?;
    let images = if cid_hashes.is_empty() {
        HashMap::new()
    } else {
        let store = open_read_only(&data_dir.join("inline_images.db"))?;
        rtsk::inline_image_store::InlineImageStoreReadState::get_batch_sync(&store, &cid_hashes)?
    };
    let quote = build_quote(&message, &images, source.utc_offset_minutes)?;
    Ok(Original {
        message,
        quote,
        own_emails,
    })
}

fn build_quote(
    message: &ThreadDetailMessage,
    images: &HashMap<String, Vec<u8>>,
    utc_offset_minutes: i32,
) -> Result<Quote, String> {
    let (body_html, body_text) = match (&message.body_html, &message.body_text) {
        (Some(html), text) if !html.trim().is_empty() => (
            sanitize_html_body(html),
            text.clone()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| html_to_plain_text(html)),
        ),
        (_, Some(text)) => (plain_text_body(text), text.clone()),
        _ => (String::new(), String::new()),
    };
    let quoted_images: Vec<(String, Vec<u8>)> = images
        .iter()
        .map(|(cid, bytes)| {
            (
                cid.trim_start_matches('<').trim_end_matches('>').to_owned(),
                bytes,
            )
        })
        .filter(|(cid, _)| body_html.contains(&format!("cid:{cid}")))
        .map(|(cid, bytes)| (cid, bytes.clone()))
        .collect();

    let from = match (&message.from_name, &message.from_address) {
        (Some(name), Some(email)) if !name.trim().is_empty() => format!("{name} <{email}>"),
        (_, Some(email)) => email.clone(),
        (Some(name), None) => name.clone(),
        (None, None) => String::new(),
    };
    let mut fields = vec![
        ("De :", from),
        ("Envoyé :", quote_date(message.date, utc_offset_minutes)?),
    ];
    if let Some(to) = message
        .to_addresses
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        fields.push(("À :", to.clone()));
    }
    if let Some(cc) = message
        .cc_addresses
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        fields.push(("Cc :", cc.clone()));
    }
    fields.push(("Objet :", message.subject.clone().unwrap_or_default()));

    let header_html = fields
        .iter()
        .map(|(label, value)| format!("<b>{label}</b> {}", escape(value)))
        .collect::<Vec<_>>()
        .join("<br>");
    let header_text = fields
        .iter()
        .map(|(label, value)| format!("{label} {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Quote {
        html: format!(
            "<hr style=\"display:inline-block;width:98%\" tabindex=\"-1\">\
             <div id=\"divRplyFwdMsg\" dir=\"ltr\">\
             <font face=\"Calibri, sans-serif\" style=\"font-size:11pt\" color=\"#000000\">\
             {header_html}</font><div>&nbsp;</div></div><div>{body_html}</div>"
        ),
        text: format!("________________________________\n{header_text}\n\n{body_text}"),
        images: quoted_images,
    })
}

/// "jeudi 26 novembre 2026 18:40", as Outlook writes it in French.
fn quote_date(date_ms: i64, utc_offset_minutes: i32) -> Result<String, String> {
    const DAYS: [&str; 7] = [
        "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
    ];
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    let offset = jiff::tz::Offset::from_seconds(utc_offset_minutes * 60)
        .map_err(|e| format!("time zone offset: {e}"))?;
    let local = jiff::Timestamp::from_millisecond(date_ms)
        .map_err(|e| format!("message date: {e}"))?
        .to_zoned(jiff::tz::TimeZone::fixed(offset));
    let day = DAYS[usize::from(local.weekday().to_monday_zero_offset().unsigned_abs())];
    let month = MONTHS[usize::from(local.month().unsigned_abs()) - 1];
    Ok(format!(
        "{day} {} {month} {} {:02}:{:02}",
        local.day(),
        local.year(),
        local.hour(),
        local.minute()
    ))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One subject mark at the start: "Re:", "RE :", "TR:", "Fwd:", "Re[2]:".
fn strip_one_mark(subject: &str) -> Option<&str> {
    let trimmed = subject.trim_start();
    let word_end = trimmed
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(trimmed.len());
    let word = trimmed[..word_end].to_ascii_lowercase();
    if !matches!(word.as_str(), "re" | "fw" | "fwd" | "tr" | "aw" | "wg") {
        return None;
    }
    let mut rest = &trimmed[word_end..];
    if let Some(counted) = rest.strip_prefix('[').or_else(|| rest.strip_prefix('(')) {
        let digits = counted
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(counted.len());
        let close = counted[digits..].chars().next()?;
        if digits == 0 || !matches!(close, ']' | ')') {
            return None;
        }
        rest = &counted[digits + 1..];
    }
    rest.trim_start().strip_prefix(':').map(str::trim_start)
}

/// "Re: " and the subject without the marks earlier replies piled up.
fn reply_subject(subject: &str) -> String {
    let mut bare = subject.trim();
    while let Some(rest) = strip_one_mark(bare) {
        bare = rest;
    }
    format!("Re: {bare}")
}

/// The desktop's reply recipients: the sender, or the original recipients
/// when we wrote the original; everyone for a reply to all; never ourselves,
/// never twice; ourselves only when nobody else is left.
fn reply_recipients(
    message: &ThreadDetailMessage,
    own_emails: &[String],
    reply_all: bool,
) -> (Vec<String>, Vec<String>) {
    let is_own = |email: &str| own_emails.contains(&email.to_lowercase());
    let sender: Vec<String> = message.from_address.iter().cloned().collect();
    let addresses = |raw: &Option<String>| -> Vec<String> {
        parse_address_list(raw.as_deref().unwrap_or_default())
            .into_iter()
            .map(|(_, email)| email)
            .collect()
    };
    let original_to = addresses(&message.to_addresses);
    let original_cc = addresses(&message.cc_addresses);
    let from_self = message.from_address.as_deref().is_some_and(is_own);
    let (to, cc) = match (reply_all, from_self) {
        (false, true) => (original_to, Vec::new()),
        (false, false) => (sender.clone(), Vec::new()),
        (true, true) => (original_to, original_cc),
        (true, false) => (
            sender.iter().cloned().chain(original_to).collect(),
            original_cc,
        ),
    };
    let mut seen: Vec<String> = Vec::new();
    let mut keep = |list: Vec<String>| -> Vec<String> {
        list.into_iter()
            .filter(|email| {
                let key = email.to_lowercase();
                if is_own(email) || seen.contains(&key) {
                    return false;
                }
                seen.push(key);
                true
            })
            .collect()
    };
    let to = keep(to);
    let cc = keep(cc);
    if to.is_empty() && cc.is_empty() {
        return (sender, Vec::new());
    }
    (to, cc)
}

fn account_email(data_dir: &Path, account_id: &str) -> Result<(String, Option<String>), String> {
    let conn = open_read_only(&data_dir.join(db::db::DB_FILE_NAME))?;
    get_all_accounts_sync(&ReadConn::from_raw(&conn))?
        .into_iter()
        .find(|account| account.id == account_id)
        .map(|account| (account.email, account.display_name))
        .ok_or_else(|| format!("account {account_id} does not exist"))
}

/// Fill a compose screen: empty for a new message, addressed and quoted for
/// a reply.
pub fn compose_draft(
    data_dir: String,
    mode: ComposeMode,
    account_id: String,
    reply_source: Option<ReplySource>,
) -> Result<ComposeDraft, String> {
    let dir = Path::new(&data_dir);
    let (from_email, _) = account_email(dir, &account_id)?;
    let reply_all = match mode {
        ComposeMode::New => {
            return Ok(ComposeDraft {
                account_id,
                from_email,
                to: Vec::new(),
                cc: Vec::new(),
                subject: String::new(),
                quote_preview_html: String::new(),
            });
        }
        ComposeMode::Reply => false,
        ComposeMode::ReplyAll => true,
    };
    let source = reply_source.ok_or("a reply needs the message it answers")?;
    let original = load_original(dir, &account_id, &source)?;
    let (to, cc) = reply_recipients(&original.message, &original.own_emails, reply_all);
    let images: HashMap<String, Vec<u8>> = original.quote.images.into_iter().collect();
    Ok(ComposeDraft {
        account_id,
        from_email,
        to,
        cc,
        subject: reply_subject(original.message.subject.as_deref().unwrap_or_default()),
        quote_preview_html: super::thread::html_document(
            &embed_inline_images(&original.quote.html, &images),
            "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:",
        ),
    })
}

/// Hand the mail to the engine, which journals it and sends it, retrying
/// while the network is down.
pub fn send_mail(data_dir: String, mail: OutgoingMail) -> Result<(), String> {
    let clean = |list: Vec<String>| -> Vec<String> {
        list.into_iter()
            .map(|email| email.trim().to_owned())
            .filter(|email| !email.is_empty())
            .collect()
    };
    let (to, cc, bcc) = (clean(mail.to), clean(mail.cc), clean(mail.bcc));
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err("add at least one recipient".to_owned());
    }
    let dir = Path::new(&data_dir);
    let (email, display_name) = account_email(dir, &mail.account_id)?;
    let from = match display_name.filter(|name| !name.trim().is_empty()) {
        Some(name) => format!("{name} <{email}>"),
        None => email,
    };

    let mut body_html = plain_text_body(mail.body_text.trim_end());
    let mut body_text = mail.body_text.trim_end().to_owned();
    let mut images = Vec::new();
    let (intent, in_reply_to, thread_id, source_message_id) = match (&mail.mode, mail.reply_source)
    {
        (ComposeMode::New, _) => (SendIntent::New, None, None, None),
        (_, None) => return Err("a reply needs the message it answers".to_owned()),
        (_, Some(source)) => {
            let original = load_original(dir, &mail.account_id, &source)?;
            body_html.push_str("<br>");
            body_html.push_str(&original.quote.html);
            body_text.push_str("\n\n");
            body_text.push_str(&original.quote.text);
            images = original.quote.images;
            (
                SendIntent::Reply,
                original
                    .message
                    .message_id_header
                    .clone()
                    .or_else(|| Some(original.message.id.clone())),
                Some(source.thread_id),
                Some(source.message_id),
            )
        }
    };

    let send_id = PlanId::new_v7();
    let attachments = stage_inline_images(dir, send_id, &images)?;
    let request = SendWireRequest {
        send_id,
        from_account_id: mail.account_id,
        message: SendWireMessage {
            draft_id: PlanId::new_v7().to_string(),
            from,
            to,
            cc,
            bcc,
            subject: Some(mail.subject).filter(|subject| !subject.trim().is_empty()),
            body_html,
            body_text,
            references: in_reply_to.clone(),
            in_reply_to,
            thread_id,
            source_message_id,
            intent,
            scheduled_at: None,
            request_read_receipt: false,
        },
        attachments,
        scheduled_at: None,
    };
    let ack: SendAck = super::engine::call(RequestParams::ActionSend {
        request: Box::new(request),
    })?;
    if !ack.journaled {
        return Err("the engine refused the mail".to_owned());
    }
    Ok(())
}

/// Write each quoted inline image where the engine takes attachments from,
/// `<data>/staging/<send id>/<index>.bin`, as the desktop does.
fn stage_inline_images(
    data_dir: &Path,
    send_id: PlanId,
    images: &[(String, Vec<u8>)],
) -> Result<Vec<SendWireAttachment>, String> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let staging = data_dir.join("staging").join(send_id.to_string());
    std::fs::create_dir_all(&staging).map_err(|e| format!("create {}: {e}", staging.display()))?;
    images
        .iter()
        .enumerate()
        .map(|(index, (cid, bytes))| {
            let relative_path = format!("{index}.bin");
            std::fs::write(staging.join(&relative_path), bytes)
                .map_err(|e| format!("stage inline image: {e}"))?;
            let mime = image_mime_type(bytes);
            let extension = mime.rsplit('/').next().unwrap_or("bin");
            Ok(SendWireAttachment {
                source: SendAttachmentSource::StagingFile {
                    relative_path,
                    content_hash: *rtsk::blob_hash::BlobHash::hash(bytes).as_bytes(),
                },
                size: bytes.len() as u64,
                mime: mime.to_owned(),
                filename: format!("image{index}.{extension}"),
                content_id: Some(cid.clone()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(from: &str, to: &str, cc: Option<&str>) -> ThreadDetailMessage {
        ThreadDetailMessage {
            id: "m1".into(),
            thread_id: "t1".into(),
            account_id: "a1".into(),
            from_address: Some(from.into()),
            from_name: Some("Sender".into()),
            to_addresses: Some(to.into()),
            cc_addresses: cc.map(Into::into),
            bcc_addresses: None,
            date: 1_764_178_800_000,
            subject: Some("RE: TR: Re[2]: Facture".into()),
            message_id_header: Some("<abc@mail>".into()),
            body_html: Some("<p>Bonjour <img src=\"cid:logo@x\"></p>".into()),
            body_text: None,
            is_read: true,
            is_starred: false,
            is_replied: false,
            is_forwarded: false,
            has_meeting_invite: false,
            meeting_invite_method: None,
            meeting_invite_uid: None,
            is_own_message: false,
            collapsed_summary: None,
        }
    }

    #[test]
    fn the_subject_keeps_one_reply_mark() {
        assert_eq!(reply_subject("RE: TR: Re[2]: Facture"), "Re: Facture");
        assert_eq!(reply_subject("Review: le plan"), "Re: Review: le plan");
        assert_eq!(reply_subject(""), "Re: ");
    }

    #[test]
    fn a_reply_goes_to_the_sender_and_a_reply_all_to_everyone_but_us() {
        let own = vec!["me@x.com".to_owned()];
        let original = message(
            "boss@x.com",
            "me@x.com, \"Doe, Jane\" <jane@x.com>",
            Some("bob@x.com"),
        );
        assert_eq!(
            reply_recipients(&original, &own, false),
            (vec!["boss@x.com".to_owned()], Vec::new())
        );
        assert_eq!(
            reply_recipients(&original, &own, true),
            (
                vec!["boss@x.com".to_owned(), "jane@x.com".to_owned()],
                vec!["bob@x.com".to_owned()]
            )
        );
    }

    #[test]
    fn replying_to_our_own_message_goes_to_its_recipients() {
        let own = vec!["me@x.com".to_owned()];
        let original = message("me@x.com", "client@y.com", None);
        assert_eq!(
            reply_recipients(&original, &own, false),
            (vec!["client@y.com".to_owned()], Vec::new())
        );
    }

    #[test]
    fn the_quote_has_an_outlook_header_and_carries_the_images_it_shows() {
        let images = HashMap::from([
            ("<logo@x>".to_owned(), vec![0x89, b'P', b'N', b'G']),
            ("unused@x".to_owned(), vec![0xFF, 0xD8, 0xFF]),
        ]);
        let quote =
            build_quote(&message("boss@x.com", "me@x.com", None), &images, 60).expect("quote");
        assert!(quote.html.contains("<b>De :</b> Sender &lt;boss@x.com&gt;"));
        assert!(
            quote
                .text
                .contains("Envoyé : mercredi 26 novembre 2025 18:40")
        );
        assert_eq!(quote.images.len(), 1);
        assert_eq!(quote.images[0].0, "logo@x");
    }

    #[test]
    fn inline_images_are_staged_with_their_content_id() {
        let dir = tempfile::tempdir().expect("temp dir");
        let send_id = PlanId::new_v7();
        let staged = stage_inline_images(
            dir.path(),
            send_id,
            &[("logo@x".to_owned(), vec![0x89, b'P', b'N', b'G'])],
        )
        .expect("staged");
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].content_id.as_deref(), Some("logo@x"));
        assert_eq!(staged[0].mime, "image/png");
        assert!(
            dir.path()
                .join("staging")
                .join(send_id.to_string())
                .join("0.bin")
                .is_file()
        );
    }
}
