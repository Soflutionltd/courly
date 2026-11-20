//! HTML signatures: importing one exported from Outlook or a browser, and
//! turning its embedded images into inline MIME parts when sending.

use std::path::Path;

use base64::Engine as _;

/// An image moved out of the HTML into an inline MIME part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub content_id: String,
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// Read a signature saved as `.htm` / `.html`. The body is kept, styles of
/// the head included, and images stored next to the file (Outlook writes
/// them to a `<name>_files` folder) are embedded so the signature no longer
/// depends on the file system.
pub fn import_signature_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let html = decode(&bytes);
    let body = crate::pop_out::compose::embeddable_body(&html);
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(embed_local_images(&body, base))
}

/// A signature pasted as HTML, ready to keep as an HTML signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastedSignature {
    pub html: String,
    /// Images the paste referenced by `cid:` that no synced message holds.
    pub missing_images: usize,
}

/// Whether pasted HTML is a laid out signature to keep as it is, as opposed
/// to a few styled words that paste as text.
pub fn is_laid_out(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("<img") || lower.contains("<table")
}

/// Turn HTML pasted from a mail client into a self-contained signature.
///
/// Outlook does not put a signature's images on the clipboard: the HTML
/// names them `cid:<id>`, the inline parts of the message it was copied
/// from. A signature copied from the user's own mail is in the mailbox many
/// times over, so each `cid:` is looked up among the synced inline
/// attachments and its bytes are embedded.
///
/// `source` is the `(account_id, message_id)` the HTML is the body of, when
/// it is known: that message's own parts are then preferred over another
/// message's part carrying the same content id.
pub async fn import_pasted_signature(
    db: std::sync::Arc<crate::db::Db>,
    inline_images: Option<rtsk::inline_image_store::InlineImageStoreReadState>,
    html: String,
    source: Option<(String, String)>,
) -> Result<PastedSignature, String> {
    let body = crate::pop_out::compose::embeddable_body(&html);
    let content_ids = cid_sources(&body);
    let lookup_ids = content_ids.clone();
    let candidates = db
        .with_read(move |conn| inline_image_hashes(conn, &lookup_ids, source.as_ref()))
        .await?;

    let mut found: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(store) = inline_images.as_ref() {
        for (content_id, hashes) in content_ids.iter().zip(candidates) {
            for hash in hashes {
                if let Some((data, mime)) = store.get(hash).await? {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                    found.insert(content_id.clone(), format!("data:{mime};base64,{encoded}"));
                    break;
                }
            }
        }
    }

    let missing_images = content_ids
        .iter()
        .filter(|content_id| !found.contains_key(*content_id))
        .count();
    let html = rewrite_img_sources(&body, |src| found.get(content_id_of(src)?).cloned());
    Ok(PastedSignature {
        html,
        missing_images,
    })
}

/// The content id a `cid:` source names, `None` for any other source.
fn content_id_of(src: &str) -> Option<&str> {
    let (scheme, content_id) = src.split_at_checked(4)?;
    scheme.eq_ignore_ascii_case("cid:").then_some(content_id)
}

/// The distinct content ids the `<img>` tags reference, in document order.
fn cid_sources(html: &str) -> Vec<String> {
    let mut content_ids: Vec<String> = Vec::new();
    let _ = rewrite_img_sources(html, |src| {
        if let Some(content_id) = content_id_of(src)
            && !content_ids.iter().any(|known| known == content_id)
        {
            content_ids.push(content_id.to_string());
        }
        None
    });
    content_ids
}

/// For each content id, the hashes of the synced inline attachments carrying
/// it, those of the `source` message first, then newest first. Several are
/// returned because the image store may have dropped the bytes of some.
fn inline_image_hashes(
    conn: &rtsk::db::ReadConn<'_>,
    content_ids: &[String],
    source: Option<&(String, String)>,
) -> Result<Vec<Vec<String>>, String> {
    let mut statement = conn
        .prepare(
            "SELECT lower(hex(content_hash))
             FROM attachments
             WHERE content_id = ?1 AND content_hash IS NOT NULL
             GROUP BY content_hash
             ORDER BY max(account_id = ?2 AND message_id = ?3) DESC, max(rowid) DESC
             LIMIT 4",
        )
        .map_err(|e| format!("prepare signature image lookup: {e}"))?;
    let (account_id, message_id) = source.map_or(("", ""), |(account_id, message_id)| {
        (account_id.as_str(), message_id.as_str())
    });
    content_ids
        .iter()
        .map(|content_id| {
            let mut rows = statement
                .query([content_id.as_str(), account_id, message_id])
                .map_err(|e| format!("query signature image lookup: {e}"))?;
            let mut hashes = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| format!("read signature image lookup: {e}"))?
            {
                hashes.push(
                    row.get::<_, String>(0)
                        .map_err(|e| format!("map signature image lookup: {e}"))?,
                );
            }
            Ok(hashes)
        })
        .collect()
}

