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
        encoding_rs::Encoding::for_label(rest[..end].as_bytes())
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
fn embed_local_images(html: &str, base: &Path) -> String {
    rewrite_img_sources(html, |src| {
        let lower = src.to_ascii_lowercase();
        if ["http:", "https:", "cid:", "data:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
        {
            return None;
        }
        let relative = percent_decode(src.trim_start_matches("file://"));
        let path = base.join(relative);
        let bytes = std::fs::read(&path).ok()?;
        let mime = mime_guess::from_path(&path)
            .first_raw()
            .unwrap_or("application/octet-stream");
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{mime};base64,{encoded}"))
    })
}

/// Move `data:` images into inline parts referenced by `cid:`, which every
/// mail client shows (several webmails drop `data:` images).
pub fn extract_data_images(html: &str) -> (String, Vec<InlineImage>) {
    let mut images = Vec::new();
    let rewritten = rewrite_img_sources(html, |src| {
        let rest = src.strip_prefix("data:")?;
        let (meta, payload) = rest.split_once(',')?;
        let mime = meta.strip_suffix(";base64")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?;
        let number = images.len() + 1;
        let content_id = format!("image{number:03}@ratatoskr");
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
        std::fs::write(files.join("image001.png"), [0x89, b'P', b'N', b'G']).expect("png");
        let html = b"<html><head><meta charset=windows-1252><style>p{margin:0}</style></head>\
            <body><p>H\xf4tel</p><img src=\"La%20Villa_files/image001.png\"></body></html>";
        let path = dir.join("La Villa.htm");
        std::fs::write(&path, html).expect("htm");

        let imported = import_signature_file(&path).expect("import");
        assert!(imported.starts_with("<style>p{margin:0}</style><p>H\u{f4}tel</p>"));
        assert!(imported.contains("src=\"data:image/png;base64,iVBORw==\""));
    }

    #[test]
    fn remote_images_are_left_alone() {
        let html = "<img src=\"https://example.com/logo.png\" alt=\"x\">";
        assert_eq!(embed_local_images(html, Path::new("/nowhere")), html);
    }

    #[test]
    fn data_images_become_inline_parts() {
        let html = "<p>A</p><img alt=x src=\"data:image/png;base64,iVBORw==\"><IMG SRC='data:image/gif;base64,R0lG'>";
        let (rewritten, images) = extract_data_images(html);
        assert_eq!(
            rewritten,
            "<p>A</p><img alt=x src=\"cid:image001@ratatoskr\"><IMG SRC='cid:image002@ratatoskr'>"
        );
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(images[0].data, vec![0x89, b'P', b'N', b'G']);
        assert_eq!(images[1].mime_type, "image/gif");
    }

    #[test]
    fn img_without_src_is_copied() {
        let html = "<img alt=x><p>t</p>";
        assert_eq!(extract_data_images(html).0, html);
    }
}
