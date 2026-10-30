//! HTML carried below the editor, laid out like Outlook: the signature,
//! then for a reply or forward a rule, a From / Sent / To / Cc / Subject
//! block and the original body. Both stay out of the editor, which has no
//! images or tables, so they reach the recipient untouched.

use crate::ui::mail_body::HtmlBodyState;

/// Separates the editable part from the quoted original in a saved draft.
/// Outlook uses the same element, so a draft opened there keeps the split.
const QUOTE_MARKER: &str = r#"<div id="appendonsend"></div>"#;
const SIGNATURE_MARKER: &str = "<!--ratatoskr-signature-->";

const TEXT_RULE: &str = "________________________________";

/// The message a reply or forward is built on.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplySource<'a> {
    pub from_name: Option<&'a str>,
    pub from_address: Option<&'a str>,
    pub to_addresses: Option<&'a str>,
    pub cc_addresses: Option<&'a str>,
    pub subject: Option<&'a str>,
    /// Unix milliseconds.
    pub date: Option<i64>,
    pub body_html: Option<&'a str>,
    pub body_text: Option<&'a str>,
    pub snippet: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub message_id: Option<&'a str>,
    pub message_id_header: Option<&'a str>,
}

impl<'a> ReplySource<'a> {
    pub fn from_thread_message(
        message: &'a crate::db::ThreadMessage,
        thread_id: Option<&'a str>,
    ) -> Self {
        Self {
            from_name: message.from_name.as_deref(),
            from_address: message.from_address.as_deref(),
            to_addresses: message.to_addresses.as_deref(),
            cc_addresses: message.cc_addresses.as_deref(),
            subject: message.subject.as_deref(),
            date: message.date,
            body_html: message.body_html.as_deref(),
            body_text: message.body_text.as_deref(),
            snippet: message.snippet.as_deref(),
            thread_id,
            message_id: Some(&message.id),
            message_id_header: message.message_id_header.as_deref(),
        }
    }
}

/// Which read-only block of a compose window a preview message is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposePreview {
    Signature,
    Quote,
}

/// Read-only HTML shown under the editor and appended when sending.
#[derive(Debug, Clone)]
pub struct HtmlBlock {
    pub html: String,
    /// Plain-text equivalent for the `text/plain` part.
    pub text: String,
    pub preview: HtmlBodyState,
}

impl HtmlBlock {
    fn new(html: String, text: String) -> Self {
        Self {
            html,
            text,
            preview: HtmlBodyState::default(),
        }
    }

    /// A signature, or `None` when its HTML shows nothing.
    pub fn signature(html: &str) -> Option<Self> {
        let text = html_to_text(html);
        let has_image = html.to_ascii_lowercase().contains("<img");
        (has_image || !text.trim().is_empty()).then(|| Self::new(html.to_string(), text))
    }

