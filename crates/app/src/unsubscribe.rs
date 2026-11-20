//! Leaving a mailing list from the reading pane.
//!
//! A message offers one of three ways out, tried from the most automatic:
//!
//! 1. RFC 8058 one-click: a `List-Unsubscribe` HTTPS URI with a
//!    `List-Unsubscribe-Post: List-Unsubscribe=One-Click` header. A POST
//!    does it, nothing opens. Bulk senders to Gmail and Yahoo must offer it.
//! 2. RFC 2369 `mailto:`: a short message to the list does it, sent from the
//!    account that received the newsletter.
//! 3. A web page where the reader would confirm: the `List-Unsubscribe`
//!    HTTP(S) URI without one-click, or failing any header, the unsubscribe
//!    link of the body footer. Nothing opens: the page is fetched in the
//!    background, its unsubscribe form submitted (or its confirm link
//!    followed), and the answer read for a confirmation.
//!
//! Acting on a list header tells the sender the address is read, so a
//! message whose DKIM check failed gets no offer at all. The footer link is
//! a guess from the body text, so it needs a DKIM pass.

use mail_html::Url;

/// How a message lets its reader leave the list it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnsubscribeMethod {
    /// RFC 8058 one-click: an HTTPS POST does it.
    OneClick { url: String },
    /// RFC 2369 mailto: this message, sent to the list, does it.
    Mail {
        to: String,
        subject: String,
        body: String,
    },
    /// A page where the reader would confirm, handled in the background.
    Page { url: String },
}

/// Where leaving a list stands for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsubscribeStatus {
    Ready,
    Working,
    Done,
    /// The unsubscribe page was submitted but did not say it worked.
    Requested,
    Failed(String),
}

/// The headers and body a message offers its way out with.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ListHeaders<'a> {
    pub list_unsubscribe: Option<&'a str>,
    pub list_unsubscribe_post: Option<&'a str>,
    pub auth_results: Option<&'a str>,
    pub body_html: Option<&'a str>,
}

/// Default subject and body of the `mailto:` message when the URI sets none.
const MAIL_DEFAULT_TEXT: &str = "unsubscribe";

/// Link texts and addresses that mark a footer unsubscribe link.
const FOOTER_KEYWORDS: &[&str] = &[
    "unsubscribe",
    "désabonner",
    "desabonner",
    "désabonnement",
    "desabonnement",
    "désinscri",
    "desinscri",
    "opt-out",
    "opt out",
    "abmelden",
    "darse de baja",
    "cancelar suscripción",
    "cancelar suscripcion",
    "annulla iscrizione",
    "uitschrijven",
];

/// Words in a subject or an attachment name that mark a message the reader
/// needs: a bill, a payment or delivery notice, an account or security
/// alert. Senders put a `List-Unsubscribe` header on these too, and leaving
/// that "list" stops the reminders.
const TRANSACTIONAL_KEYWORDS: &[&str] = &[
    // Billing and payment
    "factur",
    "invoice",
    "paiement",
    "payment",
    "prélev",
    "prélèv",
    "prelev",
    "impayé",
    "unpaid",
    "overdue",
    "échéance",
    "echeance",
    "reçu",
    "receipt",
    "relevé",
    "statement",
    "rembours",
    "refund",
    "renouvel",
    "renewal",
    "carte",
    // Orders, deliveries, bookings
    "commande",
    "order",
    "livraison",
    "delivery",
    "expédi",
    "shipping",
    "colis",
    "réservation",
    "reservation",
    "booking",
    "billet",
    "ticket",
    // Account and security
    "mot de passe",
    "password",
    "vérification",
    "verification",
    "sécurité",
    "security",
    "connexion",
    "sign-in",
    "login",
    // Urgency
    "important",
    "rappel",
    "reminder",
    "action requise",
    "action required",
    "confirm",
];

