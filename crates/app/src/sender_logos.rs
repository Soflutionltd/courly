//! Brand logos shown beside the messages of senders the mailbox knows.
//!
//! A logo is what the sender's domain publishes: its BIMI record, else the
//! icon of its website. Neither is verified by a third party, so a lookalike
//! domain could publish a copied logo. A logo is therefore shown only for an
//! address that has written for some time (or is a contact), whose recent
//! mail passes DMARC and never fails it. See [`may_show_logo`].
//!
//! Logos are small PNGs in the data directory, named after their domain, with
//! an empty `.none` file for a domain that publishes nothing. Views read them
//! from memory; the files are read once at boot, off the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use iced::widget::image::Handle;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};
use rtsk::db::queries_extra::{SenderHistory, sender_history_sync};
use rtsk::sender_logo;

use crate::db::Db;

const DIRECTORY: &str = "sender_logos";
const LOGO_EXTENSION: &str = "png";
/// Marks a domain that was asked and publishes no usable logo.
const NONE_EXTENSION: &str = "none";

/// Side of a stored logo, in pixels: twice the largest avatar that shows one
/// on a double-density display.
const LOGO_SIDE: u32 = 128;
const LOGO_SIDE_F32: f32 = 128.0;
/// Anything smaller is a browser-tab icon: blurred once drawn as an avatar.
const MIN_SOURCE_SIDE: u32 = 32;
/// A logo this close to square fills the avatar; a wider or taller one is
/// fitted inside it instead of being cropped.
const NEAR_SQUARE: f64 = 1.25;

const MAX_LOGO_BYTES: usize = 1024 * 1024;
const MAX_PAGE_BYTES: usize = 512 * 1024;
/// Icons a page declares that are tried before giving up on the page.
const PAGE_ICONS_TRIED: usize = 3;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// A stored logo is fetched again after this long.
const LOGO_LIFETIME: Duration = DAY.saturating_mul(90);
/// A domain without a logo is asked again after this long.
const NONE_LIFETIME: Duration = DAY.saturating_mul(30);
/// How long an address must have been writing before it gets a logo.
const KNOWN_AFTER: Duration = DAY.saturating_mul(30);
/// The DMARC verdicts that count are those of the last year.
const DMARC_WINDOW: Duration = DAY.saturating_mul(365);
/// An address is judged again after this long: a forged message that fails
/// DMARC takes the logo off within this delay.
const JUDGED_FOR: Duration = Duration::from_secs(10 * 60);

/// Domains asked for a logo each time a list loads, and how many at once.
const DOMAINS_PER_WARM: usize = 12;
const CONCURRENT_FETCHES: usize = 6;
/// A site that takes longer than this to answer one request is skipped: a
/// slow site must not hold back the logos of the others.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

static ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Default)]
struct Store {
    /// Domain to its logo.
    logos: HashMap<String, Handle>,
    /// Lowercased address to the domain whose logo it shows.
    shown: HashMap<String, String>,
    /// When each lowercased address was last judged.
    judged: HashMap<String, Instant>,
    /// Domains asked without success, not to be asked again for now.
    without_logo: HashSet<String>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(|| RwLock::new(Store::default()));

/// Turn logos on or off. Off, nothing is shown and no request leaves.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The brand logo to draw for this sender, if it earned one.
pub fn logo_of(address: &str) -> Option<Handle> {
    if !is_enabled() {
        return None;
    }
    let store = STORE.read().unwrap_or_else(PoisonError::into_inner);
    if store.shown.is_empty() {
        return None;
    }
    let domain = store.shown.get(&address.trim().to_lowercase())?;
    store.logos.get(domain).cloned()
}

/// Whether a brand logo may stand beside mail from an address with this
/// history: the address is no newcomer (it has written for [`KNOWN_AFTER`],
/// or it is a contact), its recent mail passed DMARC at least once, and
/// none of it failed.
fn may_show_logo(history: SenderHistory, now_ms: i64) -> bool {
    let long_known = history
        .first_message_ms
        .is_some_and(|first| now_ms.saturating_sub(first) >= millis(KNOWN_AFTER));
    (long_known || history.is_contact) && history.dmarc_passes > 0 && history.dmarc_failures == 0
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Read the stored logos into memory, on a thread of its own. Files past
/// their lifetime are removed, so they are fetched again.
pub fn load(app_data_dir: &Path) {
    let directory = app_data_dir.join(DIRECTORY);
    let spawned = std::thread::Builder::new()
        .name("sender-logos".into())
        .spawn(move || {
            if let Err(error) = load_directory(&directory) {
                log::error!("Sender logos: {error}");
            }
        });
    if let Err(error) = spawned {
        log::error!("Sender logos: the loading thread did not start: {error}");
    }
}

fn load_directory(directory: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("reading {}: {error}", directory.display())),
    };
    let mut logos = HashMap::new();
    let mut without_logo = HashSet::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("reading {}: {error}", directory.display()))?
            .path();
        let (Some(domain), Some(extension)) = (
            path.file_stem().and_then(|stem| stem.to_str()),
            path.extension().and_then(|extension| extension.to_str()),
        ) else {
            continue;
        };
        let lifetime = match extension {
            LOGO_EXTENSION => LOGO_LIFETIME,
            NONE_EXTENSION => NONE_LIFETIME,
            _ => continue,
        };
        let age = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map(|modified| {
                SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default()
            })
            .map_err(|error| format!("reading {}: {error}", path.display()))?;
        if age > lifetime {
            std::fs::remove_file(&path)
                .map_err(|error| format!("removing {}: {error}", path.display()))?;
        } else if extension == LOGO_EXTENSION {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            logos.insert(domain.to_owned(), Handle::from_bytes(bytes));
        } else {
            without_logo.insert(domain.to_owned());
        }
    }
    let mut store = STORE.write().unwrap_or_else(PoisonError::into_inner);
    store.logos.extend(logos);
    store.without_logo.extend(without_logo);
    Ok(())
}