/// Decode with the charset the file declares (Outlook on Windows writes
/// `windows-1252`), UTF-8 otherwise.
fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding.decode(bytes).0.into_owned();
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
    let declared = head.find("charset=").and_then(|start| {
        let rest = head[start + 8..].trim_start_matches(['"', '\'']);
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        encoding_rs::Encoding::for_label(&rest.as_bytes()[..end])
    });
    declared
        .unwrap_or(encoding_rs::UTF_8)
        .decode(bytes)
        .0
        .into_owned()
}

/// Replace `src` attributes pointing at local files with `data:` URIs.
/// Remote, `cid:` and `data:` sources, and files that cannot be read, are
/// left as they are.
///
/// Only an image stored under `base`, the folder of the signature file, is
/// embedded. A signature file comes from somewhere else, and its `src` is
/// its author's: an absolute path or one climbing out with `..` would put
/// any file of the user's, a private key for one, into the signature, and
/// from there into every mail sent.
fn embed_local_images(html: &str, base: &Path) -> String {
    let Ok(base) = base.canonicalize() else {
        return html.to_owned();
    };
    rewrite_img_sources(html, |src| {
        let lower = src.to_ascii_lowercase();
        if ["http:", "https:", "cid:", "data:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
        {
            return None;
        }
        let relative = percent_decode(src.trim_start_matches("file://"));
        let path = base.join(relative).canonicalize().ok()?;
        if !path.starts_with(&base) {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let mime = image_mime(&bytes)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{mime};base64,{encoded}"))
    })
}

/// Take off every `<table>` the width a mail client measured rather than
/// chose.
///
/// Copying a signature out of Outlook puts the width each table had on
/// screen into the HTML, to a fraction of a pixel: `width: 519.921875px`.
/// Nobody asked for that width, and it turns a table that fitted its
/// content into one that cannot grow: a word added to the address no longer
/// fits its cell and drops to the next line, here and at the recipient. A
/// width with a fraction of a pixel is taken as measured and removed, on a
/// table or a cell, and so is a width in points on a line of text alone in
/// its row: Outlook writes there the width the text measured. A whole
/// number of pixels, a percentage, the width of a cell beside others and
/// that of a cell holding an image or nothing are somebody's choice and
/// are kept.
pub fn free_measured_table_widths(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some((tag, cell)) = next_sized_tag(&lower, cursor) {
        let Some(tag_end) = lower[tag..].find('>').map(|i| tag + i) else {
            break;
        };
        // A line of text alone in its row is as wide as its text: the
        // width it was given in points is the width the old text measured,
        // so a word added to it no longer fits. A cell beside others, or
        // one that holds an image or nothing, keeps its width: that width
        // is what lays the columns out.
        let line_of_text = cell
            && cell_starts_with_text(&lower[tag_end + 1..])
            && alone_in_its_row(&lower, tag, tag_end);
        let measured =
            |value: &str| is_fraction_of_pixel(value) || (line_of_text && is_points(value));
        out.push_str(&html[cursor..tag]);
        out.push_str(&without_measured_width(&html[tag..tag_end], &measured));
        cursor = tag_end;
    }
    out.push_str(&html[cursor..]);
    out
}

/// The next `<table>`, `<td>` or `<th>` opening tag from `from`, and
/// whether it is a cell.
fn next_sized_tag(lower: &str, from: usize) -> Option<(usize, bool)> {
    let mut search = from;
    while let Some(at) = lower[search..].find('<').map(|i| search + i) {
        search = at + 1;
        for (name, cell) in [("table", false), ("td", true), ("th", true)] {
            let rest = &lower[at + 1..];
            if rest.starts_with(name)
                && rest[name.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_whitespace() || c == '>')
            {
                return Some((at, cell));
            }
        }
    }
    None
}

/// Whether the cell opened at `tag` (its tag ending at `tag_end`) is the
/// only cell of its row.
fn alone_in_its_row(lower: &str, tag: usize, tag_end: usize) -> bool {
    // Nothing but the row's own tag stands before it.
    let before = &lower[..tag];
    let row = before.rfind("<tr");
    let cell_before = ["</td", "</th"]
        .iter()
        .filter_map(|closing| before.rfind(closing))
        .max();
    if row.is_none() || cell_before > row {
        return false;
    }
    // Its own closing tag, past the cells of any table inside it.
    let mut depth = 1_usize;
    let mut at = tag_end;
    let close = loop {
        let Some(next) = lower[at..].find('<').map(|i| at + i) else {
            return false;
        };
        let rest = &lower[next..];
        if rest.starts_with("</td") || rest.starts_with("</th") {
            depth -= 1;
            if depth == 0 {
                break next;
            }
        } else if (rest.starts_with("<td") || rest.starts_with("<th"))
            && rest[3..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace() || c == '>')
        {
            depth += 1;
        }
        at = next + 1;
    };
    lower[close..]
        .find('>')
        .is_some_and(|end| lower[close + end + 1..].trim_start().starts_with("</tr"))
}

/// Whether a cell shows text before it ends or before a table starts in it.
fn cell_starts_with_text(after_tag: &str) -> bool {
    let end = ["</td", "</th", "<table"]
        .iter()
        .filter_map(|stop| after_tag.find(stop))
        .min()
        .unwrap_or(after_tag.len());
    let mut in_tag = false;
    after_tag[..end]
        .replace("&nbsp;", " ")
        .chars()
        .any(|c| match c {
            '<' => {
                in_tag = true;
                false
            }
            '>' => {
                in_tag = false;
                false
            }
            _ => !in_tag && !c.is_whitespace() && c != '\u{a0}',
        })
}

/// One opening tag without its `width`, as an attribute or in its style,
/// when `measured` says that width was measured.
fn without_measured_width(tag: &str, measured: &dyn Fn(&str) -> bool) -> String {
    let mut tag = tag.to_owned();
    // In the style: "width: 519.921875px;" between two declarations.
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(at) = lower[search..].find("width").map(|i| search + i) {
        search = at + 5;
        let before = lower[..at].trim_end();
        let declared = before.ends_with([';', '"', '\'']) || before.ends_with("style=");
        let Some(colon) = lower[search..].find(':').map(|i| search + i) else {
            continue;
        };
        if !declared || !lower[search..colon].trim().is_empty() {
            continue;
        }
        let end = lower[colon..]
            .find([';', '"', '\''])
            .map_or(lower.len(), |i| colon + i);
        if measured(&lower[colon + 1..end]) {
            let end = if lower[end..].starts_with(';') {
                end + 1
            } else {
                end
            };
            tag.replace_range(at..end, "");
            return without_measured_width(&tag, measured);
        }
    }
    // As an attribute: width="519.921875px".
    for quote in ['"', '\''] {
        let open = format!(" width={quote}");
        if let Some(at) = lower.find(&open) {
            let value = at + open.len();
            if let Some(end) = lower[value..].find(quote).map(|i| value + i)
                && measured(&lower[value..end])
            {
                tag.replace_range(at..=end, "");
                return without_measured_width(&tag, measured);
            }
        }
    }
    tag
}

/// Whether a CSS length is a number of points: "225pt", "61.5pt".
fn is_points(value: &str) -> bool {
    value.trim().strip_suffix("pt").is_some_and(|number| {
        let number = number.trim();
        !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    })
}

/// Whether a CSS length is a number of pixels with a fraction: "519.92px".
fn is_fraction_of_pixel(value: &str) -> bool {
    let value = value.trim();
    let number = value.strip_suffix("px").unwrap_or(value).trim();
    number.split_once('.').is_some_and(|(whole, fraction)| {
        !whole.is_empty()
            && whole.bytes().all(|b| b.is_ascii_digit())
            && fraction.bytes().all(|b| b.is_ascii_digit())
            && fraction.bytes().any(|b| b != b'0')
    })
}

/// Write the paragraphs of a message as mail clients lay them out, so that
/// what is sent is what was typed.
///
/// The editor shows a paragraph on the line after the one before it, and an
/// empty paragraph as an empty line. Left as plain `<p>`, a mail client
/// gives every paragraph a margin, which reads as a blank line between each
/// two lines, and gives an empty one no height at all, which loses the
/// blank lines that were typed. Every `<p>` gets `margin:0`, unless it
/// already sets its margin, and an empty one gets a line break to stand on.
pub fn mail_paragraphs(html: &str) -> String {
    const NO_MARGIN: &str = "margin:0";
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut cursor = 0;
    while let Some(tag) = lower[cursor..].find("<p").map(|i| cursor + i) {
        let is_paragraph = lower[tag + 2..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '>');
        let Some(tag_end) = lower[tag..].find('>').map(|i| tag + i) else {
            break;
        };
        out.push_str(&html[cursor..tag_end]);
        cursor = tag_end;
        if !is_paragraph {
            continue;
        }
        if !lower[tag..tag_end].contains("margin:") {
            let style = ["style=\"", "style='"]
                .iter()
                .find_map(|open| lower[tag..tag_end].find(open).map(|i| tag + i + open.len()));
            match style {
                Some(value_start) => {
                    // Put back what was pushed past the start of the value.
                    out.truncate(out.len() - (tag_end - value_start));
                    out.push_str(NO_MARGIN);
                    out.push(';');
                    out.push_str(&html[value_start..tag_end]);
                }
                None => {
                    out.push_str(" style=\"");
                    out.push_str(NO_MARGIN);
                    out.push('"');
                }
            }
        }
        if lower[tag_end..].starts_with("></p>") {
            out.push_str("><br>");
            cursor = tag_end + 1;
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// Take the default underline off every link of a message being written.
///
/// A link without a style of its own is underlined by whichever mail client
/// shows it, in that client's blue, under text a signature usually colours
/// itself. Each `<a>` is given `text-decoration:none`; one that already
/// decides its decoration is left alone, and so is text the writer
/// underlined on purpose, which is a `<u>` inside the link.
pub fn plain_links(html: &str) -> String {
    const PLAIN: &str = "text-decoration:none";
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(tag) = lower[cursor..].find("<a").map(|i| cursor + i) {
        let name_end = tag + 2;
        let is_link = lower[name_end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '>');
        let Some(tag_end) = lower[tag..].find('>').map(|i| tag + i) else {
            break;
        };
        if !is_link || lower[tag..tag_end].contains("text-decoration") {
            out.push_str(&html[cursor..tag_end]);
            cursor = tag_end;
            continue;
        }
        let style = ["style=\"", "style='"]
            .iter()
            .find_map(|open| lower[tag..tag_end].find(open).map(|i| tag + i + open.len()));
        match style {
            Some(value_start) => {
                out.push_str(&html[cursor..value_start]);
                out.push_str(PLAIN);
                out.push(';');
                out.push_str(&html[value_start..tag_end]);
            }
            None => {
                out.push_str(&html[cursor..tag_end]);
                out.push_str(" style=\"");
                out.push_str(PLAIN);
                out.push('"');
            }
        }
        cursor = tag_end;
    }
    out.push_str(&html[cursor..]);
    out
}

/// Replace the `cid:` sources `image` has the bytes of with `data:` URIs.
///
/// A message names its inline images by content id, and they travel as
/// parts of that message. Quoted in a reply, the HTML is copied but the
/// parts are not: embedded here, the images show under the reply being
/// written and [`extract_data_images`] sends them with it. A source whose
/// bytes are missing, or are no image this recognises, is left as it is.
pub fn embed_inline_images<'a>(html: &str, image: impl Fn(&str) -> Option<&'a [u8]>) -> String {
    rewrite_img_sources(html, |src| {
        let content_id = content_id_of(src)?
            .trim_start_matches('<')
            .trim_end_matches('>');
        let bytes = image(content_id)?;
        let mime = image_mime(bytes)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{mime};base64,{encoded}"))
    })
}

/// The media type of an image, read from its first bytes.
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// Move `data:` images into inline parts referenced by `cid:`, which every
/// mail client shows (several webmails drop `data:` images).
///
/// `mail` is a token unique to the mail being sent, and goes into every
/// content id. A content id must be unique in the world (RFC 2045), and a
/// client that keeps the images of a whole conversation together tells them
/// apart by it alone: two mails of one conversation that both call their
/// first image `image001` show each other's pictures, or none.
///
/// Only bytes that are an image become a part, under the type their first
/// bytes give, whatever the URI calls them. The body being sent holds the
/// message it answers, written by someone else: an `<img>` whose `data:`
/// URI names an executable type and carries one would otherwise leave as
/// an attached file of that type, under the user's name, to everyone the
/// reply goes to.
pub fn extract_data_images(html: &str, mail: &str) -> (String, Vec<InlineImage>) {
    let mut images = Vec::new();
    let rewritten = rewrite_img_sources(html, |src| {
        let rest = src.strip_prefix("data:")?;
        let (meta, payload) = rest.split_once(',')?;
        meta.strip_suffix(";base64")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?;
        let mime = image_mime(&data)?;
        let number = images.len() + 1;
        let content_id = format!("image{number:03}.{mail}@courly");
        let extension = mime_guess::get_mime_extensions_str(mime)
            .and_then(|extensions| extensions.first())
            .unwrap_or(&"bin");
        images.push(InlineImage {
            content_id: content_id.clone(),
            filename: format!("image{number:03}.{extension}"),
            mime_type: mime.to_string(),
            data,
        });
        Some(format!("cid:{content_id}"))
    });
    (rewritten, images)
}

/// Give every `<img>` sized by `width` / `height` attributes the same size
/// as inline CSS too.
///
/// A signature exported from Outlook sizes its icons with attributes alone.
/// Some mail clients drop those when they show or quote the message, and the
/// image falls back to the size of its file: a 12px icon comes back 30px
/// wide and pushes the text beside it onto a second line. A client that
/// drops one of the two forms usually keeps the other. Sizes already set in
/// `style`, and percentages, are left alone.
pub fn pin_image_sizes(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(tag) = lower[cursor..].find("<img").map(|i| cursor + i) {
        let attributes_start = tag + 4;
        let tag_end = lower[tag..].find('>').map_or(html.len(), |i| tag + i);
        let attributes = attributes_start..tag_end;
        let style = attribute_value(&lower, attributes.clone(), "style");
        let declared: Vec<&str> = style
            .clone()
            .map(|range| {
                lower[range]
                    .split(';')
                    .filter_map(|declaration| declaration.split_once(':'))
                    .map(|(property, _)| property.trim())
                    .collect()
            })
            .unwrap_or_default();
        let missing: Vec<String> = ["width", "height"]
            .into_iter()
            .filter(|name| !declared.contains(name))
            .filter_map(|name| {
                let value = attribute_value(&lower, attributes.clone(), name)?;
                Some(format!("{name}:{}px", pixel_length(&html[value])?))
            })
            .collect();
        if missing.is_empty() {
            out.push_str(&html[cursor..tag_end]);
            cursor = tag_end;
            continue;
        }
        let sizes = missing.join(";");
        match style {
            Some(range) => {
                out.push_str(&html[cursor..range.end]);
                let existing = html[range.clone()].trim_end();
                if !existing.is_empty() && !existing.ends_with(';') {
                    out.push(';');
                }
                out.push_str(&sizes);
                cursor = range.end;
            }
            None => {
                out.push_str(&html[cursor..attributes_start]);
                out.push_str(&format!(" style=\"{sizes}\""));
                cursor = attributes_start;
            }
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// Where the value of attribute `name` sits in a tag, quotes excluded.
/// `lower` is the ASCII-lowercased document and `attributes` the part of the
/// tag after its name.
fn attribute_value(
    lower: &str,
    attributes: std::ops::Range<usize>,
    name: &str,
) -> Option<std::ops::Range<usize>> {
    let pattern = format!("{name}=");
    let mut from = attributes.start;
    while let Some(found) = lower[from..attributes.end].find(&pattern).map(|i| from + i) {
        let value_start = found + pattern.len();
        // `width=` must be an attribute of its own, not the tail of
        // `data-width=`.
        if !lower[..found].ends_with(|c: char| c.is_ascii_whitespace()) {
            from = value_start;
            continue;
        }
        let quote = lower[value_start..attributes.end]
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'');
        return Some(match quote {
            Some(quote) => {
                let start = value_start + 1;
                let end = lower[start..attributes.end]
                    .find(quote)
                    .map_or(attributes.end, |i| start + i);
                start..end
            }
            None => {
                let end = lower[value_start..attributes.end]
                    .find(|c: char| c.is_whitespace() || c == '/')
                    .map_or(attributes.end, |i| value_start + i);
                value_start..end
            }
        });
    }
    None
}

/// A `width` / `height` attribute value as whole pixels. Percentages and
/// anything else that is not a pixel count give `None`.
fn pixel_length(value: &str) -> Option<u32> {
    let value = value.trim();
    let digits = value.strip_suffix("px").unwrap_or(value).trim();
    digits.parse::<u32>().ok().filter(|pixels| *pixels > 0)
}

/// Call `replace` for the `src` of every `<img>`; a `Some` value becomes the
/// new source.
fn rewrite_img_sources(html: &str, mut replace: impl FnMut(&str) -> Option<String>) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(tag) = lower[cursor..].find("<img").map(|i| cursor + i) {
        let tag_end = lower[tag..].find('>').map_or(html.len(), |i| tag + i);
        let Some(attr) = lower[tag..tag_end].find("src=").map(|i| tag + i + 4) else {
            out.push_str(&html[cursor..tag_end]);
            cursor = tag_end;
            continue;
        };
        let quote = html[attr..]
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'');
        let (value_start, value_end) = match quote {
            Some(q) => {
                let start = attr + 1;
                let end = html[start..tag_end].find(q).map_or(tag_end, |i| start + i);
                (start, end)
            }
            None => {
                let end = html[attr..tag_end]
                    .find(|c: char| c.is_whitespace() || c == '/')
                    .map_or(tag_end, |i| attr + i);
                (attr, end)
            }
        };
        out.push_str(&html[cursor..value_start]);
        let value = html_unescape(&html[value_start..value_end]);
        match replace(&value) {
            Some(new) => out.push_str(&new),
            None => out.push_str(&html[value_start..value_end]),
        }
        cursor = value_end;
    }
    out.push_str(&html[cursor..]);
    out
}

fn html_unescape(value: &str) -> String {
    value.replace("&amp;", "&")
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = value.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlook_export_is_decoded_and_images_embedded() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/signature-import-test");
        let files = dir.join("La Villa_files");
        std::fs::create_dir_all(&files).expect("dir");
        std::fs::write(files.join("image001.png"), PNG_START).expect("png");
        let html = b"<html><head><meta charset=windows-1252><style>p{margin:0}</style></head>\
            <body><p>H\xf4tel</p><img src=\"La%20Villa_files/image001.png\"></body></html>";
        let path = dir.join("La Villa.htm");
        std::fs::write(&path, html).expect("htm");

        let imported = import_signature_file(&path).expect("import");
        assert!(imported.starts_with("<style>p{margin:0}</style><p>H\u{f4}tel</p>"));
        assert!(imported.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""));
    }

    /// The eight bytes every PNG file starts with.
    const PNG_START: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

    #[test]
    fn a_signature_file_embeds_nothing_from_outside_its_folder() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/signature-escape-test");
        let inside = dir.join("signature");
        std::fs::create_dir_all(&inside).expect("dir");
        // A real image, but next to the folder and not in it.
        std::fs::write(dir.join("outside.png"), PNG_START).expect("png");
        // A file in the folder that is no image.
        std::fs::write(inside.join("notes.png"), b"private notes").expect("notes");
        let outside = dir.join("outside.png");
        let html = format!(
            "<img src=\"../outside.png\"><img src=\"{}\"><img src=\"notes.png\">",
            outside.display()
        );
        assert_eq!(embed_local_images(&html, &inside), html);
    }

    #[test]
    fn data_that_is_no_image_is_not_sent_as_a_part() {
        // "MZ", the start of a Windows program, under an image type and
        // under its own.
        let html = "<img src=\"data:image/png;base64,TVqQAAMAAAA=\">\
            <img src=\"data:application/x-msdownload;base64,TVqQAAMAAAA=\">";
        let (rewritten, images) = extract_data_images(html, "mail1");
        assert_eq!(rewritten, html);
        assert!(images.is_empty());

        // A real image keeps the type its bytes give, whatever it is called.
        let html = "<img src=\"data:application/x-msdownload;base64,iVBORw0KGgo=\">";
        let (_, images) = extract_data_images(html, "mail1");
        assert_eq!(images[0].mime_type, "image/png");
        assert!(images[0].filename.ends_with(".png"));
    }

    #[test]
    fn remote_images_are_left_alone() {
        let html = "<img src=\"https://example.com/logo.png\" alt=\"x\">";
        assert_eq!(embed_local_images(html, Path::new("/nowhere")), html);
    }

    #[test]
    fn data_images_become_inline_parts() {
        let html = "<p>A</p><img alt=x src=\"data:image/png;base64,iVBORw0KGgo=\"><IMG SRC='data:image/gif;base64,R0lGODlh'>";
        let (rewritten, images) = extract_data_images(html, "mail1");
        assert_eq!(
            rewritten,
            "<p>A</p><img alt=x src=\"cid:image001.mail1@courly\"><IMG SRC='cid:image002.mail1@courly'>"
        );
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].content_id, "image001.mail1@courly");
        // Another mail of the same conversation names its images differently.
        let (_, others) = extract_data_images(html, "mail2");
        assert_ne!(others[0].content_id, images[0].content_id);
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(images[0].data, PNG_START);
        assert_eq!(images[1].mime_type, "image/gif");
    }

    #[test]
    fn a_table_loses_the_width_a_mail_client_measured() {
        let html = r#"<table cellpadding="0" style="border-collapse: collapse; width: 519.921875px; color: red"><tr><td style="width: 150pt"><img src="a.png"></td></tr></table><TABLE WIDTH="301.5" border="0"><tr><td>b</td></tr></TABLE>"#;
        assert_eq!(
            free_measured_table_widths(html),
            r#"<table cellpadding="0" style="border-collapse: collapse;  color: red"><tr><td style="width: 150pt"><img src="a.png"></td></tr></table><TABLE border="0"><tr><td>b</td></tr></TABLE>"#
        );
    }

    #[test]
    fn a_cell_holding_text_loses_the_width_its_text_measured() {
        let html = r#"<table><tr><td style="width: 150pt"><img src="a.png"></td><td style="width: 226.984375px"><table><tr><td style="width: 225pt; padding: 0"><span>Chemin, 20260</span></td></tr><tr><td width="61.5pt">&nbsp;<b>x</b></td></tr><tr><td style="width: 12pt">&nbsp;</td></tr></table></td></tr></table>"#;
        assert_eq!(
            free_measured_table_widths(html),
            r#"<table><tr><td style="width: 150pt"><img src="a.png"></td><td style=""><table><tr><td style=" padding: 0"><span>Chemin, 20260</span></td></tr><tr><td>&nbsp;<b>x</b></td></tr><tr><td style="width: 12pt">&nbsp;</td></tr></table></td></tr></table>"#
        );
    }

    #[test]
    fn a_width_somebody_chose_is_kept() {
        for html in [
            r#"<table style="width: 520px"><tr><td>a</td></tr></table>"#,
            r#"<table width="100%" style="max-width: 600.5px"><tr><td>a</td></tr></table>"#,
            r#"<table width="520.0"><tr><td>a</td></tr></table>"#,
            r#"<table style="border-width: 0.5px"><tr><td style="width: 12px">a</td></tr></table>"#,
            "<p>no table</p>",
        ] {
            assert_eq!(free_measured_table_widths(html), html);
        }
    }

    #[test]
    fn paragraphs_are_sent_without_margins_and_empty_ones_keep_their_line() {
        let html = r#"<p>Bonjour,</p><p></p><p style="text-align:center">Merci.</p><pre>x</pre><p style="margin:0 0 4px">kept</p>"#;
        let sent = mail_paragraphs(html);
        assert_eq!(
            sent,
            r#"<p style="margin:0">Bonjour,</p><p style="margin:0"><br></p><p style="margin:0;text-align:center">Merci.</p><pre>x</pre><p style="margin:0 0 4px">kept</p>"#
        );
        assert_eq!(mail_paragraphs(&sent), sent);
        assert_eq!(
            mail_paragraphs("<div>no paragraph</div>"),
            "<div>no paragraph</div>"
        );
    }

    #[test]
    fn links_lose_their_default_underline_once() {
        let html = r#"<a href="tel:+33600000000">call</a> <A HREF='https://example.com' style='color:#333'>site</A> <abbr>x</abbr> <a href="https://example.com/b" style="text-decoration:underline">kept</a>"#;
        let plain = plain_links(html);
        assert_eq!(
            plain,
            r#"<a href="tel:+33600000000" style="text-decoration:none">call</a> <A HREF='https://example.com' style='text-decoration:none;color:#333'>site</A> <abbr>x</abbr> <a href="https://example.com/b" style="text-decoration:underline">kept</a>"#
        );
        assert_eq!(plain_links(&plain), plain);
        assert_eq!(plain_links("<p>no link</p>"), "<p>no link</p>");
    }

    #[test]
    fn inline_images_are_embedded_and_unknown_ones_left_alone() {
        let png = b"\x89PNG\r\n\x1a\nrest";
        let html = r#"<img src="cid:logo@x"><img src="cid:gone@x"><img src="cid:text@x"><img src="https://x/a.png">"#;
        let embedded = embed_inline_images(html, |content_id| match content_id {
            "logo@x" => Some(png.as_slice()),
            "text@x" => Some(b"not an image".as_slice()),
            _ => None,
        });
        assert!(embedded.starts_with(r#"<img src="data:image/png;base64,"#));
        assert!(embedded.contains(r#"<img src="cid:gone@x">"#));
        assert!(embedded.contains(r#"<img src="cid:text@x">"#));
        assert!(embedded.contains(r#"<img src="https://x/a.png">"#));

        // What is embedded is sent as a part of the reply.
        let (sent, parts) = extract_data_images(&embedded, "mail1");
        assert!(sent.starts_with(r#"<img src="cid:image001.mail1@courly">"#));
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].data, png);
    }

    #[test]
    fn cid_sources_are_listed_once_in_order() {
        let html = "<img src=\"cid:logo@mail\"><IMG SRC='CID:badge@mail'>\
            <img src=\"cid:logo@mail\"><img src=\"https://example.com/a.png\">";
        assert_eq!(cid_sources(html), ["logo@mail", "badge@mail"]);
    }

    #[test]
    fn only_cid_sources_name_a_content_id() {
        assert_eq!(content_id_of("cid:logo@mail"), Some("logo@mail"));
        assert_eq!(content_id_of("data:image/png;base64,AA"), None);
        assert_eq!(content_id_of("c"), None);
    }

    #[test]
    fn images_and_tables_make_a_laid_out_signature() {
        assert!(is_laid_out("<TABLE><tr><td>MonApp</td></tr></TABLE>"));
        assert!(is_laid_out("<p>MonApp <img src=\"cid:logo@mail\"></p>"));
        assert!(!is_laid_out("<p><b>MonApp</b> support</p>"));
    }

    #[test]
    fn attribute_sizes_are_repeated_as_inline_css() {
        let html = "<p><img alt=\"icon\" width=\"12\" height=\"12\" src=\"cid:a@courly\"> Tel</p>";
        assert_eq!(
            pin_image_sizes(html),
            "<p><img style=\"width:12px;height:12px\" alt=\"icon\" width=\"12\" height=\"12\" \
             src=\"cid:a@courly\"> Tel</p>"
        );
    }

    #[test]
    fn an_existing_style_gains_only_the_missing_sizes() {
        let html = "<img width=20 height='16' style=\"border:0; width:24px\" src=x>";
        assert_eq!(
            pin_image_sizes(html),
            "<img width=20 height='16' style=\"border:0; width:24px;height:16px\" src=x>"
        );
    }

    #[test]
    fn images_already_sized_or_sized_in_percent_are_left_alone() {
        for html in [
            "<img src=x>",
            "<img width=\"100%\" src=x>",
            "<img style=\"width:12px;height:12px\" width=\"12\" height=\"12\" src=x>",
            "<img data-width=\"40\" style=\"max-width:100%\" src=x>",
        ] {
            assert_eq!(pin_image_sizes(html), html);
        }
    }

    #[test]
    fn img_without_src_is_copied() {
        let html = "<img alt=x><p>t</p>";
        assert_eq!(extract_data_images(html, "mail1").0, html);
    }
}