/// Whether a message reads as one the reader needs rather than a
/// newsletter, from its subject and the names of its attachments. Such a
/// message gets no unsubscribe offer, whatever its headers say: when in
/// doubt the offer is withheld, and the sender's own footer link remains.
pub(crate) fn is_transactional<'a>(
    subject: Option<&'a str>,
    attachment_names: impl IntoIterator<Item = &'a str>,
) -> bool {
    subject
        .into_iter()
        .chain(attachment_names)
        .map(str::to_lowercase)
        .any(|text| {
            TRANSACTIONAL_KEYWORDS
                .iter()
                .any(|word| text.contains(word))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dkim {
    Pass,
    Fail,
    Unknown,
}

/// The way out of the list this message came from, if it offers one.
pub(crate) fn method_for(headers: ListHeaders<'_>) -> Option<UnsubscribeMethod> {
    let dkim = dkim_verdict(headers.auth_results);
    if dkim == Dkim::Fail {
        return None;
    }
    let uris = headers
        .list_unsubscribe
        .map(header_uris)
        .unwrap_or_default();
    let one_click = headers.list_unsubscribe_post.is_some_and(is_one_click_post);

    if one_click && let Some(url) = uris.iter().find_map(|uri| public_web_url(uri, true)) {
        return Some(UnsubscribeMethod::OneClick { url });
    }
    if let Some(mail) = uris.iter().map(String::as_str).find_map(mail_method) {
        return Some(mail);
    }
    if let Some(url) = uris.iter().find_map(|uri| public_web_url(uri, false)) {
        return Some(UnsubscribeMethod::Page { url });
    }
    if dkim == Dkim::Pass
        && let Some(url) = headers.body_html.and_then(footer_link)
    {
        return Some(UnsubscribeMethod::Page { url });
    }
    None
}

/// Send the RFC 8058 one-click POST. Any 2xx answer means done.
pub(crate) async fn post_one_click(url: String) -> Result<UnsubscribeStatus, String> {
    let client = crate::ui::mail_body::public_http_client()
        .ok_or_else(|| "no HTTP client available".to_string())?;
    let response = client
        .post(&url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body("List-Unsubscribe=One-Click")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if status.is_success() {
        Ok(UnsubscribeStatus::Done)
    } else {
        Err(format!("HTTP {status}"))
    }
}

/// Largest page body read, so a hostile sender cannot fill memory.
const PAGE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Visible text that says a page took the address off the list.
const CONFIRMATION_PHRASES: &[&str] = &[
    "unsubscribed",
    "been removed",
    "successfully removed",
    "no longer receive",
    "opted out",
    "désabonné",
    "desabonne",
    "désinscrit",
    "desinscrit",
    "ne recevrez plus",
    "abgemeldet",
    "dado de baja",
    "uitgeschreven",
    "disiscritt",
];

/// Labels of links and buttons that confirm leaving, beyond the footer words.
const CONFIRM_KEYWORDS: &[&str] = &["confirm", "valider", "bestätigen", "bevestig"];

/// One fetched page: where it ended up, its HTML and its session cookies.
struct Page {
    url: Url,
    html: String,
    cookies: String,
}

/// Do what a person would do on the unsubscribe page, without opening it:
/// fetch it, submit its unsubscribe form (or follow its confirm link), and
/// read the answer. `Done` when a page says it worked, `Requested` when it
/// was submitted without saying so. `email` fills an empty address field.
pub(crate) async fn visit_page(
    url: String,
    email: Option<String>,
) -> Result<UnsubscribeStatus, String> {
    let client = crate::ui::mail_body::public_http_client()
        .ok_or_else(|| "no HTTP client available".to_string())?;
    let page = fetch(client.get(&url)).await?;

    if let Some(form) = unsubscribe_form(&page.html) {
        let fields = form_fields(form.inner, email.as_deref());
        let target = form
            .action
            .as_deref()
            .map(str::trim)
            .filter(|action| !action.is_empty())
            .map_or(Ok(page.url.clone()), |action| page.url.join(action))
            .map_err(|error| error.to_string())?;
        if !crate::ui::mail_body::is_public_url(&target) {
            return Err("the unsubscribe form posts to a private address".into());
        }
        // The form is the sender's: left free to post anywhere, it would
        // have this machine submit the fields it chose, and the user's
        // address, to a site that has nothing to do with the list.
        if target.host_str() != page.url.host_str() {
            return Err("the unsubscribe form posts to another site".into());
        }
        let request = if form.post {
            client.post(target).form(&fields)
        } else {
            let mut target = target;
            target.query_pairs_mut().extend_pairs(&fields);
            client.get(target)
        };
        let answer = fetch(with_session(request, &page)).await?;
        return Ok(verdict(&answer.html));
    }
    if confirms(&page.html) {
        return Ok(UnsubscribeStatus::Done);
    }
    let confirm_link = page_links(&page.html)
        .into_iter()
        .find_map(|(href, label)| {
            let target = page.url.join(href.trim()).ok()?;
            let marked =
                has_keyword(&label, FOOTER_KEYWORDS) || has_keyword(&label, CONFIRM_KEYWORDS);
            (marked
                && target != page.url
                && target.host_str() == page.url.host_str()
                && crate::ui::mail_body::is_public_url(&target))
            .then_some(target)
        });
    match confirm_link {
        Some(target) => {
            let answer = fetch(with_session(client.get(target), &page)).await?;
            Ok(verdict(&answer.html))
        }
        // Visiting the link is the unsubscribe at many senders.
        None => Ok(UnsubscribeStatus::Requested),
    }
}

async fn fetch(request: reqwest::RequestBuilder) -> Result<Page, String> {
    let mut response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let url = response.url().clone();
    let cookies = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|cookie| cookie.split(';').next())
        .map(str::trim)
        .filter(|pair| pair.contains('='))
        .collect::<Vec<_>>()
        .join("; ");
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > PAGE_MAX_BYTES {
            body.truncate(PAGE_MAX_BYTES);
            break;
        }
    }
    Ok(Page {
        url,
        html: String::from_utf8_lossy(&body).into_owned(),
        cookies,
    })
}

/// Send back what a browser would: the page's cookies and where it was.
fn with_session(request: reqwest::RequestBuilder, page: &Page) -> reqwest::RequestBuilder {
    let request = request.header(reqwest::header::REFERER, page.url.as_str());
    if page.cookies.is_empty() {
        request
    } else {
        request.header(reqwest::header::COOKIE, page.cookies.as_str())
    }
}

fn verdict(html: &str) -> UnsubscribeStatus {
    if confirms(html) {
        UnsubscribeStatus::Done
    } else {
        UnsubscribeStatus::Requested
    }
}

/// Whether the visible text of a page says the address left the list.
fn confirms(html: &str) -> bool {
    let text = decode_entities(&strip_tags(&without_blocks(html))).to_lowercase();
    CONFIRMATION_PHRASES
        .iter()
        .any(|phrase| text.contains(phrase))
}

/// The HTML without its `<script>` and `<style>` contents, which are not text.
fn without_blocks(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut kept = String::with_capacity(html.len());
    let mut cursor = 0;
    loop {
        let next = ["<script", "<style"]
            .iter()
            .filter_map(|open| lower[cursor..].find(open).map(|at| (cursor + at, *open)))
            .min_by_key(|(at, _)| *at);
        let Some((start, open)) = next else {
            kept.push_str(&html[cursor..]);
            return kept;
        };
        kept.push_str(&html[cursor..start]);
        let close = format!("</{}", &open[1..]);
        match lower[start..].find(&close) {
            Some(at) => cursor = start + at + close.len(),
            None => return kept,
        }
    }
}

fn has_keyword(text: &str, keywords: &[&str]) -> bool {
    let text = text.to_lowercase();
    keywords.iter().any(|keyword| text.contains(keyword))
}

/// A form of a page: how it submits and what it holds.
struct Form<'a> {
    action: Option<String>,
    post: bool,
    inner: &'a str,
}

/// The first form of the page about leaving the list: its text, action or
/// buttons carry an unsubscribe word.
fn unsubscribe_form(html: &str) -> Option<Form<'_>> {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(start) = find_tag(&lower, "form", cursor) {
        let tag_end = lower[start..].find('>').map(|at| start + at)?;
        let close = lower[tag_end..]
            .find("</form")
            .map_or(lower.len(), |at| tag_end + at);
        cursor = close;
        let tag = &html[start + "<form".len()..tag_end];
        let inner = &html[tag_end + 1..close];
        let action = attribute(tag, "action").map(|action| decode_entities(&action));
        let labels = submit_labels(inner).join(" ");
        let text = decode_entities(&strip_tags(inner));
        let marked = has_keyword(&text, FOOTER_KEYWORDS)
            || has_keyword(&labels, FOOTER_KEYWORDS)
            || action
                .as_deref()
                .is_some_and(|action| has_keyword(action, FOOTER_KEYWORDS));
        if marked {
            let post = attribute(tag, "method").is_some_and(|m| m.eq_ignore_ascii_case("post"));
            return Some(Form {
                action,
                post,
                inner,
            });
        }
    }
    None
}

