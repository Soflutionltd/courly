//! HTML email rendering on top of the Blitz engine (Stylo CSS, Taffy layout,
//! Parley text, Vello CPU raster).
//!
//! The renderer is deliberately static: no scripts, no forms, no network
//! unless the caller opts in per message. Output is a set of RGBA tiles plus
//! the hit areas of every link and the position of every character, so the
//! UI layer only has to draw images and can still select text.

mod fonts;
pub mod quote_audit;
mod quote_header;
mod quote_lines;
mod quotes;
mod text;

pub use quote_header::QuoteHeader;
pub use text::{TextCell, TextLine};

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::util::{Color, ToColorColor as _};
use blitz_dom::{
    BaseDocument, DocumentConfig, LocalName, NodeId, QualName, UserAgentStylesheet, local_name, ns,
};
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
use peniko::color::{AlphaColor, Hsl, LinearSrgb, Srgb};
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
/// Marks a picture declared wider than the page: see `fit_wide_images`.
const FIT_ATTRIBUTE: &str = "data-courly-fit";

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
/* Blitz paints no cell border in the collapsing border model, which drops
   the separators of Outlook signatures and every bordered table they write
   (`border-collapse: collapse` inline). The separate model with no spacing
   draws the same lines; two touching bordered cells show both borders. */
table[style*="collapse" i] { border-collapse: separate !important; border-spacing: 0 !important; }
/* A picture declared wider than the page (a screenshot pasted into Outlook)
   is scaled down to the page like other mail clients do, instead of widening
   the whole message. `fit_wide_images` marks them and gives them their
   proportions. */
img[data-courly-fit] { max-width: 100% !important; height: auto !important; }

/* An earlier message quoted in a reply is told apart from the reply above
   it, instead of running on under an indent. The quotes are found and marked
   by `quotes::frame_quotes`, by structure: a class alone is not enough,
   Outlook copies Gmail's onto every paragraph.
   A quote the sender answered inside stays where it is, in a rounded frame:
   an outline only, in a translucent grey that reads on a light and a dark
   page, since a fill would tint the quoted message. */
[data-courly-quote] {
    display: block !important;
    margin: 16px 0 0 0 !important;
    padding: 12px 14px !important;
    border: 1px solid rgba(127, 127, 127, 0.4) !important;
    border-radius: 8px !important;
    background-color: transparent !important;
}
/* The quotes that end the message are cut out of the page and shown by the
   caller as cards of their own, so they bring no frame or spacing. */
[data-courly-quote="card"] {
    margin: 0 !important;
    padding: 0 !important;
    border: none !important;
    border-radius: 0 !important;
}
/* The lines that name the sender of a quoted message, once read: the caller
   shows them as the header of the card. */
[data-courly-quote-header] { display: none !important; }
/* The rule Outlook draws above the header lines of a quoted message set it
   apart from the reply; the card's own edge does that now. */
[data-courly-quote="card"][id*="mail-editor-reference-message-container"] > div:first-child,
[data-courly-quote="card"] > div:first-child[style*="border-top"],
[data-courly-quote="card"] > div:first-child > div:first-child[style*="border-top"] {
    border-top: none !important;
    padding-top: 0 !important;
}
/* Gmail wraps the quoted text in a second, indented block with a bar: the
   rounded block replaces both. */
div[data-courly-quote] > blockquote:not([data-courly-quote]) {
    margin: 0 !important;
    padding: 0 !important;
    border: none !important;
}
div[data-courly-quote] > .gmail_attr { margin-bottom: 8px !important; }
"#;

static EMAIL_UA_SHEET: LazyLock<UserAgentStylesheet> =
    LazyLock::new(|| UserAgentStylesheet::parse(EMAIL_UA_CSS));

/// Fetches a remote resource. Only called for `http`/`https` URLs, and only
/// when the caller allowed remote content for this message.
pub type RemoteFetcher = Arc<dyn Fn(&Url) -> Option<Bytes> + Send + Sync>;

