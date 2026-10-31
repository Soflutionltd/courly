//! HTML email rendering on top of the Blitz engine (Stylo CSS, Taffy layout,
//! Parley text, Vello CPU raster).
//!
//! The renderer is deliberately static: no scripts, no forms, no network
//! unless the caller opts in per message. Output is a set of RGBA tiles plus
//! the hit areas of every link, so the UI layer only has to draw images.

mod fonts;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, UserAgentStylesheet, util::Color};
use blitz_html::DocumentHtmlParser;
use blitz_paint::paint_scene;
pub use blitz_traits::net::{Bytes, Url};
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use html5ever::tendril::TendrilSink as _;
use html5ever::tree_builder::{QuirksMode, TreeBuilderOpts};
use html5ever::{ParseOpts, parse_document};
use num_traits::ToPrimitive as _;
use peniko::Fill;
use peniko::kurbo::Rect;

/// Device pixel ratio used for normal-length emails.
const HIDPI_SCALE: f32 = 2.0;
/// Emails taller than this (CSS px) are rasterised at 1x to bound memory.
const HIDPI_MAX_HEIGHT: f32 = 6_000.0;
/// Hard cap on rendered height (CSS px); anything below is cut off.
const MAX_HEIGHT: f32 = 30_000.0;
/// Hard cap on rendered width (display px) for emails wider than the pane.
const MAX_WIDTH: f32 = 2_400.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
/// Height of each RGBA tile in device pixels. Kept under common GPU
/// texture limits.
const TILE_HEIGHT_PX: u32 = 4_096;
/// Viewport height handed to layout. Only matters for `vh` units and
/// `height: 100%` on the root; content is measured by overflow afterwards.
const LAYOUT_VIEWPORT_HEIGHT: u32 = 800;
/// How long to wait for opted-in remote images before painting anyway.
const REMOTE_DEADLINE: Duration = Duration::from_secs(10);
const REMOTE_POLL: Duration = Duration::from_millis(15);
/// Base URL for relative references. Without one Blitz panics on the first
/// relative `src`; `.invalid` never resolves, so those resources are skipped
/// while protocol-relative URLs (`//cdn...`) still resolve to https.
const BASE_HOST: &str = "mail.invalid";

thread_local! {
    static RENDERING: Cell<bool> = const { Cell::new(false) };
}

/// True while the current thread is inside [`render`]. A panic hook uses it
/// to let a Blitz failure on one email unwind into a plain-text fallback
/// instead of aborting the process.
pub fn is_rendering() -> bool {
    RENDERING.with(Cell::get)
}

struct RenderingGuard;