/// Where the next `<name` opening tag starts, at or after `from`.
fn find_tag(lower: &str, name: &str, from: usize) -> Option<usize> {
    let open = format!("<{name}");
    let mut cursor = from;
    while let Some(start) = lower[cursor..].find(&open).map(|at| cursor + at) {
        cursor = start + open.len();
        if lower[cursor..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_whitespace() || c == '>' || c == '/')
        {
            return Some(start);
        }
    }
    None
}

/// One control of a form, as the page wrote it.
struct Control {
    kind: String,
    name: Option<String>,
    value: Option<String>,
    checked: bool,
    disabled: bool,
    label: String,
}

/// The `<input>`, `<button>`, `<select>` and `<textarea>` of a form, in order.
fn form_controls(inner: &str) -> Vec<Control> {
    let lower = inner.to_ascii_lowercase();
    let mut controls = Vec::new();
    let mut cursor = 0;
    loop {
        let next = ["input", "button", "select", "textarea"]
            .iter()
            .filter_map(|name| find_tag(&lower, name, cursor).map(|at| (at, *name)))
            .min_by_key(|(at, _)| *at);
        let Some((start, element)) = next else {
            return controls;
        };
        let Some(tag_end) = lower[start..].find('>').map(|at| start + at) else {
            return controls;
        };
        let tag = &inner[start + 1 + element.len()..tag_end];
        let tag_lower = &lower[start + 1 + element.len()..tag_end];
        let close_tag = format!("</{element}");
        let (body, after) = if element == "input" {
            ("", tag_end + 1)
        } else {
            let close = lower[tag_end..]
                .find(&close_tag)
                .map_or(lower.len(), |at| tag_end + at);
            (&inner[tag_end + 1..close], close)
        };
        cursor = after;
        let flag = |name: &str| {
            tag_lower
                .split(|c: char| c.is_ascii_whitespace() || c == '/')
                .any(|token| token == name || token.starts_with(&format!("{name}=")))
        };
        let attr = |name: &str| attribute(tag, name).map(|value| decode_entities(&value));
        let kind = match element {
            "input" => attr("type").map_or_else(|| "text".to_string(), |t| t.to_ascii_lowercase()),
            "button" => {
                attr("type").map_or_else(|| "submit".to_string(), |t| t.to_ascii_lowercase())
            }
            other => other.to_string(),
        };
        let value = match element {
            "select" => selected_option(body),
            "textarea" => Some(decode_entities(body)),
            _ => attr("value"),
        };
        let label = match element {
            "button" => decode_entities(&strip_tags(body)),
            _ => value.clone().unwrap_or_default(),
        };
        controls.push(Control {
            kind,
            name: attr("name"),
            value,
            checked: flag("checked"),
            disabled: flag("disabled"),
            label,
        });
    }
}