    /// The quoted original of a reply or forward, or `None` when the source
    /// has no body at all.
    pub fn quote(source: &ReplySource<'_>) -> Option<Self> {
        let body_html = match (source.body_html, source.body_text, source.snippet) {
            (Some(html), _, _) if !html.trim().is_empty() => embeddable_body(html),
            (_, Some(text), _) if !text.trim().is_empty() => text_to_html(text),
            (_, _, Some(snippet)) if !snippet.trim().is_empty() => text_to_html(snippet),
            _ => return None,
        };
        let body_text = match (source.body_text, source.body_html) {
            (Some(text), _) if !text.trim().is_empty() => text.to_string(),
            (_, Some(html)) => html_to_text(html),
            _ => source.snippet.unwrap_or_default().to_string(),
        };

        let fields = header_fields(source);
        let header_html = fields
            .iter()
            .map(|(label, value)| format!("<b>{label}</b> {}", escape(value)))
            .collect::<Vec<_>>()
            .join("<br>");
        let html = format!(
            "<hr style=\"display:inline-block;width:98%\" tabindex=\"-1\">\
             <div id=\"divRplyFwdMsg\" dir=\"ltr\">\
             <font face=\"Calibri, sans-serif\" style=\"font-size:11pt\" color=\"#000000\">\
             {header_html}</font><div>&nbsp;</div></div>\
             <div>{body_html}</div>"
        );

        let header_text = fields
            .iter()
            .map(|(label, value)| format!("{label} {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!("{TEXT_RULE}\n{header_text}\n\n{body_text}");

        Some(Self::new(html, text))
    }
}

/// HTML and plain text sent to the recipient: the editor content, the
/// signature, then the quoted original.
pub fn outgoing_body(
    editor_html: &str,
    editor_text: &str,
    signature: Option<&HtmlBlock>,
    quote: Option<&HtmlBlock>,
) -> (String, String) {
    let mut html = editor_html.to_string();
    let mut text = editor_text.trim_end().to_string();
    if let Some(signature) = signature {
        html.push_str(&format!("<div id=\"Signature\">{}</div>", signature.html));
        text.push_str("\n\n");
        text.push_str(signature.text.trim_end());
    }
    if let Some(quote) = quote {
        html.push_str("<br>");
        html.push_str(&quote.html);
        text.push_str("\n\n");
        text.push_str(&quote.text);
    }
    (html, text)
}

/// HTML stored in a draft, with markers so it can be split again.
pub fn draft_html(
    editor_html: &str,
    signature: Option<&HtmlBlock>,
    quote: Option<&HtmlBlock>,
) -> String {
    let mut html = editor_html.to_string();
    if let Some(signature) = signature {
        html.push_str(SIGNATURE_MARKER);
        html.push_str(&signature.html);
    }
    if let Some(quote) = quote {
        html.push_str(QUOTE_MARKER);
        html.push_str(&quote.html);
    }
    html
}

/// A saved draft split back into its parts.
pub struct DraftParts<'a> {
    pub editor_html: &'a str,
    pub signature: Option<HtmlBlock>,
    pub quote: Option<HtmlBlock>,
}

pub fn split_draft_html(html: &str) -> DraftParts<'_> {
    let (rest, quote) = match html.split_once(QUOTE_MARKER) {
        Some((rest, quoted)) => {
            let quote = (!quoted.trim().is_empty())
                .then(|| HtmlBlock::new(quoted.to_string(), html_to_text(quoted)));
            (rest, quote)
        }
        None => (html, None),
    };
    let (editor_html, signature) = match rest.split_once(SIGNATURE_MARKER) {
        Some((editor, signature)) => (editor, HtmlBlock::signature(signature)),
        None => (rest, None),
    };
    DraftParts {
        editor_html,
        signature,
        quote,
    }
}

fn header_fields(source: &ReplySource<'_>) -> Vec<(String, String)> {
    let labels = crate::i18n::quote_labels();
    let mut fields = Vec::with_capacity(5);
    let from = match (source.from_name, source.from_address) {
        (Some(name), Some(email)) if !name.trim().is_empty() => format!("{name} <{email}>"),
        (_, Some(email)) => email.to_string(),
        (Some(name), None) => name.to_string(),
        (None, None) => String::new(),
    };
    fields.push((labels.from, from));
    if let Some(date) = source.date.and_then(crate::i18n::format_quote_date) {
        fields.push((labels.sent, date));
    }
    if let Some(to) = source.to_addresses.filter(|s| !s.trim().is_empty()) {
        fields.push((labels.to, to.to_string()));
    }
    if let Some(cc) = source.cc_addresses.filter(|s| !s.trim().is_empty()) {
        fields.push((labels.cc, cc.to_string()));
    }
    fields.push((
        labels.subject,
        source.subject.unwrap_or_default().to_string(),
    ));
    fields
}

/// The part of a full HTML document that can be nested in another one:
/// the `<body>` content, preceded by the `<style>` blocks of the head.
pub fn embeddable_body(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(body_open) = lower.find("<body") else {
        return html.to_string();
    };
    let Some(body_start) = lower[body_open..].find('>').map(|i| body_open + i + 1) else {
        return html.to_string();
    };
    let body_end = lower.rfind("</body").filter(|&end| end >= body_start);
    let body = &html[body_start..body_end.unwrap_or(html.len())];

    let mut styles = String::new();
    let mut cursor = 0;
    while let Some(open) = lower[cursor..body_open].find("<style").map(|i| cursor + i) {
        let Some(close) = lower[open..body_open]
            .find("</style>")
            .map(|i| open + i + 8)
        else {
            break;
        };
        styles.push_str(&html[open..close]);
        cursor = close;
    }
    format!("{styles}{body}")
}