impl RenderingGuard {
    fn enter() -> Self {
        RENDERING.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for RenderingGuard {
    fn drop(&mut self) {
        RENDERING.with(|flag| flag.set(false));
    }
}

/// Rules layered over Blitz's default user-agent sheet.
///
/// `td, th` are forced back to table cells because responsive email
/// templates set `display: block !important` on cells, which Blitz's table
/// layout drops entirely. User-agent `!important` outranks author
/// `!important`, so this wins over inline styles.
///
/// Blitz does not map the legacy `align="center"` attribute on tables, which
/// most newsletter and receipt templates use to center their 600px column.
const EMAIL_UA_CSS: &str = r#"
html {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    font-size: 14px;
    color: #1f1f1f;
}
td, th { display: table-cell !important; }
table[align=center] { margin-left: auto; margin-right: auto; }
"#;

static EMAIL_UA_SHEET: LazyLock<UserAgentStylesheet> =
    LazyLock::new(|| UserAgentStylesheet::parse(EMAIL_UA_CSS));

/// Fetches a remote resource. Only called for `http`/`https` URLs, and only
/// when the caller allowed remote content for this message.
pub type RemoteFetcher = Arc<dyn Fn(&Url) -> Option<Bytes> + Send + Sync>;

pub struct RenderRequest {
    pub html: String,
    /// Available width in display px.
    pub width: f32,
    /// Reading zoom. The email is laid out at `width / zoom` CSS px and
    /// painted scaled by `zoom`, like a browser's page zoom.
    pub zoom: f32,
    /// Inline parts keyed by Content-ID (without the `cid:` prefix).
    pub inline_images: HashMap<String, Bytes>,
    pub remote: Option<RemoteFetcher>,
}

#[derive(Debug, Clone)]
pub struct Tile {
    /// Offset from the top of the email, in CSS px.
    pub y: f32,
    /// Height in CSS px.
    pub height: f32,
    pub px_width: u32,
    pub px_height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LinkArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub href: String,
}

/// Sizes and positions are in display px, so already multiplied by the zoom.
#[derive(Debug, Clone)]
pub struct RenderedMail {
    /// Rendered width. Larger than the requested width when the email has a
    /// fixed layout wider than the pane.
    pub width: f32,
    pub height: f32,
    pub tiles: Vec<Tile>,
    pub links: Vec<LinkArea>,
    /// Remote resources that were requested but not loaded.
    pub blocked_remote: usize,
}

/// Remote resources fetched per message once the user allows them. Each
/// fetch runs on its own thread, so a message listing thousands of images
/// must not get thousands of threads.
const MAX_REMOTE_RESOURCES: usize = 200;

struct MailNetProvider {
    inline_images: HashMap<String, Bytes>,
    remote: Option<RemoteFetcher>,
    blocked: AtomicUsize,
    pending: Arc<AtomicUsize>,
    remote_requested: AtomicUsize,
}

/// Every request is answered, with empty bytes when the resource is skipped,
/// blocked or fails. Blitz holds back all layout while a `<head>` stylesheet
/// is unanswered, so a blocked `<link rel=stylesheet>` (web fonts, mostly)
/// would otherwise leave the whole email blank.
impl NetProvider for MailNetProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url;
        let bytes = match url.scheme() {
            "cid" => {
                let cid = url.path().trim_start_matches('<').trim_end_matches('>');
                self.inline_images.get(cid).cloned()
            }
            "data" => data_url::DataUrl::process(url.as_str())
                .ok()
                .and_then(|data| data.decode_to_vec().ok())
                .map(|(body, _)| Bytes::from(body)),
            "http" | "https" if url.host_str() == Some(BASE_HOST) => None,
            "http" | "https" => match &self.remote {
                Some(_)
                    if self.remote_requested.fetch_add(1, Ordering::Relaxed)
                        >= MAX_REMOTE_RESOURCES =>
                {
                    None
                }
                Some(fetch) => {
                    let fetch = Arc::clone(fetch);
                    let pending = Arc::clone(&self.pending);
                    pending.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let bytes = fetch(&url).unwrap_or_default();
                        handler.bytes(url.to_string(), bytes);
                        pending.fetch_sub(1, Ordering::SeqCst);
                    });
                    return;
                }
                None => {
                    self.blocked.fetch_add(1, Ordering::Relaxed);
                    None
                }
            },
            _ => None,
        };
        handler.bytes(url.to_string(), bytes.unwrap_or_default());
    }
}

/// Logical size times scale, as a whole number of device pixels (at least
/// one; NaN maps to one).
fn device_pixels(value: f32) -> u32 {
    value.round().to_u32().unwrap_or(1).max(1)
}

/// A viewport `width` CSS px wide. Text is shaped at `hidpi * zoom`, so
/// the scene must be painted at that same scale.
fn layout_viewport(width: f32, zoom: f32, hidpi: f32) -> Viewport {
    let scale = hidpi * zoom;
    let mut viewport = Viewport::new(
        device_pixels(width * scale),
        device_pixels(LAYOUT_VIEWPORT_HEIGHT as f32 * scale),
        hidpi,
        ColorScheme::Light,
    );
    viewport.set_zoom(zoom);
    viewport
}

/// Parse a mail body with the HTML parser. Mail is served as `text/html`,
/// so an XHTML doctype or namespace must not switch to the XML parser the
/// way `HtmlDocument::from_html` does: a wrong namespace URL (common in
/// templates) would leave every element unstyled.
fn parse_mail(html: &str, config: DocumentConfig) -> BaseDocument {
    let mut document = BaseDocument::new(config);
    let mut mutator = document.mutate();
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            drop_doctype: true,
            quirks_mode: QuirksMode::NoQuirks,
            ..Default::default()
        },
        ..Default::default()
    };
    if let Err(error) = parse_document(DocumentHtmlParser::new(&mut mutator), opts)
        .from_utf8()
        .read_from(&mut html.as_bytes())
    {
        log::warn!("mail body parse stopped early: {error}");
    }
    drop(mutator);
    document
}