fn is_submit(control: &Control) -> bool {
    matches!(control.kind.as_str(), "submit" | "image")
}

/// The labels of a form's submit buttons.
fn submit_labels(inner: &str) -> Vec<String> {
    form_controls(inner)
        .into_iter()
        .filter(is_submit)
        .map(|control| control.label)
        .collect()
}

/// The value a `<select>` sends: its selected option, else its first one.
fn selected_option(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let mut first = None;
    let mut cursor = 0;
    while let Some(start) = find_tag(&lower, "option", cursor) {
        let tag_end = lower[start..].find('>').map(|at| start + at)?;
        cursor = tag_end;
        let tag = &body[start + "<option".len()..tag_end];
        let text_end = lower[tag_end..]
            .find('<')
            .map_or(lower.len(), |at| tag_end + at);
        let value = attribute(tag, "value")
            .unwrap_or_else(|| body[tag_end + 1..text_end].trim().to_string());
        let value = decode_entities(&value);
        if lower[start..tag_end].contains(" selected") {
            return Some(value);
        }
        first.get_or_insert(value);
    }
    first
}

/// The pairs a browser would send when the reader presses the form's
/// unsubscribe button: hidden and filled fields as the page set them,
/// checked boxes, the pressed button, and the address in an empty email
/// field. A radio group with nothing checked takes its unsubscribe choice.
fn form_fields(inner: &str, email: Option<&str>) -> Vec<(String, String)> {
    let controls = form_controls(inner);
    let pressed = controls
        .iter()
        .position(|control| is_submit(control) && has_keyword(&control.label, FOOTER_KEYWORDS))
        .or_else(|| controls.iter().position(is_submit));
    let checked_groups: Vec<&str> = controls
        .iter()
        .filter(|control| control.kind == "radio" && control.checked)
        .filter_map(|control| control.name.as_deref())
        .collect();
    let mut chosen_groups: Vec<String> = Vec::new();
    let mut fields = Vec::new();
    for (index, control) in controls.iter().enumerate() {
        let Some(name) = control.name.clone() else {
            continue;
        };
        if control.disabled {
            continue;
        }
        let value = control.value.clone().unwrap_or_default();
        match control.kind.as_str() {
            "submit" | "image" => {
                if Some(index) == pressed {
                    fields.push((name, value));
                }
            }
            "button" | "reset" | "file" => {}
            "checkbox" => {
                let wanted =
                    control.checked || has_keyword(&format!("{name} {value}"), FOOTER_KEYWORDS);
                if wanted {
                    let value = if value.is_empty() { "on".into() } else { value };
                    fields.push((name, value));
                }
            }
            "radio" => {
                let pick = if checked_groups.contains(&name.as_str()) {
                    control.checked
                } else {
                    !chosen_groups.contains(&name)
                        && has_keyword(&format!("{name} {value}"), FOOTER_KEYWORDS)
                };
                if pick {
                    chosen_groups.push(name.clone());
                    fields.push((name, value));
                }
            }
            kind => {
                let is_email = kind == "email" || name.to_ascii_lowercase().contains("mail");
                let value = match email {
                    Some(address) if is_email && value.trim().is_empty() => address.to_string(),
                    _ => value,
                };
                fields.push((name, value));
            }
        }
    }
    fields
}