/// How the page around the email is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    /// Page colour behind the email, as sRGB bytes.
    pub background: [u8; 3],
    /// Recolour the email for a dark page, like Outlook: light backgrounds
    /// turn dark and dark text turns light, hues kept. Images are untouched.
    pub dark: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            background: [255, 255, 255],
            dark: false,
        }
    }
}

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
    pub appearance: Appearance,
    /// The HTML is a quoted message on its own, the one a reply is being
    /// written to: the header it opens with is reported as `lead_header`
    /// and left off the page.
    pub quote_only: bool,
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
    /// Every line of text, in document order.
    pub text: Vec<TextLine>,
    /// Where each earlier message quoted at the end of this one starts, from
    /// the top of the email, in rising order. The caller shows what lies
    /// between two of them as a card of its own. No tile spans one.
    pub quote_starts: Vec<f32>,
    /// For each of `quote_starts`, who wrote that message and when, when the
    /// mail said so in a form that is understood. What spelled it out is
    /// then left off the page: the caller shows the header its own way.
    pub quote_headers: Vec<Option<QuoteHeader>>,
    /// With `quote_only`, the header the quoted message opened with.
    pub lead_header: Option<QuoteHeader>,
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
fn layout_viewport(width: f32, zoom: f32, hidpi: f32, scheme: ColorScheme) -> Viewport {
    let scale = hidpi * zoom;
    let mut viewport = Viewport::new(
        device_pixels(width * scale),
        device_pixels(LAYOUT_VIEWPORT_HEIGHT as f32 * scale),
        hidpi,
        scheme,
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

/// Mark the pictures declared wider than a page `width` CSS px wide, so the
/// user-agent sheet scales them down to it. Each keeps the proportions it
/// declares, which also holds its place while the picture is not loaded.
fn fit_wide_images(document: &mut BaseDocument, width: f32) {
    let Ok(images) = document.query_selector_all("img[width]") else {
        return;
    };
    let pixels = |value: &str| value.trim().parse::<f32>().ok().filter(|px| *px > 0.0);
    let wide: Vec<(NodeId, Option<String>)> = images
        .into_iter()
        .filter_map(|id| {
            let node = document.get_node(id)?;
            let declared = node.attr(local_name!("width")).and_then(pixels)?;
            if declared <= width {
                return None;
            }
            let style = node
                .attr(local_name!("height"))
                .and_then(pixels)
                .map(|height| {
                    let written = node.attr(local_name!("style")).unwrap_or_default();
                    format!("{written};aspect-ratio:{declared}/{height}")
                });
            Some((id, style))
        })
        .collect();
    let mark = QualName::new(None, ns!(), LocalName::from(FIT_ATTRIBUTE));
    let style_attribute = QualName::new(None, ns!(), local_name!("style"));
    let mut mutator = document.mutate();
    for (id, style) in wide {
        mutator.set_attribute(id, mark.clone(), "");
        if let Some(style) = style {
            mutator.set_attribute(id, style_attribute.clone(), &style);
        }
    }
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
    let appearance = request.appearance;
    let scheme = if appearance.dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    };
    let [red, green, blue] = appearance.background;
    let page = Color::from_rgb8(red, green, blue);
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
            viewport: Some(layout_viewport(width, zoom, HIDPI_SCALE, scheme)),
            base_url: Some(format!("https://{BASE_HOST}/")),
            net_provider: Some(Arc::clone(&net) as Arc<dyn NetProvider>),
            ua_stylesheets: vec![UserAgentStylesheet::default_sheet(), EMAIL_UA_SHEET.clone()],
            font_ctx: Some(fonts::context_for(&request.html)),
            ..Default::default()
        },
    );
    let (quote_cards, lead_header) = quotes::frame_quotes(&mut document, request.quote_only);
    fit_wide_images(&mut document, width);

    let deadline = Instant::now() + REMOTE_DEADLINE;
    loop {
        document.resolve(0.0);
        if pending.load(Ordering::SeqCst) == 0 || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(REMOTE_POLL);
    }
    document.resolve(0.0);
    if appearance.dark {
        adapt_to_dark(&mut document, page);
        document.resolve(0.0);
    }

    let (mut content_width, mut content_height) = content_size(&document, width, zoom);
    let scale = if content_height > HIDPI_MAX_HEIGHT {
        document.set_viewport(layout_viewport(width, zoom, 1.0, scheme));
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
                page,
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

    let (cut_rows, quote_headers) = quote_rows(&document, quote_cards, zoom * scale, px_height);
    let tiles = split_into_tiles(&buffer, px_width, px_height, scale, &cut_rows);
    let links = collect_links(&document, zoom);
    let text = text::collect_text(&document, zoom);

    RenderedMail {
        width: content_width,
        height: content_height,
        tiles,
        links,
        text,
        quote_starts: cut_rows.iter().map(|&row| row as f32 / scale).collect(),
        quote_headers,
        lead_header,
        blocked_remote: net.blocked.load(Ordering::Relaxed),
    }
}

/// HSL lightness (0 to 100) dark text is lifted to: black lands on the
/// first value, mid grey on the second.
const DARK_TEXT_LIGHTNESS: (f32, f32) = (90.0, 62.0);
/// Lightest a recoloured light background may end up.
const DARK_BACKGROUND_CEILING: f32 = 40.0;
/// Saturation kept on recoloured backgrounds, so pastel panels do not turn
/// into loud dark colours.
const DARK_BACKGROUND_SATURATION: f32 = 0.6;
/// Channel spread under which a light background counts as plain white and
/// takes the page colour exactly.
const NEAR_WHITE_SPREAD: f32 = 0.04;
const NEAR_WHITE_LIGHTNESS: f32 = 95.0;

fn hsl_components(color: Color) -> [f32; 4] {
    let [hue, saturation, lightness, alpha] = color.convert::<Hsl>().components;
    let hue = if hue.is_finite() { hue } else { 0.0 };
    [hue, saturation, lightness, alpha]
}

fn from_hsl(components: [f32; 4]) -> Color {
    AlphaColor::<Hsl>::new(components).convert::<Srgb>()
}

/// WCAG relative luminance text needs on a dark page: about 5:1 contrast
/// against a near-black background.
const MIN_TEXT_LUMINANCE: f32 = 0.3;
const TEXT_LIGHTNESS_STEP: f32 = 2.0;