/// Width and height of the laid out page in display px.
fn content_size(document: &BaseDocument, width: f32, zoom: f32) -> (f32, f32) {
    let layout = document.root_element().final_layout();
    let height = layout
        .size
        .height
        .max(layout.scrollable_overflow_rect.bottom)
        .clamp(1.0, MAX_HEIGHT)
        .ceil();
    let content_width = width
        .max(layout.scrollable_overflow_rect.right)
        .min(MAX_WIDTH / zoom)
        .ceil();
    ((content_width * zoom).ceil(), (height * zoom).ceil())
}

/// Lay out and rasterise one HTML email. CPU only; call from a blocking
/// worker, never from the UI thread.
pub fn render(request: RenderRequest) -> RenderedMail {
    let _rendering = RenderingGuard::enter();
    let zoom = if request.zoom.is_finite() {
        request.zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    } else {
        1.0
    };
    let width = (request.width.clamp(1.0, MAX_WIDTH) / zoom)
        .floor()
        .max(1.0);
    let pending = Arc::new(AtomicUsize::new(0));
    let net = Arc::new(MailNetProvider {
        inline_images: request.inline_images,
        remote: request.remote,
        blocked: AtomicUsize::new(0),
        pending: Arc::clone(&pending),
        remote_requested: AtomicUsize::new(0),
    });

    let mut document = parse_mail(
        &request.html,
        DocumentConfig {
            viewport: Some(layout_viewport(width, zoom, HIDPI_SCALE)),
            base_url: Some(format!("https://{BASE_HOST}/")),
            net_provider: Some(Arc::clone(&net) as Arc<dyn NetProvider>),
            ua_stylesheets: vec![UserAgentStylesheet::default_sheet(), EMAIL_UA_SHEET.clone()],
            font_ctx: Some(fonts::context_for(&request.html)),
            ..Default::default()
        },
    );

    let deadline = Instant::now() + REMOTE_DEADLINE;
    loop {
        document.resolve(0.0);
        if pending.load(Ordering::SeqCst) == 0 || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(REMOTE_POLL);
    }
    document.resolve(0.0);

    let (mut content_width, mut content_height) = content_size(&document, width, zoom);
    let scale = if content_height > HIDPI_MAX_HEIGHT {
        document.set_viewport(layout_viewport(width, zoom, 1.0));
        document.resolve(0.0);
        (content_width, content_height) = content_size(&document, width, zoom);
        1.0
    } else {
        HIDPI_SCALE
    };
    let px_width = device_pixels(content_width * scale);
    let px_height = device_pixels(content_height * scale);

    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Default::default(),
                Color::WHITE,
                Default::default(),
                &Rect::new(0.0, 0.0, f64::from(px_width), f64::from(px_height)),
            );
            paint_scene(
                scene,
                &mut document,
                f64::from(scale * zoom),
                px_width,
                px_height,
                0,
                0,
            );
        },
        px_width,
        px_height,
    );

    let tiles = split_into_tiles(&buffer, px_width, px_height, scale);
    let links = collect_links(&document, zoom);

    RenderedMail {
        width: content_width,
        height: content_height,
        tiles,
        links,
        blocked_remote: net.blocked.load(Ordering::Relaxed),
    }
}

fn split_into_tiles(buffer: &[u8], px_width: u32, px_height: u32, scale: f32) -> Vec<Tile> {
    let row_bytes = px_width as usize * 4;
    let mut tiles = Vec::new();
    let mut top = 0;
    while top < px_height {
        let rows = TILE_HEIGHT_PX.min(px_height - top);
        let start = top as usize * row_bytes;
        let end = start + rows as usize * row_bytes;
        tiles.push(Tile {
            y: top as f32 / scale,
            height: rows as f32 / scale,
            px_width,
            px_height: rows,
            rgba: buffer[start..end].to_vec(),
        });
        top += rows;
    }
    tiles
}

