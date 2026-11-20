//! Where the logo of a sender's domain is published: the BIMI record of the
//! domain, then the icons its website declares.
//!
//! Nothing here fetches a logo or decides whether one may be shown. This
//! module names the domains to ask and reads what they answer; the caller
//! owns the HTTP client, the cache and the rule for trusting a sender.

use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;
use url::Url;

const DNS_TIMEOUT: Duration = Duration::from_secs(5);

/// Mailbox providers anyone can open an address at: their logo says
/// nothing about who is writing.
const MAILBOX_PROVIDERS: &[&str] = &[
    "126.com",
    "163.com",
    "aol.com",
    "att.net",
    "bluewin.ch",
    "btinternet.com",
    "comcast.net",
    "daum.net",
    "fastmail.com",
    "free.fr",
    "gmail.com",
    "gmx.com",
    "gmx.de",
    "gmx.net",
    "googlemail.com",
    "hey.com",
    "hotmail.co.uk",
    "hotmail.com",
    "hotmail.fr",
    "icloud.com",
    "laposte.net",
    "libero.it",
    "live.com",
    "live.fr",
    "mac.com",
    "mail.com",
    "mail.ru",
    "me.com",
    "msn.com",
    "naver.com",
    "orange.fr",
    "outlook.com",
    "outlook.fr",
    "pm.me",
    "proton.me",
    "protonmail.com",
    "qq.com",
    "rambler.ru",
    "seznam.cz",
    "sfr.fr",
    "t-online.de",
    "verizon.net",
    "wanadoo.fr",
    "web.de",
    "yahoo.co.uk",
    "yahoo.com",
    "yahoo.fr",
    "yandex.com",
    "yandex.ru",
    "zoho.com",
];

/// The domains whose logo may stand for `address`, most specific first:
/// the address's own domain, then its registrable domain when that is a
/// different one (`news.example.com`, then `example.com`). The registrable
/// domain comes from the Public Suffix List, so an address at a shared host
/// (`someone.github.io`) is never given the host's own logo.
///
/// Empty for an address without a usable domain and for mailbox providers.
pub fn logo_domains(address: &str) -> Vec<String> {
    let Some((_, domain)) = address.trim().rsplit_once('@') else {
        return Vec::new();
    };
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    let well_formed = domain.len() <= 253
        && domain.contains('.')
        // An IP address is not a domain: a top-level label has a letter.
        && domain
            .rsplit('.')
            .next()
            .is_some_and(|top| top.bytes().any(|byte| byte.is_ascii_alphabetic()))
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !well_formed {
        return Vec::new();
    }
    let Some(registrable) = psl::domain_str(&domain) else {
        return Vec::new();
    };
    if MAILBOX_PROVIDERS.contains(&registrable) {
        return Vec::new();
    }
    let mut domains = vec![domain.clone()];
    if registrable != domain {
        domains.push(registrable.to_owned());
    }
    domains
}

/// The logo URL of a BIMI record (`v=BIMI1; l=https://...; a=...`).
/// `None` for anything else, for a record that declines to publish a logo
/// (`l=` empty) and for a logo that is not served over HTTPS.
pub fn bimi_logo_location(record: &str) -> Option<String> {
    let mut tags = record.split(';').map(str::trim);
    let version = tags.next()?;
    let (name, value) = version.split_once('=')?;
    if !name.trim().eq_ignore_ascii_case("v") || !value.trim().eq_ignore_ascii_case("BIMI1") {
        return None;
    }
    let location = tags.find_map(|tag| {
        let (name, value) = tag.split_once('=')?;
        name.trim().eq_ignore_ascii_case("l").then(|| value.trim())
    })?;
    let url = Url::parse(location).ok()?;
    (url.scheme() == "https" && url.host_str().is_some()).then(|| url.into())
}

/// The logo `domain` publishes through BIMI, read from the TXT record at
/// `default._bimi.<domain>`. `None` when it publishes none or DNS fails.
pub async fn bimi_logo_url(domain: &str) -> Option<String> {
    let resolver = TokioResolver::builder_tokio()
        .and_then(hickory_resolver::ResolverBuilder::build)
        .map_err(|error| log::debug!("BIMI resolver creation failed: {error}"))
        .ok()?;
    let name = format!("default._bimi.{domain}.");
    let lookup = tokio::time::timeout(DNS_TIMEOUT, resolver.txt_lookup(name))
        .await
        .ok()?
        .ok()?;
    lookup.answers().iter().find_map(|record| {
        let RData::TXT(txt) = &record.data else {
            return None;
        };
        let text: String = txt
            .txt_data
            .iter()
            .map(|part| String::from_utf8_lossy(part))
            .collect();
        bimi_logo_location(&text)
    })
}

/// An icon a web page declares for itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteIcon {
    pub url: String,
    /// Declared as `apple-touch-icon`: drawn to stand alone on a home
    /// screen, so the best of what a page offers for an avatar.
    pub touch: bool,
    /// The largest side the page declares for it, 0 when it declares none.
    pub side: u32,
}