/// The DKIM verdict of the receiving server. A message signed twice passes
/// when either signature passes.
fn dkim_verdict(auth_results: Option<&str>) -> Dkim {
    let Some(text) = auth_results else {
        return Dkim::Unknown;
    };
    let text = text.to_ascii_lowercase();
    let mut seen = false;
    for (index, _) in text.match_indices("dkim=") {
        // `header.d=` and friends also end in `d=`; a real result starts a token.
        let starts_token = text[..index]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_ascii_whitespace() || c == ';');
        if !starts_token {
            continue;
        }
        seen = true;
        if text[index + "dkim=".len()..].starts_with("pass") {
            return Dkim::Pass;
        }
    }
    if seen { Dkim::Fail } else { Dkim::Unknown }
}

/// The URIs of a `List-Unsubscribe` header. RFC 2369 puts each one between
/// angle brackets, but some senders write bare comma-separated URIs, and
/// some MIME-encode the whole header (RFC 2047), so both are accepted.
fn header_uris(header: &str) -> Vec<String> {
    let header = decode_encoded_words(header);
    let compact = |uri: &str| uri.split_whitespace().collect::<String>();
    if header.contains('<') {
        header
            .split('<')
            .skip(1)
            .filter_map(|part| part.split_once('>'))
            .map(|(uri, _)| compact(uri))
            .filter(|uri| !uri.is_empty())
            .collect()
    } else {
        header
            .split(',')
            .map(compact)
            .filter(|uri| !uri.is_empty())
            .collect()
    }
}

fn is_one_click_post(header: &str) -> bool {
    decode_encoded_words(header)
        .split_whitespace()
        .collect::<String>()
        .eq_ignore_ascii_case("List-Unsubscribe=One-Click")
}