fn html_to_text(html: &str) -> String {
    rte::html_parse::from_html(html).flattened_text()
}

fn text_to_html(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                "<div><br></div>".to_string()
            } else {
                format!("<div>{}</div>", escape(line))
            }
        })
        .collect()
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source<'a>(html: Option<&'a str>, text: Option<&'a str>) -> ReplySource<'a> {
        ReplySource {
            from_name: Some("PayByPhone"),
            from_address: Some("noreply@paybyphone.com"),
            to_addresses: Some("antoine@pinelli.fr"),
            subject: Some("Votre recu"),
            date: Some(1_793_624_400_000),
            body_html: html,
            body_text: text,
            ..ReplySource::default()
        }
    }

    #[test]
    fn html_original_is_kept_with_rule_and_header() {
        let quote = HtmlBlock::quote(&source(
            Some("<html><head><style>p{color:red}</style></head><body class=\"x\"><p>Hello</p></body></html>"),
            Some("Votre client de messagerie ne prend pas en charge le format HTML"),
        ))
        .expect("quote");
        assert!(quote.html.starts_with("<hr"));
        assert!(
            quote
                .html
                .contains("PayByPhone &lt;noreply@paybyphone.com&gt;")
        );
        assert!(
            quote
                .html
                .contains("<style>p{color:red}</style><p>Hello</p>")
        );
        assert!(!quote.html.contains("<body"));
        assert!(!quote.html.contains("ne prend pas en charge"));
        assert!(quote.text.starts_with(TEXT_RULE));
    }

    #[test]
    fn text_only_original_is_escaped() {
        let quote = HtmlBlock::quote(&source(None, Some("a < b\n\nfin"))).expect("quote");
        assert!(
            quote
                .html
                .contains("<div>a &lt; b</div><div><br></div><div>fin</div>")
        );
    }

    #[test]
    fn nothing_to_quote_gives_none() {
        assert!(HtmlBlock::quote(&source(None, None)).is_none());
    }

    #[test]
    fn signature_sits_between_text_and_quote() {
        let signature = HtmlBlock::signature("<p>Antoine</p><img src=\"https://x/logo.png\">");
        let quote = HtmlBlock::quote(&source(Some("<p>Hi</p>"), None));
        let (html, text) =
            outgoing_body("<p>Reply</p>", "Reply", signature.as_ref(), quote.as_ref());
        let reply = html.find("Reply").expect("reply");
        let sig = html.find("Antoine").expect("signature");
        let rule = html.find("<hr").expect("rule");
        assert!(reply < sig && sig < rule);
        assert!(!html.contains("appendonsend") && !html.contains(SIGNATURE_MARKER));
        assert!(text.starts_with("Reply\n\nAntoine"));
    }

    #[test]
    fn image_only_signature_is_kept_and_blank_one_dropped() {
        assert!(HtmlBlock::signature("<img src=\"cid:logo\">").is_some());
        assert!(HtmlBlock::signature("<p> </p>").is_none());
    }

    #[test]
    fn draft_round_trip_restores_every_part() {
        let signature = HtmlBlock::signature("<p>Antoine</p>");
        let quote = HtmlBlock::quote(&source(Some("<p>Hi</p>"), None));
        let saved = draft_html("<p>Reply</p>", signature.as_ref(), quote.as_ref());
        let parts = split_draft_html(&saved);
        assert_eq!(parts.editor_html, "<p>Reply</p>");
        assert_eq!(parts.signature.expect("signature").html, "<p>Antoine</p>");
        assert_eq!(parts.quote.expect("quote").html, quote.expect("quote").html);
    }

    #[test]
    fn draft_without_markers_is_all_editor() {
        let parts = split_draft_html("<p>Only</p>");
        assert_eq!(parts.editor_html, "<p>Only</p>");
        assert!(parts.signature.is_none() && parts.quote.is_none());
    }
}