/// The icons `html` declares in its `<link>` tags, best first: touch icons
/// before plain ones, larger before smaller. Only HTTPS URLs are kept;
/// relative ones are resolved against `page_url`.
pub fn site_icons(html: &str, page_url: &str) -> Vec<SiteIcon> {
    let Ok(base) = Url::parse(page_url) else {
        return Vec::new();
    };
    let mut icons: Vec<SiteIcon> = Vec::new();
    let mut rest = html;
    while let Some(start) = find_ignore_case(rest, "<link") {
        let tag_and_after = &rest[start..];
        let Some(end) = tag_and_after.find('>') else {
            break;
        };
        let tag = &tag_and_after[..end];
        rest = &tag_and_after[end..];

        let Some(rel) = attribute(tag, "rel") else {
            continue;
        };
        let rel = rel.to_ascii_lowercase();
        let mut kinds = rel.split_ascii_whitespace();
        let touch = kinds
            .clone()
            .any(|kind| kind == "apple-touch-icon" || kind == "apple-touch-icon-precomposed");
        if !touch && !kinds.any(|kind| kind == "icon") {
            continue;
        }
        let Some(url) = attribute(tag, "href").and_then(|href| base.join(href.trim()).ok()) else {
            continue;
        };
        if url.scheme() != "https" {
            continue;
        }
        let side = attribute(tag, "sizes").map_or(0, largest_side);
        let url = String::from(url);
        if icons.iter().all(|icon| icon.url != url) {
            icons.push(SiteIcon { url, touch, side });
        }
    }
    icons.sort_by_key(|icon| std::cmp::Reverse((icon.touch, icon.side)));
    icons
}

/// Byte offset of the first ASCII-case-insensitive match of `needle`.
fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The value of attribute `name` in an HTML start tag, quoted or bare.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let at = find_ignore_case(rest, name)?;
        let before = rest[..at].bytes().last();
        let after = rest[at + name.len()..].trim_start();
        rest = &rest[at + name.len()..];
        // A whole attribute name, not the tail of another (`href` inside
        // `data-href`), followed by its `=`.
        if !before.is_some_and(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        return match value.chars().next()? {
            quote @ ('"' | '\'') => {
                let inner = &value[1..];
                inner.find(quote).map(|close| &inner[..close])
            }
            // A bare value runs to the next space; the tag was cut before
            // its closing bracket.
            _ => value.split_ascii_whitespace().next(),
        };
    }
}

/// The largest side in a `sizes` attribute (`"32x32 180x180"`), 0 for
/// `any` or anything unreadable.
fn largest_side(sizes: &str) -> u32 {
    sizes
        .split_ascii_whitespace()
        .filter_map(|size| {
            let (width, _) = size.split_once(['x', 'X'])?;
            width.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subdomain_is_asked_first_then_its_registrable_domain() {
        assert_eq!(
            logo_domains("News@Mail.Example.co.uk"),
            vec!["mail.example.co.uk", "example.co.uk"]
        );
        assert_eq!(logo_domains("team@example.com"), vec!["example.com"]);
    }

    #[test]
    fn a_tenant_of_a_shared_host_never_gets_the_hosts_logo() {
        assert_eq!(
            logo_domains("someone@project.github.io"),
            vec!["project.github.io"]
        );
    }

    #[test]
    fn mailbox_providers_and_unusable_domains_have_no_logo() {
        assert!(logo_domains("someone@gmail.com").is_empty());
        assert!(logo_domains("someone@mail.yahoo.fr").is_empty());
        assert!(logo_domains("someone@localhost").is_empty());
        assert!(logo_domains("someone@192.168.1.1").is_empty());
        assert!(logo_domains("someone@exa mple.com").is_empty());
        assert!(logo_domains("someone@example.com/../x").is_empty());
        assert!(logo_domains("no-at-sign").is_empty());
    }

    #[test]
    fn a_bimi_record_gives_its_https_logo() {
        assert_eq!(
            bimi_logo_location(
                "v=BIMI1; l=https://example.com/logo.svg; a=https://example.com/vmc.pem"
            ),
            Some("https://example.com/logo.svg".to_owned())
        );
        assert_eq!(
            bimi_logo_location("V=bimi1;L=https://example.com/l.svg"),
            Some("https://example.com/l.svg".to_owned())
        );
    }

    #[test]
    fn a_record_that_declines_or_is_not_bimi_gives_nothing() {
        assert_eq!(bimi_logo_location("v=BIMI1; l=; a="), None);
        assert_eq!(
            bimi_logo_location("v=BIMI1; l=http://example.com/logo.svg"),
            None
        );
        assert_eq!(bimi_logo_location("v=spf1 include:example.com ~all"), None);
        assert_eq!(bimi_logo_location(""), None);
    }

    #[test]
    fn touch_icons_come_first_then_the_largest() {
        let html = r#"<html><head>
            <LINK REL="shortcut icon" href="/favicon.ico">
            <link rel=icon sizes=32x32 href=/icon-32.png>
            <link href="https://cdn.example.com/touch.png" sizes="120x120 180x180" rel="apple-touch-icon">
            <link rel="stylesheet" href="/style.css">
            <link rel="icon" data-href="/wrong.png" href='icons/192.png' sizes="192x192">
        </head></html>"#;
        let icons = site_icons(html, "https://example.com/welcome/");
        let urls: Vec<&str> = icons.iter().map(|icon| icon.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://cdn.example.com/touch.png",
                "https://example.com/welcome/icons/192.png",
                "https://example.com/icon-32.png",
                "https://example.com/favicon.ico",
            ]
        );
        assert!(icons[0].touch);
        assert_eq!(icons[0].side, 180);
    }

    #[test]
    fn icons_served_without_tls_are_left_out() {
        let html = r#"<link rel="icon" href="http://example.com/icon.png">
                      <link rel="icon" href="data:image/png;base64,AAAA">"#;
        assert!(site_icons(html, "https://example.com/").is_empty());
    }
}