/// Decode RFC 2047 encoded words (`=?charset?Q?...?=`, `=?charset?B?...?=`).
/// Whitespace between two encoded words is dropped, as the RFC says. Text
/// that is not a well-formed encoded word is kept as it is.
fn decode_encoded_words(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    let mut after_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, word) = rest.split_at(start);
        match encoded_word(&word[2..]) {
            Some((value, consumed)) => {
                if !(after_word && before.trim().is_empty()) {
                    decoded.push_str(before);
                }
                decoded.push_str(&value);
                rest = &word[2 + consumed..];
                after_word = true;
            }
            None => {
                decoded.push_str(before);
                decoded.push_str("=?");
                rest = &word[2..];
                after_word = false;
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

/// One encoded word after its `=?`: the decoded text and how many bytes it
/// took, up to and including its `?=`. The charset is read as UTF-8, which
/// covers the ASCII URIs these headers carry.
fn encoded_word(word: &str) -> Option<(String, usize)> {
    use base64::Engine as _;

    let (charset, rest) = word.split_once('?')?;
    let (encoding, rest) = rest.split_once('?')?;
    let end = rest.find("?=")?;
    let payload = &rest[..end];
    let bytes = if encoding.eq_ignore_ascii_case("q") {
        q_decode(payload)
    } else if encoding.eq_ignore_ascii_case("b") {
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .ok()?
    } else {
        return None;
    };
    let consumed = charset.len() + 1 + encoding.len() + 1 + end + 2;
    Some((String::from_utf8_lossy(&bytes).into_owned(), consumed))
}

/// RFC 2047 Q encoding: `_` is a space, `=XX` a byte in hex.
fn q_decode(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex_byte = (bytes[index] == b'=')
            .then(|| payload.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = hex_byte {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(if bytes[index] == b'_' {
                b' '
            } else {
                bytes[index]
            });
            index += 1;
        }
    }
    decoded
}

/// The URI as a web address the app may reach, HTTPS only when asked.
fn public_web_url(uri: &str, https_only: bool) -> Option<String> {
    let url = Url::parse(uri).ok()?;
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => !https_only,
        _ => false,
    };
    (scheme_ok && crate::ui::mail_body::is_public_url(&url)).then(|| url.to_string())
}

fn mail_method(uri: &str) -> Option<UnsubscribeMethod> {
    let url = Url::parse(uri)
        .ok()
        .filter(|url| url.scheme() == "mailto")?;
    let to = url
        .path()
        .split(',')
        .map(|address| percent_decode(address.trim()))
        .find(|address| is_plain_address(address))?;
    let mut subject = None;
    let mut body = None;
    for (key, value) in url.query_pairs() {
        if key.eq_ignore_ascii_case("subject") {
            subject = Some(value.into_owned());
        } else if key.eq_ignore_ascii_case("body") {
            body = Some(value.into_owned());
        }
    }
    // The subject and the body come from the sender of the mail, and go out
    // under the user's name to the address the sender chose. A list takes
    // a word or a token there, never prose: anything longer than a short
    // line is replaced, so the header cannot dictate a letter.
    let or_default = |text: Option<String>| {
        text.map(|text| text.trim().to_owned())
            .filter(|text| is_unsubscribe_token(text))
            .unwrap_or_else(|| MAIL_DEFAULT_TEXT.to_string())
    };
    Some(UnsubscribeMethod::Mail {
        to,
        subject: or_default(subject),
        body: or_default(body),
    })
}

/// Longest subject or body a `mailto:` unsubscribe may set.
const MAIL_TOKEN_MAX_CHARS: usize = 120;
/// Most words it may hold: "unsubscribe me from list-name" passes, a
/// sentence written to be read by a person does not.
const MAIL_TOKEN_MAX_WORDS: usize = 6;

/// Whether `text` is what a list asks for to take an address off: one short
/// line of a few words, with no control character.
fn is_unsubscribe_token(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= MAIL_TOKEN_MAX_CHARS
        && text.split_whitespace().count() <= MAIL_TOKEN_MAX_WORDS
        && !text.chars().any(char::is_control)
}

/// One bare address: a local part, an `@` and a domain, nothing else.
fn is_plain_address(address: &str) -> bool {
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.contains('@')
        && !address.contains(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | ',' | '"'))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The last unsubscribe link of an HTML body: footers close the message.
fn footer_link(html: &str) -> Option<String> {
    page_links(html)
        .into_iter()
        .filter(|(href, label)| {
            has_keyword(label, FOOTER_KEYWORDS) || has_keyword(href, FOOTER_KEYWORDS)
        })
        .filter_map(|(href, _)| public_web_url(href.trim(), false))
        .next_back()
}

/// The links of an HTML page: each `href` and its visible label, both decoded.
fn page_links(html: &str) -> Vec<(String, String)> {
    // ASCII lowercasing keeps byte offsets, so indices map back to `html`.
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut cursor = 0;
    while let Some(start) = find_tag(&lower, "a", cursor) {
        let after_name = start + 2;
        let Some(tag_end) = lower[after_name..].find('>').map(|at| after_name + at) else {
            break;
        };
        let close = lower[tag_end..]
            .find("</a")
            .map_or(lower.len(), |at| tag_end + at);
        cursor = close;
        let Some(href) = attribute(&html[after_name..tag_end], "href") else {
            continue;
        };
        let label = decode_entities(&strip_tags(&html[tag_end + 1..close]));
        links.push((decode_entities(&href), label));
    }
    links
}

/// The value of an attribute in the inside of an opening tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(at) = lower[search..].find(name).map(|at| search + at) {
        search = at + name.len();
        let preceded = lower[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_whitespace());
        let rest = lower[search..].trim_start();
        if !preceded || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        return Some(match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or("").to_string(),
            _ => value
                .split(|c: char| c.is_ascii_whitespace())
                .next()
                .unwrap_or("")
                .to_string(),
        });
    }
    None
}