/// Judge the senders of a list that just loaded and fetch the logos still
/// missing for those that earned one. Returns once the fetches are done.
pub async fn warm(db: Arc<Db>, addresses: Vec<String>) {
    if !is_enabled() {
        return;
    }
    let Some(app_data_dir) = crate::APP_DATA_DIR.get() else {
        return;
    };
    let due = addresses_to_judge(addresses);
    if due.is_empty() {
        return;
    }

    let now_ms = jiff::Timestamp::now().as_millisecond();
    let dmarc_since_ms = now_ms.saturating_sub(millis(DMARC_WINDOW));
    let to_judge = due.clone();
    let judged = db
        .with_read(move |conn| {
            to_judge
                .into_iter()
                .map(|address| {
                    let history = sender_history_sync(conn, &address, dmarc_since_ms)?;
                    Ok((address, may_show_logo(history, now_ms)))
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .await;
    let judged = match judged {
        Ok(judged) => judged,
        Err(error) => {
            log::error!("Sender logos: judging senders failed: {error}");
            forget_judgement(&due);
            return;
        }
    };

    let wanted = apply_judgement(judged);
    let Some(client) = crate::ui::mail_body::public_http_client() else {
        return;
    };
    let directory = app_data_dir.join(DIRECTORY);
    let mut fetches = tokio::task::JoinSet::new();
    let permits = Arc::new(tokio::sync::Semaphore::new(CONCURRENT_FETCHES));
    for (address, domains) in wanted {
        let directory = directory.clone();
        let permits = Arc::clone(&permits);
        fetches.spawn(async move {
            let Ok(_permit) = permits.acquire().await else {
                return;
            };
            fetch_for_address(client, &directory, &address, &domains).await;
        });
    }
    while let Some(finished) = fetches.join_next().await {
        if let Err(error) = finished {
            log::error!("Sender logos: a fetch did not finish: {error}");
        }
    }
}

/// The addresses of `addresses` not judged recently, lowercased and marked
/// as judged now.
fn addresses_to_judge(addresses: Vec<String>) -> Vec<String> {
    let now = Instant::now();
    let mut store = STORE.write().unwrap_or_else(PoisonError::into_inner);
    let mut due = Vec::new();
    for address in addresses {
        let address = address.trim().to_lowercase();
        if sender_logo::logo_domains(&address).is_empty() {
            continue;
        }
        let fresh = store
            .judged
            .get(&address)
            .is_some_and(|at| now.duration_since(*at) < JUDGED_FOR);
        if !fresh {
            store.judged.insert(address.clone(), now);
            due.push(address);
        }
    }
    due
}

fn forget_judgement(addresses: &[String]) {
    let mut store = STORE.write().unwrap_or_else(PoisonError::into_inner);
    for address in addresses {
        store.judged.remove(address);
    }
}

/// Record who may show a logo. An address that may and whose logo is in
/// memory shows it from now on; one that may not loses it. Returns the
/// addresses whose logo is still to fetch, with the domains to ask, at most
/// [`DOMAINS_PER_WARM`] of them: the rest are judged again at the next load.
fn apply_judgement(judged: Vec<(String, bool)>) -> Vec<(String, Vec<String>)> {
    let mut store = STORE.write().unwrap_or_else(PoisonError::into_inner);
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    let mut asked: HashSet<String> = HashSet::new();
    for (address, may_show) in judged {
        if !may_show {
            store.shown.remove(&address);
            continue;
        }
        let domains = sender_logo::logo_domains(&address);
        if let Some(domain) = domains
            .iter()
            .find(|domain| store.logos.contains_key(*domain))
        {
            store.shown.insert(address, domain.clone());
            continue;
        }
        let to_ask: Vec<String> = domains
            .into_iter()
            .filter(|domain| !store.without_logo.contains(domain))
            .collect();
        let Some(first) = to_ask.first() else {
            continue;
        };
        if asked.contains(first) || asked.len() < DOMAINS_PER_WARM {
            asked.insert(first.clone());
            wanted.push((address, to_ask));
        } else {
            store.judged.remove(&address);
        }
    }
    wanted
}

/// Ask `domains` in turn for a logo, store the first one found and show it
/// for `address`. A domain that has none is remembered as such.
async fn fetch_for_address(
    client: &reqwest::Client,
    directory: &Path,
    address: &str,
    domains: &[String],
) {
    for domain in domains {
        let already = {
            let store = STORE.read().unwrap_or_else(PoisonError::into_inner);
            if store.logos.contains_key(domain) {
                Some(true)
            } else if store.without_logo.contains(domain) {
                Some(false)
            } else {
                None
            }
        };
        let found = match already {
            Some(found) => found,
            None => {
                let logo = fetch_logo(client, domain).await;
                match remember(directory, domain, logo) {
                    Ok(found) => found,
                    Err(error) => {
                        log::error!("Sender logos: {error}");
                        return;
                    }
                }
            }
        };
        if found {
            STORE
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .shown
                .insert(address.to_owned(), domain.clone());
            return;
        }
    }
}

fn logo_file(directory: &Path, domain: &str, extension: &str) -> PathBuf {
    directory.join(format!("{domain}.{extension}"))
}

/// Write what `domain` answered to disk and to memory. `Ok(true)` when it
/// has a logo.
fn remember(directory: &Path, domain: &str, logo: Option<Vec<u8>>) -> Result<bool, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("creating {}: {error}", directory.display()))?;
    let mut store = STORE.write().unwrap_or_else(PoisonError::into_inner);
    match logo {
        Some(png) => {
            let file = logo_file(directory, domain, LOGO_EXTENSION);
            std::fs::write(&file, &png)
                .map_err(|error| format!("writing {}: {error}", file.display()))?;
            store
                .logos
                .insert(domain.to_owned(), Handle::from_bytes(png));
            Ok(true)
        }
        None => {
            let file = logo_file(directory, domain, NONE_EXTENSION);
            std::fs::write(&file, b"")
                .map_err(|error| format!("writing {}: {error}", file.display()))?;
            store.without_logo.insert(domain.to_owned());
            Ok(false)
        }
    }
}

/// The logo `domain` publishes, as a PNG ready to store: its BIMI logo,
/// else its touch icon, else the best icon its home page declares, else
/// its favicon.
async fn fetch_logo(client: &reqwest::Client, domain: &str) -> Option<Vec<u8>> {
    if let Some(url) = sender_logo::bimi_logo_url(domain).await
        && let Some(logo) = download_logo(client, &url).await
    {
        return Some(logo);
    }
    let origin = format!("https://{domain}/");
    if let Some(logo) = download_logo(client, &format!("{origin}apple-touch-icon.png")).await {
        return Some(logo);
    }
    if let Some((page, page_url)) = download(client, &origin, MAX_PAGE_BYTES).await {
        let html = String::from_utf8_lossy(&page);
        for icon in sender_logo::site_icons(&html, &page_url)
            .into_iter()
            .take(PAGE_ICONS_TRIED)
        {
            if let Some(logo) = download_logo(client, &icon.url).await {
                return Some(logo);
            }
        }
    }
    download_logo(client, &format!("{origin}favicon.ico")).await
}

async fn download_logo(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let (bytes, _) = download(client, url, MAX_LOGO_BYTES).await?;
    stored_png(&bytes)
}

/// The body of `url` and the URL it was finally served from. `None` for
/// anything but a success, and for a body larger than `max_bytes`.
async fn download(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Option<(Vec<u8>, String)> {
    let mut response = client
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| log::debug!("Sender logos: {url}: {error}"))
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let served_from = response.url().to_string();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > max_bytes {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some((body, served_from))
}

/// Turn a downloaded logo (SVG or raster) into the PNG that is stored.
/// `None` for anything that is not an image or is too small to draw.
fn stored_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let picture = if looks_like_svg(bytes) {
        rasterized_svg(bytes)?
    } else {
        image::load_from_memory(bytes).ok()?
    };
    let (width, height) = (picture.width(), picture.height());
    if width.min(height) < MIN_SOURCE_SIDE {
        return None;
    }
    let side = LOGO_SIDE.min(width.max(height));
    let ratio = f64::from(width.max(height)) / f64::from(width.min(height));
    let square = if ratio <= NEAR_SQUARE {
        picture.resize_to_fill(side, side, FilterType::Lanczos3)
    } else {
        let fitted = picture.resize(side, side, FilterType::Lanczos3).to_rgba8();
        let mut canvas = RgbaImage::new(side, side);
        image::imageops::overlay(
            &mut canvas,
            &fitted,
            i64::from((side - fitted.width()) / 2),
            i64::from((side - fitted.height()) / 2),
        );
        DynamicImage::ImageRgba8(canvas)
    };
    let mut png = std::io::Cursor::new(Vec::new());
    square.write_to(&mut png, ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let head = String::from_utf8_lossy(head);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// Draw an SVG centered in a [`LOGO_SIDE`] square. The SVG comes from the
/// network: it may embed images, but may not name files to load.
fn rasterized_svg(bytes: &[u8]) -> Option<DynamicImage> {
    use resvg::{tiny_skia, usvg};

    let mut options = usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let size = tree.size();
    let side = LOGO_SIDE_F32;
    let scale = side / size.width().max(size.height());
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (side - size.width() * scale) / 2.0,
        (side - size.height() * scale) / 2.0,
    );
    let mut pixmap = tiny_skia::Pixmap::new(LOGO_SIDE, LOGO_SIDE)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    image::load_from_memory(&pixmap.encode_png().ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: i64 = 1_800_000_000_000;

    fn history(days_known: u32, passes: i64, failures: i64, is_contact: bool) -> SenderHistory {
        SenderHistory {
            first_message_ms: Some(NOW_MS - millis(DAY.saturating_mul(days_known))),
            dmarc_passes: passes,
            dmarc_failures: failures,
            is_contact,
        }
    }

    #[test]
    fn a_long_known_sender_whose_mail_passes_dmarc_gets_a_logo() {
        assert!(may_show_logo(history(45, 3, 0, false), NOW_MS));
    }

    #[test]
    fn a_newcomer_gets_no_logo_unless_it_is_a_contact() {
        assert!(!may_show_logo(history(2, 3, 0, false), NOW_MS));
        assert!(may_show_logo(history(2, 3, 0, true), NOW_MS));
    }

    #[test]
    fn one_dmarc_failure_takes_the_logo_off() {
        assert!(!may_show_logo(history(400, 50, 1, true), NOW_MS));
    }

    #[test]
    fn mail_that_was_never_authenticated_gets_no_logo() {
        assert!(!may_show_logo(history(400, 0, 0, true), NOW_MS));
        let never_written = SenderHistory {
            first_message_ms: None,
            dmarc_passes: 0,
            dmarc_failures: 0,
            is_contact: true,
        };
        assert!(!may_show_logo(never_written, NOW_MS));
    }

    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(width, height, image::Rgb([12, 127, 125]))
            .write_to(&mut png, ImageFormat::Png)
            .expect("encode the source picture");
        png.into_inner()
    }

    fn side_of(png: &[u8]) -> (u32, u32) {
        let stored = image::load_from_memory(png).expect("stored logo decodes");
        (stored.width(), stored.height())
    }

    #[test]
    fn a_large_icon_is_stored_as_a_square_of_the_logo_side() {
        let stored = stored_png(&png_of(512, 512)).expect("logo");
        assert_eq!(side_of(&stored), (LOGO_SIDE, LOGO_SIDE));
    }

    #[test]
    fn a_wide_logo_is_fitted_not_cropped() {
        let stored = stored_png(&png_of(400, 100)).expect("logo");
        assert_eq!(side_of(&stored), (LOGO_SIDE, LOGO_SIDE));
        let stored = image::load_from_memory(&stored)
            .expect("decodes")
            .to_rgba8();
        assert_eq!(stored.get_pixel(LOGO_SIDE / 2, 0).0[3], 0, "padding above");
        assert_eq!(
            stored.get_pixel(0, LOGO_SIDE / 2).0[3],
            255,
            "the full width is kept"
        );
    }

    #[test]
    fn a_small_icon_keeps_its_size_and_a_tab_icon_is_refused() {
        assert_eq!(
            side_of(&stored_png(&png_of(64, 64)).expect("logo")),
            (64, 64)
        );
        assert!(stored_png(&png_of(16, 16)).is_none());
    }

    #[test]
    fn an_svg_logo_is_drawn_and_a_web_page_is_not_a_logo() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
            <rect width="10" height="10" fill="#0c7f7d"/></svg>"##;
        let stored = stored_png(svg).expect("logo");
        assert_eq!(side_of(&stored), (LOGO_SIDE, LOGO_SIDE));
        assert!(stored_png(b"<!doctype html><html><body>Not found</body></html>").is_none());
    }

    #[test]
    fn an_svg_cannot_pull_a_local_file_into_the_logo() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"
            xmlns:xlink="http://www.w3.org/1999/xlink">
            <image width="10" height="10" xlink:href="/etc/hosts"/></svg>"#;
        let stored = stored_png(svg).expect("an empty drawing is still a picture");
        let stored = image::load_from_memory(&stored)
            .expect("decodes")
            .to_rgba8();
        assert!(stored.pixels().all(|pixel| pixel.0[3] == 0));
    }
}