fn relative_luminance(color: Color) -> f32 {
    let [red, green, blue, _] = color.convert::<LinearSrgb>().components;
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/// Text made light enough for a dark page, hue kept. `None` when it
/// already reads. Saturated blues and reds are dim at any HSL lightness, so
/// lightness keeps rising until the luminance is high enough.
fn dark_text(color: Color) -> Option<Color> {
    let [hue, saturation, lightness, alpha] = hsl_components(color);
    if alpha == 0.0 || (lightness >= 50.0 && relative_luminance(color) >= MIN_TEXT_LUMINANCE) {
        return None;
    }
    let (black, mid) = DARK_TEXT_LIGHTNESS;
    let mut lightness = if lightness < 50.0 {
        black - lightness / 50.0 * (black - mid)
    } else {
        lightness
    };
    let mut lifted = from_hsl([hue, saturation, lightness, alpha]);
    while relative_luminance(lifted) < MIN_TEXT_LUMINANCE && lightness < 100.0 {
        lightness = (lightness + TEXT_LIGHTNESS_STEP).min(100.0);
        lifted = from_hsl([hue, saturation, lightness, alpha]);
    }
    Some(lifted)
}

/// Light background made dark for a dark page. Plain white becomes the
/// page colour itself, so the email merges with the sheet around it.
/// `None` for transparent or already dark backgrounds.
fn dark_background(color: Color, page: Color) -> Option<Color> {
    let [hue, saturation, lightness, alpha] = hsl_components(color);
    if alpha == 0.0 || lightness < 50.0 {
        return None;
    }
    let [red, green, blue, _] = color.components;
    let spread = red.max(green).max(blue) - red.min(green).min(blue);
    if lightness >= NEAR_WHITE_LIGHTNESS && spread < NEAR_WHITE_SPREAD {
        return Some(page.with_alpha(alpha));
    }
    let page_lightness = hsl_components(page)[2];
    let lightness =
        page_lightness + (100.0 - lightness) / 50.0 * (DARK_BACKGROUND_CEILING - page_lightness);
    Some(from_hsl([
        hue,
        saturation * DARK_BACKGROUND_SATURATION,
        lightness,
        alpha,
    ]))
}

fn css_color(color: Color) -> String {
    let rgba = color.to_rgba8();
    format!(
        "rgba({}, {}, {}, {:.3})",
        rgba.r,
        rgba.g,
        rgba.b,
        f32::from(rgba.a) / 255.0
    )
}

/// Recolour every element for a dark page. The new colours are appended to
/// each element's inline style as `!important`, which outranks the email's
/// own rules, including its `!important` ones.
fn adapt_to_dark(document: &mut BaseDocument, page: Color) {
    let restyles: Vec<(NodeId, String)> = document
        .tree()
        .iter()
        .filter(|(_, node)| node.is_element())
        .filter_map(|(id, node)| {
            let styles = node.primary_styles()?;
            let current = styles.clone_color();
            let text = dark_text(current.as_color_color());
            let background = dark_background(
                styles
                    .get_background()
                    .background_color
                    .resolve_to_absolute(&current)
                    .as_color_color(),
                page,
            );
            if text.is_none() && background.is_none() {
                return None;
            }
            let mut style = node
                .attr(local_name!("style"))
                .unwrap_or_default()
                .trim_end()
                .trim_end_matches(';')
                .to_string();
            if let Some(text) = text {
                style.push_str(&format!(";color:{} !important", css_color(text)));
            }
            if let Some(background) = background {
                style.push_str(&format!(
                    ";background-color:{} !important",
                    css_color(background)
                ));
            }
            Some((id, style))
        })
        .collect();
    let mut mutator = document.mutate();
    for (id, style) in restyles {
        mutator.set_attribute(id, QualName::new(None, ns!(), local_name!("style")), &style);
    }
}

/// The pixel row each quote card starts on, in rising order. A quote that
/// does not start below the one before it (a sender's layout put them side
/// by side) is not cut out: it stays in the card above.
///
/// Each row comes with the header read for its quote, if any.
fn quote_rows(
    doc: &BaseDocument,
    cards: Vec<quotes::QuoteCard>,
    px_per_css: f32,
    px_height: u32,
) -> (Vec<u32>, Vec<Option<QuoteHeader>>) {
    let mut rows: Vec<u32> = Vec::new();
    let mut headers = Vec::new();
    for card in cards {
        let Some(top) = doc
            .get_client_bounding_rect(card.node)
            .and_then(|rect| rect.y.to_f32())
        else {
            continue;
        };
        let Some(row) = (top * px_per_css).round().to_u32() else {
            continue;
        };
        if row > rows.last().copied().unwrap_or(0) && row < px_height {
            rows.push(row);
            headers.push(card.header);
        }
    }
    (rows, headers)
}

/// Cut the picture into tiles no taller than the GPU takes, and never
/// across one of `cut_rows`.
fn split_into_tiles(
    buffer: &[u8],
    px_width: u32,
    px_height: u32,
    scale: f32,
    cut_rows: &[u32],
) -> Vec<Tile> {
    let row_bytes = px_width as usize * 4;
    let mut tiles = Vec::new();
    let mut top = 0;
    while top < px_height {
        let next_cut = cut_rows
            .iter()
            .copied()
            .find(|&row| row > top)
            .unwrap_or(px_height);
        let rows = TILE_HEIGHT_PX.min(next_cut - top);
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
            appearance: Appearance::default(),
            quote_only: false,
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
            appearance: Appearance::default(),
            quote_only: false,
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

    /// A screenshot pasted into Outlook is written at its full size, in a
    /// message and in every reply quoting it.
    #[test]
    fn a_picture_wider_than_the_page_is_scaled_down_to_it() {
        let mail = render_html(
            r#"<body style="margin:0"><p>Before</p>
               <p><img width="1647" height="398" style="width:17.1562in;height:4.1458in"
                       src="cid:missing"></p></body>"#,
        );
        assert!(mail.width <= 601.0, "width was {}", mail.width);
        // 600 px wide at 1647:398 is 145 px tall, under the line before it.
        assert!(
            (150.0..260.0).contains(&mail.height),
            "height was {}",
            mail.height
        );
    }

    #[test]
    fn a_picture_that_fits_the_page_keeps_its_size() {
        let mail = render_html(
            r#"<body style="margin:0"><img width="300" height="400" src="cid:missing"></body>"#,
        );
        assert!(mail.height >= 400.0, "height was {}", mail.height);
    }

    const DARK_PAGE: [u8; 3] = [31, 31, 31];

    fn render_dark(html: &str) -> RenderedMail {
        render(RenderRequest {
            html: html.to_string(),
            width: 200.0,
            zoom: 1.0,
            inline_images: HashMap::new(),
            remote: None,
            appearance: Appearance {
                background: DARK_PAGE,
                dark: true,
            },
            quote_only: false,
        })
    }

    /// RGB of the device pixel at CSS position (`x`, `y`) of a short email,
    /// painted at `HIDPI_SCALE`.
    fn pixel(mail: &RenderedMail, x: usize, y: usize) -> [u8; 3] {
        let tile = mail
            .tiles
            .iter()
            .rev()
            .find(|tile| tile.y.to_usize().expect("tile top") <= y)
            .expect("a tile at this height");
        let scale = HIDPI_SCALE.to_usize().expect("integral scale");
        let row = tile.px_width.to_usize().expect("tile width");
        let y = y - tile.y.to_usize().expect("tile top");
        let offset = (y * scale * row + x * scale) * 4;
        [
            tile.rgba[offset],
            tile.rgba[offset + 1],
            tile.rgba[offset + 2],
        ]
    }

    fn luma([red, green, blue]: [u8; 3]) -> f32 {
        0.299 * f32::from(red) + 0.587 * f32::from(green) + 0.114 * f32::from(blue)
    }

    /// At a reading zoom below 1 a 1px border is thinner than a layout
    /// pixel, and rounding the layout used to leave it 0 or 1 wide by where
    /// its box fell: of a column of identical bordered buttons (a Zendesk
    /// survey's), some kept their border and the others lost all or part of
    /// it. Each box here starts at a different fraction of a pixel; down
    /// the middle of the boxes, every one must show its top and its bottom
    /// edge.
    #[test]
    fn thin_borders_survive_a_fractional_zoom() {
        let boxes: String = [3, 7, 11, 13, 17, 5]
            .iter()
            .map(|gap| {
                format!(
                    r#"<div style="border:1px solid #000;width:100px;height:20px;margin-bottom:{gap}px"></div>"#
                )
            })
            .collect();
        let mail = render(RenderRequest {
            html: format!(r#"<body style="margin:0">{boxes}</body>"#),
            width: 300.0,
            zoom: 0.9,
            inline_images: HashMap::new(),
            remote: None,
            appearance: Appearance::default(),
            quote_only: false,
        });
        // Every device row: a border is under two device pixels thick here,
        // so one row in two could fall between its lines.
        let column = 90;
        let mut edges = 0;
        let mut inside = false;
        for tile in &mail.tiles {
            let row = tile.px_width.to_usize().expect("tile width");
            for y in 0..tile.px_height.to_usize().expect("tile height") {
                let offset = (y * row + column) * 4;
                let dark = luma([
                    tile.rgba[offset],
                    tile.rgba[offset + 1],
                    tile.rgba[offset + 2],
                ]) < 128.0;
                edges += usize::from(dark && !inside);
                inside = dark;
            }
        }
        assert_eq!(edges, 12, "{edges} of the 12 top and bottom edges painted");
    }

    /// Pixels darker than plain text's antialiasing threshold in the CSS
    /// square from the origin to (`size`, `size`).
    fn inked_pixels(mail: &RenderedMail, size: usize) -> usize {
        let height = mail.height.to_usize().expect("height");
        (0..size.min(height))
            .flat_map(|y| (0..size).map(move |x| (x, y)))
            .filter(|&(x, y)| luma(pixel(mail, x, y)) < 200.0)
            .count()
    }

    /// Apple Color Emoji keeps each emoji as a PNG bitmap. The painter used
    /// to skip PNG glyphs, so emoji laid out (the line took their height)
    /// but painted nothing, in text and inside bordered survey buttons
    /// alike.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_emoji_is_painted() {
        let mail = render_html(
            r#"<body style="margin:0"><p style="font-size:40px;margin:0">😍</p></body>"#,
        );
        assert!(
            inked_pixels(&mail, 60) > 100,
            "the emoji painted {} pixels",
            inked_pixels(&mail, 60)
        );
    }

    /// Darkest pixel of row `y` between CSS `x_from` and `x_to`.
    fn darkest_in_row(mail: &RenderedMail, y: usize, x_from: usize, x_to: usize) -> f32 {
        (x_from..x_to)
            .map(|x| luma(pixel(mail, x, y)))
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn outlook_signature_separator_is_painted_in_a_collapsing_table() {
        // The separator of an Outlook signature: a left border on the text
        // cell, written as longhands, in a `border-collapse: collapse` table.
        let cell = "border-width: medium medium medium 1pt; border-style: none none none solid; \
                    border-color: currentcolor currentcolor currentcolor rgb(189, 189, 189); \
                    padding: 0 0 0 18px; vertical-align: top;";
        let html = format!(
            r#"<body style="margin:0;background:#fff"><table border="0" cellspacing="0" cellpadding="0" style="box-sizing: border-box; border-collapse: collapse; border-spacing: 0px;"><tr>
            <td style="padding:0; width:100px; height:60px"></td>
            <td style="{cell}"><div style="height:60px;width:50px"></div></td></tr></table></body>"#
        );
        let mail = render_html(&html);
        let darkest = darkest_in_row(&mail, 30, 95, 110);
        assert!(
            darkest < 200.0,
            "no separator painted, darkest luma {darkest}"
        );
    }

    #[test]
    fn dark_mode_turns_white_panels_into_the_page_colour() {
        let mail = render_dark(
            r#"<body style="margin:0;background:#ffffff">
            <div style="background:#fff !important;height:40px"></div></body>"#,
        );
        assert_eq!(pixel(&mail, 100, 20), DARK_PAGE);
    }

    #[test]
    fn dark_mode_darkens_tinted_panels_and_keeps_dark_ones() {
        let mail = render_dark(
            r#"<body style="margin:0">
            <div style="background:#e8f4ff;height:40px"></div>
            <div style="background:#0b3d91;height:40px"></div></body>"#,
        );
        let tinted = pixel(&mail, 100, 20);
        assert!(luma(tinted) < 90.0, "tinted panel stayed light: {tinted:?}");
        assert!(
            tinted[2] > tinted[0],
            "tinted panel lost its blue hue: {tinted:?}"
        );
        assert_eq!(pixel(&mail, 100, 60), [11, 61, 145]);
    }

    #[test]
    fn dark_mode_lifts_dark_text_and_keeps_light_text() {
        let lifted = dark_text(Color::from_rgb8(31, 31, 31))
            .expect("dark text is lifted")
            .to_rgba8();
        assert!(luma([lifted.r, lifted.g, lifted.b]) > 200.0);
        assert_eq!(dark_text(Color::WHITE), None);
        let link = dark_text(Color::from_rgb8(0, 0, 238)).expect("dark links are lifted");
        assert!(relative_luminance(link) >= MIN_TEXT_LUMINANCE);
        let link = link.to_rgba8();
        assert!(
            link.b > link.r && link.b > 200,
            "link lost its hue: {link:?}"
        );
        let brand = dark_text(Color::from_rgb8(26, 115, 232)).expect("dim brand blue is lifted");
        assert!(relative_luminance(brand) >= MIN_TEXT_LUMINANCE);
    }

    #[test]
    fn light_mode_paints_the_email_unchanged() {
        let mail = render_html(r#"<body style="margin:0"><div style="height:40px"></div></body>"#);
        assert_eq!(pixel(&mail, 100, 20), [255, 255, 255]);
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
    fn text_is_reported_line_by_line_in_document_order() {
        let mail = render_html("<p>Your code is 482913</p><p>Second paragraph</p>");
        let texts: Vec<&str> = mail.text.iter().map(|line| line.text.trim()).collect();
        assert_eq!(texts, ["Your code is 482913", "Second paragraph"]);
        assert_ne!(mail.text[0].block, mail.text[1].block);
        assert!(mail.text[0].y < mail.text[1].y);

        let line = &mail.text[0];
        let spelled: String = line
            .cells
            .iter()
            .map(|cell| &line.text[cell.start..cell.end])
            .collect();
        assert_eq!(spelled, line.text);
        assert!(
            line.cells.windows(2).all(|pair| pair[0].x < pair[1].x),
            "characters must advance left to right"
        );
    }

    #[test]
    fn text_positions_match_where_the_text_is_painted() {
        let mail = render_html(
            r#"<body style="margin:0"><table><tr><td style="padding:40px 0 0 120px">
               <a href="https://example.com/x">Open</a></td></tr></table></body>"#,
        );
        let link = &mail.links[0];
        let line = &mail.text[0];
        let first = &line.cells[0];
        let last = &line.cells[line.cells.len() - 1];
        assert!((first.x - link.x).abs() < 1.0, "{} vs {}", first.x, link.x);
        let right = last.x + last.width;
        assert!(
            (right - (link.x + link.width)).abs() < 1.0,
            "{right} vs {}",
            link.x + link.width
        );
        assert!((line.y - link.y).abs() < 1.0, "{} vs {}", line.y, link.y);
    }

    #[test]
    fn wrapped_lines_stay_in_one_block() {
        let words = "word ".repeat(80);
        let mail = render_html(&format!("<p>{words}</p>"));
        assert!(mail.text.len() > 1, "the paragraph must wrap");
        assert!(
            mail.text
                .iter()
                .all(|line| line.block == mail.text[0].block)
        );
    }

    #[test]
    fn text_positions_follow_the_zoom() {
        let html = r#"<p style="margin:0 0 0 100px">Zoomed</p>"#;
        let left_edge = |mail: &RenderedMail| {
            mail.text
                .iter()
                .find(|line| line.text.trim() == "Zoomed")
                .map(|line| line.cells[0].x)
        };
        let near = left_edge(&render_line(html, 1.0)).expect("line at 1x");
        let far = left_edge(&render_line(html, 2.0)).expect("line at 2x");
        assert!((far - near * 2.0).abs() < 1.0, "{near} then {far}");
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

    #[test]
    fn an_import_with_an_address_that_does_not_parse_is_skipped() {
        for import in [
            r#"@import url("http://[::1");"#,
            r#"@import "http://exa mple.com:port/a.css";"#,
            r#"@import url("https://:0/a.css");"#,
        ] {
            let html = format!(
                r#"<html><head><style>{import} p {{ margin: 0 }}</style></head>
                <body><p style="height:300px">Receipt</p></body></html>"#
            );
            let mail = render_html(&html);
            assert!(mail.height >= 300.0, "{import}: height was {}", mail.height);
        }
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

    /// The line of text reading `wanted`.
    fn text_line<'m>(mail: &'m RenderedMail, wanted: &str) -> &'m TextLine {
        mail.text
            .iter()
            .find(|line| line.text.trim() == wanted)
            .unwrap_or_else(|| panic!("no line reads {wanted:?}"))
    }

    /// The background behind a line of text, sampled just right of it.
    fn background_beside(mail: &RenderedMail, line: &TextLine) -> [u8; 3] {
        let last = &line.cells[line.cells.len() - 1];
        let x = (last.x + last.width + 4.0)
            .to_usize()
            .expect("x on the page");
        let y = (line.y + line.height / 2.0)
            .to_usize()
            .expect("y on the page");
        pixel(mail, x, y)
    }

    /// Check the page is cut into one card per quoted message: `firsts`
    /// names the first line of each, and `above` the last line before the
    /// first of them.
    fn assert_quote_cards(mail: &RenderedMail, above: &str, firsts: &[&str]) {
        assert_eq!(
            mail.quote_starts.len(),
            firsts.len(),
            "one card per quoted message, cuts at {:?}",
            mail.quote_starts
        );
        let mut previous = text_line(mail, above);
        for (&start, &first) in mail.quote_starts.iter().zip(firsts) {
            let line = text_line(mail, first);
            assert!(
                start >= previous.y + previous.height - 1.0 && start <= line.y + 1.0,
                "the card of {first:?} starts at {start}, outside {}..{}",
                previous.y + previous.height,
                line.y
            );
            // An outline would run along the first row of the card.
            let top = start.to_usize().expect("y on the page");
            assert!(
                darkest_in_row(mail, top, 20, 60) > 250.0,
                "a card brings no outline of its own"
            );
            previous = line;
        }
        for tile in &mail.tiles {
            assert!(
                !mail
                    .quote_starts
                    .iter()
                    .any(|&start| tile.y < start - 0.1 && tile.y + tile.height > start + 0.1),
                "a tile spans a cut"
            );
        }
    }

    #[test]
    fn a_gmail_quote_sits_in_its_own_block_apart_from_the_reply() {
        let mail = render_html(
            r#"<body style="margin:0"><div dir="ltr">The reply</div><br>
               <div class="gmail_quote gmail_quote_container">
               <div dir="ltr" class="gmail_attr">On Monday, Ann wrote:<br></div>
               <blockquote class="gmail_quote"
                 style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex">
               <div dir="ltr">The earlier message</div></blockquote></div></body>"#,
        );
        let reply = text_line(&mail, "The reply");
        let quoted = text_line(&mail, "The earlier message");
        assert_eq!(background_beside(&mail, reply), [255, 255, 255]);
        assert_eq!(
            background_beside(&mail, quoted),
            [255, 255, 255],
            "nothing is painted behind the quoted text"
        );
        assert_quote_cards(&mail, "The reply", &["On Monday, Ann wrote:"]);
        // The card's edge, not Gmail's indent and bar, sets the text in.
        let attribution = text_line(&mail, "On Monday, Ann wrote:");
        assert!(
            (quoted.cells[0].x - attribution.cells[0].x).abs() < 1.0,
            "quoted text at {} and its attribution at {} must share an edge",
            quoted.cells[0].x,
            attribution.cells[0].x
        );
    }

    #[test]
    fn an_apple_mail_quote_sits_in_its_own_block() {
        let mail = render_html(
            r#"<body style="margin:0"><div>The reply</div>
               <div>On Monday, Ann wrote:</div>
               <blockquote type="cite"><div>The earlier message</div></blockquote></body>"#,
        );
        let quoted = text_line(&mail, "The earlier message");
        assert_eq!(background_beside(&mail, quoted), [255, 255, 255]);
        assert_quote_cards(&mail, "On Monday, Ann wrote:", &["The earlier message"]);
    }

    #[test]
    fn a_new_outlook_quote_is_a_card_with_its_header_lines() {
        let mail = render_html(
            r#"<body style="margin:0"><div>The reply</div>
               <div id="m_-42mail-editor-reference-message-container">
               <div style="padding:3pt 0in 0in;border-width:1pt medium medium;border-style:solid none none">
               <div><b>De :</b> Ann<br><b>Objet :</b> Hello<br></div></div>
               <div id="m_-42mail-editor-reference-message-body"><div>The earlier message</div>
               <div class="gmail_attr">On Monday, Bob wrote:</div>
               <blockquote><div class="gmail_quote">The first message</div></blockquote>
               </div></div></body>"#,
        );
        // The header lines are read and left off the page: the card starts
        // with the message.
        assert_quote_cards(
            &mail,
            "The reply",
            &["The earlier message", "On Monday, Bob wrote:"],
        );
        assert!(mail.text.iter().all(|line| !line.text.contains("Objet")));
        let header = mail.quote_headers[0].as_ref().expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann"));
        assert_eq!(
            mail.quote_headers[1], None,
            "a line that names no address is left as written"
        );
    }

    #[test]
    fn a_quoted_message_on_its_own_reports_the_header_it_opens_with() {
        // What a reply written here carries under the editor.
        let html = r#"<hr style="display:inline-block;width:98%" tabindex="-1">
            <div id="divRplyFwdMsg" dir="ltr"><font face="Calibri, sans-serif">
            <b>De :</b> Ann Example &lt;ann@example.com&gt;<br>
            <b>Envoyé :</b> lundi 2 novembre 2026 11:31<br>
            <b>À :</b> Bob Example &lt;bob@example.com&gt;<br>
            <b>Objet :</b> Hello</font><div>&nbsp;</div></div>
            <div><p>The message answered</p></div>"#;
        let request = |quote_only| RenderRequest {
            html: html.to_string(),
            width: 600.0,
            zoom: 1.0,
            inline_images: HashMap::new(),
            remote: None,
            appearance: Appearance::default(),
            quote_only,
        };
        let quote = render(request(true));
        let header = quote.lead_header.as_ref().expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(header.date.as_deref(), Some("lundi 2 novembre 2026 11:31"));
        assert_eq!(header.to.as_deref(), Some("Bob Example <bob@example.com>"));
        assert!(quote.text.iter().all(|line| !line.text.contains("Objet")));
        assert!(quote.quote_starts.is_empty(), "it is one card");

        // Read as a message, the same lines stay on the page.
        let message = render(request(false));
        assert_eq!(message.lead_header, None);
        assert!(message.text.iter().any(|line| line.text.contains("Objet")));
    }

    #[test]
    fn a_gmail_attribution_becomes_the_header_of_its_card() {
        let mail = render_html(
            r#"<body style="margin:0"><div dir="ltr">The reply</div><br>
               <div class="gmail_quote gmail_quote_container">
               <div dir="ltr" class="gmail_attr">Le lun. 26 oct. 2026 à 22:48, Ann Example
                 &lt;<a href="mailto:ann@example.com">ann@example.com</a>&gt; a écrit :<br></div>
               <blockquote class="gmail_quote"><div dir="ltr">The earlier message</div>
               </blockquote></div></body>"#,
        );
        assert_quote_cards(&mail, "The reply", &["The earlier message"]);
        assert!(mail.text.iter().all(|line| !line.text.contains("a écrit")));
        assert_eq!(
            mail.quote_headers,
            [Some(QuoteHeader {
                name: Some("Ann Example".to_owned()),
                address: Some("ann@example.com".to_owned()),
                date: Some("lun. 26 oct. 2026 à 22:48".to_owned()),
                to: None,
                cc: None,
            })]
        );
    }

    #[test]
    fn a_desktop_outlook_quote_runs_from_its_header_to_the_end() {
        let mail = render_html(
            r#"<body style="margin:0"><div class="WordSection1"><p>The reply</p>
               <div><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0cm 0cm 0cm">
               <p><b>From:</b> Ann<br><b>Subject:</b> Hello</p></div></div>
               <p>The earlier message</p><p>Its second paragraph</p></div></body>"#,
        );
        assert_quote_cards(&mail, "The reply", &["The earlier message"]);
        let header = mail.quote_headers[0].as_ref().expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann"));
    }

    #[test]
    fn header_lines_start_a_quote_whatever_their_markup() {
        // The same header block after three mail clients rewrote it: the
        // rule is spelled another way, the classes are prefixed, and the
        // innermost message has no markup left but its paragraphs.
        let mail = render_html(
            r#"<body style="margin:0"><div><p class="MsoNormal">The reply</p>
               <div style="padding:3pt 0cm 0cm;border-width:1pt medium medium;border-style:solid none none">
               <p class="x_MsoNormal"><b>De&nbsp;:</b> Ann Example &lt;ann@example.com&gt;<br>
               <b>Envoyé&nbsp;:</b> lundi 2 novembre 2026 11:31<br><b>À&nbsp;:</b> Bob<br>
               <b>Objet&nbsp;:</b> Hello</p></div>
               <p class="x_MsoNormal">The second message</p>
               <p class="x_x_MsoNormal"><b>From:</b> Cy Example &lt;cy@example.com&gt;</p>
               <p class="x_x_MsoNormal"><b>Sent:</b> Sunday</p>
               <p class="x_x_MsoNormal"><b>To:</b> Ann</p>
               <p class="x_x_MsoNormal">The first message</p></div></body>"#,
        );
        assert_quote_cards(
            &mail,
            "The reply",
            &["The second message", "The first message"],
        );
        let names: Vec<_> = mail
            .quote_headers
            .iter()
            .map(|header| header.as_ref().and_then(|header| header.name.as_deref()))
            .collect();
        assert_eq!(names, [Some("Ann Example"), Some("Cy Example")]);
        assert_eq!(
            mail.quote_headers[0]
                .as_ref()
                .and_then(|h| h.date.as_deref()),
            Some("lundi 2 novembre 2026 11:31")
        );
        assert!(mail.text.iter().all(|line| !line.text.contains("Objet")));
        assert!(mail.text.iter().all(|line| !line.text.contains("Sent:")));
    }

    #[test]
    fn a_plain_text_forward_is_cut_at_its_rule() {
        // No element at all: text and line breaks straight in the body.
        let mail = render_html(
            "<body style=\"margin:0\">Here it is.<br><br>-----Original Message-----<br>\
             From: Ann Example &lt;ann@example.com&gt;<br>Sent: Monday<br>To: Bob<br>\
             Subject: Hello<br><br>The earlier message</body>",
        );
        assert_quote_cards(&mail, "Here it is.", &["The earlier message"]);
        let header = mail.quote_headers[0].as_ref().expect("a header");
        assert_eq!(header.address.as_deref(), Some("ann@example.com"));
        assert_eq!(header.date.as_deref(), Some("Monday"));
        assert!(mail.text.iter().all(|line| !line.text.contains("Original")));
        assert!(mail.text.iter().all(|line| !line.text.contains("Subject")));
    }

    #[test]
    fn messages_written_as_lines_of_one_span_are_cut_at_each_header() {
        // A text mail turned to HTML: every message of the conversation in
        // one span, its lines parted by line breaks alone.
        let mail = render_html(
            "<body style=\"margin:0\"><div><p>The reply</p>\
             <p><span>The second message<br>Regards<br><br>\
             From: Ann Example &lt;ann@example.com&gt;<br>Sent: Monday<br>To: Bob<br>\
             Subject: Hello<br><br>The first message<br><br>\
             From: Cy Example &lt;cy@example.com&gt;<br>Sent: Sunday<br>To: Ann<br>\
             Subject: Hello<br><br>The very first message</span></p></div></body>",
        );
        assert_quote_cards(
            &mail,
            "The reply",
            &["The first message", "The very first message"],
        );
        let names: Vec<_> = mail
            .quote_headers
            .iter()
            .map(|header| header.as_ref().and_then(|header| header.name.as_deref()))
            .collect();
        assert_eq!(names, [Some("Ann Example"), Some("Cy Example")]);
        // What the span shows before the first header stays in the message.
        assert!(
            mail.text
                .iter()
                .any(|line| line.text.contains("The second message"))
        );
    }

    #[test]
    fn an_attribution_line_above_a_plain_blockquote_makes_it_a_quote() {
        // Outlook strips `type=cite` from the blockquote of a message it
        // quotes again: the line above is what is left to tell.
        let mail = render_html(
            r#"<body style="margin:0"><div>The reply</div>
               <p class="MsoNormal">Le 12 nov. 2026 à 10:02, Ann Example a écrit&nbsp;:</p>
               <blockquote style="margin-left:4pt"><p>The earlier message</p></blockquote></body>"#,
        );
        assert_quote_cards(&mail, "The reply", &["The earlier message"]);
        let header = mail.quote_headers[0].as_ref().expect("a header");
        assert_eq!(header.name.as_deref(), Some("Ann Example"));
        assert_eq!(header.date.as_deref(), Some("12 nov. 2026 à 10:02"));
        assert!(mail.text.iter().all(|line| !line.text.contains("a écrit")));
    }

    #[test]
    fn an_attribution_line_with_loose_paragraphs_under_it_starts_a_quote() {
        let mail = render_html(
            r#"<body style="margin:0"><div><p>The reply</p>
               <p>On Mon, Nov 2, 2026 at 11:31 AM Ann Example &lt;ann@example.com&gt; wrote:</p>
               <p>The earlier message</p><p>Its second paragraph</p></div></body>"#,
        );
        assert_quote_cards(&mail, "The reply", &["The earlier message"]);
        let header = mail.quote_headers[0].as_ref().expect("a header");
        assert_eq!(header.address.as_deref(), Some("ann@example.com"));
    }

    #[test]
    fn lines_that_only_look_like_a_header_start_no_quote() {
        let mail = render_html(
            r#"<body style="margin:0"><p>Travel plan</p>
               <p>From: Paris<br>Note: bring the papers</p>
               <p>As Ann wrote: nothing changes.</p><p>See you there</p></body>"#,
        );
        assert!(mail.quote_starts.is_empty());
    }

    #[test]
    fn a_ruled_block_that_is_no_message_header_starts_no_card() {
        let mail = render_html(
            r#"<body style="margin:0"><p>The message</p>
               <div style="border-top:1px solid #ccc"><p>Sent from the office</p></div>
               <p>A last line</p></body>"#,
        );
        assert!(mail.quote_starts.is_empty());
    }

    #[test]
    fn a_message_forwarded_without_a_word_stays_one_card() {
        let mail = render_html(
            r#"<body style="margin:0"><div class="gmail_quote">
               <div class="gmail_attr">---------- Forwarded message ---------</div>
               <blockquote class="gmail_quote"><div>The forwarded message</div></blockquote>
               </div></body>"#,
        );
        assert!(mail.quote_starts.is_empty());
    }

    const NESTED_QUOTES: &str = r#"<div class="gmail_quote">
        <div class="gmail_attr">On Tuesday, Bob wrote:</div>
        <blockquote class="gmail_quote"><div>The middle message</div>
        <div class="gmail_quote"><div class="gmail_attr">On Monday, Ann wrote:</div>
        <blockquote class="gmail_quote"><div>The first message</div></blockquote></div>
        AFTER</blockquote></div>"#;

    #[test]
    fn a_quote_closing_another_is_framed_under_it_not_inside_it() {
        let html = format!(
            r#"<body style="margin:0"><div>The reply</div>{}</body>"#,
            NESTED_QUOTES.replace("AFTER", "")
        );
        let mail = render_html(&html);
        let middle = text_line(&mail, "On Tuesday, Bob wrote:");
        let first = text_line(&mail, "On Monday, Ann wrote:");
        assert!(
            (middle.cells[0].x - first.cells[0].x).abs() < 1.0,
            "both messages must start at the same edge, got {} and {}",
            middle.cells[0].x,
            first.cells[0].x
        );
        assert_quote_cards(
            &mail,
            "The reply",
            &["On Tuesday, Bob wrote:", "On Monday, Ann wrote:"],
        );
    }

    #[test]
    fn a_quote_written_as_several_blockquotes_gets_one_frame() {
        let mail = render_html(
            r#"<body style="margin:0"><div>The reply</div>
               <blockquote type="cite"><div>First piece</div></blockquote>
               <blockquote type="cite"><div>Second piece</div></blockquote>
               <blockquote type="cite"><div><br></div></blockquote></body>"#,
        );
        let first = text_line(&mail, "First piece");
        let second = text_line(&mail, "Second piece");
        assert_quote_cards(&mail, "The reply", &["First piece"]);
        let x = (first.cells[0].x + 40.0).to_usize().expect("x on the page");
        let from = (first.y + first.height).to_usize().expect("y on the page");
        let to = second.y.to_usize().expect("y on the page");
        for y in from..to {
            assert!(
                luma(pixel(&mail, x, y)) > 250.0,
                "an outline separates the pieces at y = {y}"
            );
        }
        // Nothing is drawn under the last piece: the empty one has no frame.
        let below = (second.y + second.height + 30.0)
            .to_usize()
            .expect("y on the page");
        let bottom = mail.height.to_usize().expect("page height");
        for y in below..bottom {
            assert!(
                luma(pixel(&mail, x, y)) > 250.0,
                "an empty frame is drawn at y = {y}"
            );
        }
    }

    /// Outlook re-serialises a Gmail message with `gmail_quote` on every
    /// paragraph and none on the `blockquote`.
    #[test]
    fn paragraphs_carrying_the_gmail_class_are_not_framed_one_by_one() {
        let mail = render_html(
            r#"<body style="margin:0"><div><div>The reply</div>
               <div class="gmail_attr">On Monday, Ann wrote:</div>
               <blockquote style="margin:0 0 0 0.8ex;padding-left:1ex;border-left:1px solid #ccc">
               <div class="gmail_quote">First paragraph</div>
               <div class="gmail_quote">Second paragraph</div>
               </blockquote></div></body>"#,
        );
        let first = text_line(&mail, "First paragraph");
        let second = text_line(&mail, "Second paragraph");
        assert_quote_cards(&mail, "The reply", &["On Monday, Ann wrote:"]);
        // One card for both: no outline runs between the paragraphs.
        let x = (first.cells[0].x + 40.0).to_usize().expect("x on the page");
        let from = (first.y + first.height).to_usize().expect("y on the page");
        let to = second.y.to_usize().expect("y on the page");
        for y in from..to {
            assert!(
                luma(pixel(&mail, x, y)) > 250.0,
                "an outline separates the paragraphs at y = {y}"
            );
        }
    }

    #[test]
    fn a_quote_answered_between_the_lines_stays_where_it_was_written() {
        let html = format!(
            r#"<body style="margin:0"><div>The reply</div>{}</body>"#,
            NESTED_QUOTES.replace("AFTER", "<div>An answer in between</div>")
        );
        let mail = render_html(&html);
        let middle = text_line(&mail, "On Tuesday, Bob wrote:");
        let first = text_line(&mail, "On Monday, Ann wrote:");
        assert!(
            first.cells[0].x > middle.cells[0].x + 1.0,
            "the inner quote must stay inside the one that quoted it"
        );
        let answer = text_line(&mail, "An answer in between");
        assert!(answer.y > first.y, "the order of the text must not change");
        assert_eq!(
            mail.quote_starts.len(),
            1,
            "only the outer quote is a card: the inner one was answered"
        );
        assert!(
            pixel_column_has_outline(&mail, first),
            "the inner quote keeps an outline inside the card"
        );
    }

    /// Whether an outline is drawn left of a line, short of the page's edge.
    fn pixel_column_has_outline(mail: &RenderedMail, line: &TextLine) -> bool {
        let y = (line.y + line.height / 2.0)
            .to_usize()
            .expect("y on the page");
        let until = line.cells[0].x.to_usize().expect("x on the page");
        darkest_in_row(mail, y, 0, until) < 250.0
    }

    #[test]
    fn a_plain_blockquote_is_left_as_the_sender_styled_it() {
        let mail = render_html(
            r#"<body style="margin:0"><blockquote><div>A saying</div></blockquote></body>"#,
        );
        let saying = text_line(&mail, "A saying");
        assert_eq!(background_beside(&mail, saying), [255, 255, 255]);
    }

    /// Notification emails wrap their 600px column in a cell declared twice
    /// as wide. A browser shrinks that cell to the room it has; keeping its
    /// declared width pushed the column, centred in it, off to the right.
    #[test]
    fn a_cell_wider_than_its_room_shrinks_to_fit() {
        let mail = render_html(
            r#"<body style="margin:0"><div style="max-width:300px;margin:0 auto">
               <table cellspacing="0" cellpadding="0" align="center"><tr>
               <td width="1200" align="center">
               <table cellspacing="0" cellpadding="0" align="center"
                      style="max-width:300px;margin:0 auto"><tr><td>
               <a href="https://example.com/">Open</a> the page to read the rest of
               this notice, which is long enough to fill the column.
               </td></tr></table></td></tr></table></div></body>"#,
        );
        assert!(
            mail.width <= 601.0,
            "the email overflowed to {}",
            mail.width
        );
        let left = mail.links[0].x;
        assert!(
            (150.0..200.0).contains(&left),
            "the column starts at {left}, outside its 300px room at 150"
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
            appearance: Appearance::default(),
            quote_only: false,
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