fn strip_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    text
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&#233;", "é")
        .replace("&eacute;", "é")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers<'a>(
        list_unsubscribe: Option<&'a str>,
        post: Option<&'a str>,
        auth: Option<&'a str>,
        body: Option<&'a str>,
    ) -> ListHeaders<'a> {
        ListHeaders {
            list_unsubscribe,
            list_unsubscribe_post: post,
            auth_results: auth,
            body_html: body,
        }
    }

    const PASS: &str = "mx.google.com; dkim=pass header.i=@news.example.com; spf=pass";

    #[test]
    fn bills_and_account_notices_are_not_newsletters() {
        assert!(is_transactional(
            Some("[Important] Impossible de prélever votre Carte"),
            []
        ));
        assert!(is_transactional(Some("Votre facture de novembre"), []));
        assert!(is_transactional(Some("Your order has shipped"), []));
        assert!(is_transactional(
            Some("Bonjour"),
            ["invoice_MONAPP2026.pdf"]
        ));
    }

    #[test]
    fn a_newsletter_keeps_its_unsubscribe_offer() {
        assert!(!is_transactional(Some("Les nouveautés du mois"), []));
        assert!(!is_transactional(
            Some("5 idées pour votre week-end"),
            ["photo.jpg"]
        ));
        assert!(!is_transactional(None, []));
    }

    #[test]
    fn one_click_wins_over_mailto() {
        let method = method_for(headers(
            Some("<mailto:leave@news.example.com>, <https://news.example.com/u?id=42>"),
            Some("List-Unsubscribe=One-Click"),
            Some(PASS),
            None,
        ));
        assert_eq!(
            method,
            Some(UnsubscribeMethod::OneClick {
                url: "https://news.example.com/u?id=42".into()
            })
        );
    }

    #[test]
    fn one_click_needs_https() {
        let method = method_for(headers(
            Some("<http://news.example.com/u>"),
            Some("List-Unsubscribe=One-Click"),
            None,
            None,
        ));
        assert_eq!(
            method,
            Some(UnsubscribeMethod::Page {
                url: "http://news.example.com/u".into()
            })
        );
    }

    #[test]
    fn mailto_without_one_click_is_sent_with_its_subject() {
        let method = method_for(headers(
            Some(
                "<https://news.example.com/u>, <mailto:leave@news.example.com?subject=Remove%20me>",
            ),
            None,
            Some(PASS),
            None,
        ));
        assert_eq!(
            method,
            Some(UnsubscribeMethod::Mail {
                to: "leave@news.example.com".into(),
                subject: "Remove me".into(),
                body: "unsubscribe".into(),
            })
        );
    }

    #[test]
    fn a_mailto_cannot_dictate_a_letter() {
        // A hostile newsletter: one click would send this text, under the
        // user's name, to the address it names.
        let method = mail_method(
            "mailto:staff@victim.example?subject=Urgent%20payment%20needed%20today%20for%20the%20open%20invoice%20please\
             &body=Hello%2C%0AWire%20the%20amount%20to%20the%20account%20below%20before%20noon.",
        );
        assert_eq!(
            method,
            Some(UnsubscribeMethod::Mail {
                to: "staff@victim.example".into(),
                subject: MAIL_DEFAULT_TEXT.into(),
                body: MAIL_DEFAULT_TEXT.into(),
            })
        );
        // What a list really asks for goes through.
        let method = mail_method(
            "mailto:leave@list.example.org?subject=unsubscribe%20news-weekly&body=token-8f3a91c2",
        );
        assert_eq!(
            method,
            Some(UnsubscribeMethod::Mail {
                to: "leave@list.example.org".into(),
                subject: "unsubscribe news-weekly".into(),
                body: "token-8f3a91c2".into(),
            })
        );
    }

    #[test]
    fn folded_header_uri_is_rejoined() {
        assert_eq!(
            header_uris("<https://news.example.com/u?a=1\r\n &b=2>"),
            vec!["https://news.example.com/u?a=1&b=2".to_string()]
        );
    }

    #[test]
    fn bare_header_uri_without_brackets_is_read() {
        let method = method_for(headers(
            Some("http://www.review.example/front/unsubscribe?id=1&sign=2"),
            None,
            Some(PASS),
            None,
        ));
        assert_eq!(
            method,
            Some(UnsubscribeMethod::Page {
                url: "http://www.review.example/front/unsubscribe?id=1&sign=2".into()
            })
        );
    }

    #[test]
    fn rfc2047_encoded_header_is_decoded() {
        let header = "=?us-ascii?Q?=3Chttps=3A=2F=2Fmarketing=2Eexample=2Ecom=2Fone-click?=\r\n \
                      =?us-ascii?Q?=3Fu=3D1=3E?=";
        assert_eq!(
            header_uris(header),
            vec!["https://marketing.example.com/one-click?u=1".to_string()]
        );
        assert_eq!(
            decode_encoded_words("=?utf-8?B?PG1haWx0bzp4QHkuY29tPg==?="),
            "<mailto:x@y.com>"
        );
    }

    #[test]
    fn malformed_encoded_word_is_kept_as_is() {
        assert_eq!(decode_encoded_words("a =?broken b"), "a =?broken b");
    }

    #[test]
    fn failed_dkim_gets_no_offer() {
        let method = method_for(headers(
            Some("<https://news.example.com/u>"),
            Some("List-Unsubscribe=One-Click"),
            Some("mx.example.net; dkim=fail header.d=example.com"),
            None,
        ));
        assert_eq!(method, None);
    }

    #[test]
    fn second_signature_passing_is_enough() {
        assert_eq!(
            dkim_verdict(Some(
                "mx; dkim=fail header.d=a.com; dkim=pass header.d=b.com"
            )),
            Dkim::Pass
        );
        assert_eq!(
            dkim_verdict(Some("mx; spf=pass smtp.mailfrom=a.com")),
            Dkim::Unknown
        );
    }

    #[test]
    fn private_hosts_are_never_offered() {
        let method = method_for(headers(
            Some("<https://192.168.1.1/u>, <https://localhost/u>"),
            Some("List-Unsubscribe=One-Click"),
            Some(PASS),
            None,
        ));
        assert_eq!(method, None);
    }

    #[test]
    fn footer_link_is_used_without_headers_when_dkim_passes() {
        let body = r#"<p>Hello <a href="https://news.example.com/read">read</a></p>
            <p><a class="x" href='https://news.example.com/prefs?u=1&amp;l=2'>Se
            <b>désabonner</b></a></p>"#;
        assert_eq!(
            method_for(headers(None, None, Some(PASS), Some(body))),
            Some(UnsubscribeMethod::Page {
                url: "https://news.example.com/prefs?u=1&l=2".into()
            })
        );
        assert_eq!(method_for(headers(None, None, None, Some(body))), None);
    }

    #[test]
    fn footer_link_matches_on_its_address_too() {
        let body = r#"<a href="https://x.example.com/unsubscribe/abc">Click here</a>"#;
        assert_eq!(
            footer_link(body),
            Some("https://x.example.com/unsubscribe/abc".into())
        );
    }

    #[test]
    fn footer_ignores_other_tags_starting_with_a() {
        let body = r#"<abbr title="unsubscribe">x</abbr><article>unsubscribe</article>"#;
        assert_eq!(footer_link(body), None);
    }

    #[test]
    fn mailto_with_two_addresses_takes_the_first_valid_one() {
        assert_eq!(
            mail_method("mailto:not-an-address,leave@list.example.org?body=stop"),
            Some(UnsubscribeMethod::Mail {
                to: "leave@list.example.org".into(),
                subject: "unsubscribe".into(),
                body: "stop".into(),
            })
        );
    }
    #[test]
    fn unsubscribe_form_sends_hidden_fields_the_pressed_button_and_the_address() {
        let html = r#"<form action="/search"><input name="q"><button>Go</button></form>
            <form method="POST" action="/unsub/do?x=1&amp;y=2">
              <input type="hidden" name="token" value="abc">
              <input type="email" name="email" value="">
              <input type="checkbox" name="reason_spam">
              <input type="checkbox" name="unsubscribe_all">
              <input type="radio" name="scope" value="weekly">
              <input type="radio" name="scope" value="unsubscribe_everything">
              <select name="lang"><option value="en">EN</option><option value="fr" selected>FR</option></select>
              <input type="submit" name="keep" value="Stay subscribed">
              <button type="submit" name="go" value="1">Se <b>désabonner</b></button>
            </form>"#;
        let form = unsubscribe_form(html).expect("the unsubscribe form");
        assert!(form.post);
        assert_eq!(form.action.as_deref(), Some("/unsub/do?x=1&y=2"));
        assert_eq!(
            form_fields(form.inner, Some("me@example.com")),
            vec![
                ("token".to_string(), "abc".to_string()),
                ("email".to_string(), "me@example.com".to_string()),
                ("unsubscribe_all".to_string(), "on".to_string()),
                ("scope".to_string(), "unsubscribe_everything".to_string()),
                ("lang".to_string(), "fr".to_string()),
                ("go".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn form_without_an_unsubscribe_word_is_not_taken() {
        let html = r#"<form action="/login"><input name="user"><input type="submit" value="Log in"></form>"#;
        assert!(unsubscribe_form(html).is_none());
    }

    #[test]
    fn checked_radio_keeps_its_group() {
        let inner = r#"<input type="radio" name="s" value="a" checked>
            <input type="radio" name="s" value="unsubscribe">"#;
        assert_eq!(
            form_fields(inner, None),
            vec![("s".to_string(), "a".to_string())]
        );
    }

    #[test]
    fn confirmation_is_read_from_visible_text_only() {
        assert!(confirms(
            "<h1>Vous avez &eacute;t&eacute; d&eacute;sabonn&eacute;</h1>"
        ));
        assert!(confirms("<p>You have been <b>unsubscribed</b>.</p>"));
        assert!(!confirms(
            "<script>var done = 'unsubscribed';</script><style>.unsubscribed{}</style><p>Hi</p>"
        ));
    }

    #[test]
    fn links_are_listed_with_their_labels() {
        let html = r#"<a href="/x?a=1&amp;b=2">Confirm <i>now</i></a><abbr>a</abbr><a>none</a>"#;
        assert_eq!(
            page_links(html),
            vec![("/x?a=1&b=2".to_string(), "Confirm  now ".to_string())]
        );
    }
}