fn collect_links(doc: &BaseDocument, zoom: f32) -> Vec<LinkArea> {
    let Ok(anchors) = doc.query_selector_all("a[href]") else {
        return Vec::new();
    };
    anchors
        .iter()
        .filter_map(|&node_id| {
            let href = doc
                .get_node(node_id)?
                .attr(blitz_dom::local_name!("href"))?;
            let href = href.trim();
            let scheme_ok = Url::parse(href)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"));
            if !scheme_ok {
                return None;
            }
            let rect = doc.get_client_bounding_rect(node_id)?;
            if rect.width <= 0.0 || rect.height <= 0.0 {
                return None;
            }
            Some(LinkArea {
                x: rect.x.to_f32()? * zoom,
                y: rect.y.to_f32()? * zoom,
                width: rect.width.to_f32()? * zoom,
                height: rect.height.to_f32()? * zoom,
                href: href.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_html(html: &str) -> RenderedMail {
        render(RenderRequest {
            html: html.to_string(),
            width: 600.0,
            zoom: 1.0,
            inline_images: HashMap::new(),
            remote: None,
        })
    }

    #[test]
    fn zoom_lays_out_wider_and_paints_smaller() {
        let mail = render(RenderRequest {
            html: r#"<body style="margin:0"><div style="width:1000px;height:200px">
                <a href="https://e.x/">x</a></div></body>"#
                .to_string(),
            width: 600.0,
            zoom: 0.5,
            inline_images: HashMap::new(),
            remote: None,
        });
        assert!(mail.width <= 601.0, "width was {}", mail.width);
        assert!(
            (mail.height - 100.0).abs() < 2.0,
            "height was {}",
            mail.height
        );
        let covered: f32 = mail.tiles.iter().map(|t| t.height).sum();
        assert!((covered - mail.height).abs() < 1.0);
        assert!(mail.links[0].height < 15.0);
    }

    #[test]
    fn block_cells_still_render() {
        let html = r#"<table><tr><td style="display:block !important">
            <p style="height:300px">Paid</p></td></tr></table>"#;
        let mail = render_html(html);
        assert!(mail.height >= 300.0, "height was {}", mail.height);
    }

    #[test]
    fn remote_images_are_blocked_by_default() {
        let mail = render_html(r#"<img src="https://tracker.example/p.gif">"#);
        assert_eq!(mail.blocked_remote, 1);
    }

    #[test]
    fn links_are_reported_with_areas() {
        let mail = render_html(r#"<p><a href="https://example.com/x">Open</a></p>"#);
        assert_eq!(mail.links.len(), 1);
        assert_eq!(mail.links[0].href, "https://example.com/x");
        assert!(mail.links[0].width > 0.0);
    }

    #[test]
    fn fixed_width_layouts_widen_the_render() {
        let mail = render_html(r#"<table width="900"><tr><td>wide</td></tr></table>"#);
        assert!(mail.width >= 900.0, "width was {}", mail.width);
    }

    #[test]
    fn relative_image_sources_do_not_panic() {
        let mail = render_html(r#"<img src="logo.png"><img src="/a/b.gif"><p>Receipt</p>"#);
        assert_eq!(mail.blocked_remote, 0);
        assert!(!is_rendering());
    }

    #[test]
    fn protocol_relative_images_stay_remote() {
        let mail = render_html(r#"<img src="//cdn.example/p.gif">"#);
        assert_eq!(mail.blocked_remote, 1);
    }

    #[test]
    fn script_links_are_ignored() {
        let mail = render_html(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(mail.links.is_empty());
    }

    #[test]
    fn blocked_head_stylesheets_do_not_blank_the_email() {
        let html = r#"<html><head><link href="//fonts.example/css" rel="stylesheet"></head>
            <body><p style="height:300px">Security alert</p></body></html>"#;
        let mail = render_html(html);
        assert!(mail.height >= 300.0, "height was {}", mail.height);
    }

    #[test]
    fn relative_head_stylesheets_do_not_blank_the_email() {
        let html = r#"<html><head><link href="style.css" rel="stylesheet"></head>
            <body><p style="height:300px">Receipt</p></body></html>"#;
        let mail = render_html(html);
        assert!(mail.height >= 300.0, "height was {}", mail.height);
    }

    fn first_link_x(html: &str) -> f32 {
        render_html(html).links.first().expect("link rendered").x
    }

    #[test]
    fn align_center_tables_are_centered() {
        let x = first_link_x(
            r#"<table align="center" width="200"><tr><td><a href="https://e.x/">x</a></td></tr></table>"#,
        );
        assert!(x > 150.0, "link x was {x}");
    }

    #[test]
    fn tables_inside_centered_cells_are_centered() {
        let x = first_link_x(
            r#"<table width="100%"><tr><td align="center">
            <table width="200"><tr><td><a href="https://e.x/">x</a></td></tr></table>
            </td></tr></table>"#,
        );
        assert!(x > 150.0, "link x was {x}");
    }

    #[test]
    fn xhtml_doctypes_are_read_as_html() {
        let html = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "https://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd"><html xmlns="https://www.w3.org/1999/xhtml"><head>
            <meta http-equiv="Content-Type" content="text/html; charset=utf-8">
            <title>Emailing survey 02</title>
            <style>body {margin:0} p.tall {height:300px; margin:0}</style>
            </head><body><table align="center" width="200"><tr><td><a href="https://e.x/">x</a></td></tr></table>
            <p class="tall">Survey</p></body></html>"#;
        let mail = render_html(html);
        let link = mail.links.first().expect("link rendered");
        assert!(link.x > 150.0, "table not centered, link x was {}", link.x);
        assert!(
            link.y < 40.0,
            "head text pushed the body down to {}",
            link.y
        );
        assert!(
            mail.height >= 300.0,
            "stylesheet ignored, height was {}",
            mail.height
        );
    }

    #[test]
    fn center_elements_center_their_tables() {
        let x = first_link_x(
            r#"<center><table width="200"><tr><td><a href="https://e.x/">x</a></td></tr></table></center>"#,
        );
        assert!(x > 150.0, "link x was {x}");
    }

    #[test]
    fn cell_widths_from_later_rows_size_auto_tables() {
        let table = |address_style: &str| {
            render_html(&format!(
                r#"<table><tr><td style="width:96pt">+33</td></tr>
                <tr><td style="font-size:10pt;{address_style}">Chemin notre dame de la serra, 20260 CALVI</td></tr></table>"#
            ))
        };
        let later = table("width:225pt");
        let one_line = table("white-space:nowrap");
        assert!(
            (later.height - one_line.height).abs() < 1.0,
            "address wrapped: {} vs {}",
            later.height,
            one_line.height
        );
    }

    /// Rightmost painted (non-white) column of the first tile, in display px.
    fn ink_right(mail: &RenderedMail) -> f32 {
        let tile = &mail.tiles[0];
        let px_width = tile.px_width as usize;
        let rightmost = tile
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, px)| px[0] < 128)
            .map(|(i, _)| i % px_width)
            .max()
            .expect("text painted");
        (rightmost + 1) as f32 * mail.width / px_width as f32
    }

    fn render_line(body: &str, zoom: f32) -> RenderedMail {
        render(RenderRequest {
            html: format!(
                r#"<body style="margin:0"><p style="margin:0;font-size:40px;white-space:nowrap">MMMMMMMM</p>{body}</body>"#
            ),
            width: 600.0,
            zoom,
            inline_images: HashMap::new(),
            remote: None,
        })
    }

    #[test]
    fn zoom_scales_text_with_its_layout() {
        let actual = ink_right(&render_line("", 1.0));
        let zoomed = ink_right(&render_line("", 0.5));
        assert!(
            (zoomed / (actual * 0.5) - 1.0).abs() < 0.05,
            "text at 50% ends at {zoomed}, at 100% at {actual}"
        );
    }

    #[test]
    fn tall_emails_paint_text_at_its_layout_size() {
        let short = ink_right(&render_line("", 1.0));
        let tall = render_line(r#"<div style="height:7000px"></div>"#, 1.0);
        assert!(tall.height > HIDPI_MAX_HEIGHT);
        let tall = ink_right(&tall);
        assert!(
            (tall / short - 1.0).abs() < 0.05,
            "tall text ends at {tall}, short at {short}"
        );
    }

    #[test]
    fn tiles_cover_the_full_height() {
        let mail = render_html(r#"<div style="height:5000px">tall</div>"#);
        let covered: f32 = mail.tiles.iter().map(|t| t.height).sum();
        assert!((covered - mail.height).abs() < 1.0);
        assert!(mail.tiles.len() > 1);
    }
}
