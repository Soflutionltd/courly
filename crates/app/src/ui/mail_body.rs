//! HTML email bodies rendered off the UI thread by the `mail-html` engine
//! and shown as image tiles with clickable link areas.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use iced::widget::{
    Space, button, column, container, image, mouse_area, row, scrollable, sensor, stack, text,
};
use iced::{Alignment, Element, Length, Padding, mouse};
use mail_html::{Bytes, LinkArea, RemoteFetcher, RenderRequest, Url};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;

/// Largest remote resource accepted when the user allows remote content.
const REMOTE_MAX_BYTES: u64 = 15 * 1024 * 1024;
const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);

/// Source of render tickets. A result is applied only if its ticket is the
/// one its state is waiting for, so renders for replaced content are dropped.
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

static HTTP: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(REMOTE_TIMEOUT)
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| log::warn!("remote image client unavailable: {e}"))
        .ok()
});

/// A rendered email body, ready to draw.
pub struct RenderedBody {
    width: f32,
    height: f32,
    tiles: Vec<(image::Handle, f32)>,
    links: Vec<LinkArea>,
    blocked_remote: usize,
}

impl std::fmt::Debug for RenderedBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderedBody")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("tiles", &self.tiles.len())
            .field("links", &self.links.len())
            .field("blocked_remote", &self.blocked_remote)
            .finish()
    }
}

/// Inputs of one render pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderInputs {
    pub ticket: u64,
    pub width: f32,
    pub zoom: f32,
    pub allow_remote: bool,
}

/// Per-message render state in the reading pane.
#[derive(Debug, Clone)]
pub struct HtmlBodyState {
    desired_width: Option<f32>,
    zoom: f32,
    /// Ticket of the render in flight, if any.
    in_flight: Option<u64>,
    failed: bool,
    allow_remote: bool,
    rendered: Option<Arc<RenderedBody>>,
}

thread_local! {
    static READING_ZOOM: Cell<f32> = const { Cell::new(1.0) };
}

/// Zoom applied to HTML bodies, set from the user's preference.
pub fn reading_zoom() -> f32 {
    READING_ZOOM.with(Cell::get)
}

pub fn set_reading_zoom(zoom: f32) {
    READING_ZOOM.with(|cell| cell.set(zoom));
}

impl Default for HtmlBodyState {
    fn default() -> Self {
        Self {
            desired_width: None,
            zoom: reading_zoom(),
            in_flight: None,
            failed: false,
            allow_remote: false,
            rendered: None,
        }
    }
}

impl HtmlBodyState {
    /// Change the reading zoom. Returns true when a new render should start.
    pub fn set_zoom(&mut self, zoom: f32) -> bool {
        if (self.zoom - zoom).abs() < f32::EPSILON {
            return false;
        }
        self.zoom = zoom;
        self.desired_width.is_some() && self.in_flight.is_none()
    }

    /// Record the width the body is laid out at. Returns true when a new
    /// render should start.
    pub fn set_width(&mut self, width: f32) -> bool {
        let width = width.floor();
        if width < 1.0 || self.desired_width == Some(width) {
            return false;
        }
        self.desired_width = Some(width);
        self.in_flight.is_none()
    }

    /// Allow remote content for this message. Returns true when a new
    /// render should start.
    pub fn allow_remote(&mut self) -> bool {
        if self.allow_remote {
            return false;
        }
        self.allow_remote = true;
        self.in_flight.is_none()
    }

    /// Store a finished render. Returns true when the width or zoom changed
    /// while it was running and another pass is needed. Results for any
    /// other ticket are ignored.
    pub fn finish(&mut self, inputs: RenderInputs, body: Option<Arc<RenderedBody>>) -> bool {
        if self.in_flight != Some(inputs.ticket) {
            return false;
        }
        self.in_flight = None;
        match body {
            Some(body) => {
                self.failed = false;
                self.rendered = Some(body);
            }
            None => self.failed = true,
        }
        (self.zoom - inputs.zoom).abs() >= f32::EPSILON
            || self
                .desired_width
                .is_some_and(|w| (w - inputs.width).abs() >= 1.0)
    }

    /// Mark a render as started and return its inputs.
    pub fn start(&mut self) -> Option<RenderInputs> {
        let width = self.desired_width?;
        let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
        self.in_flight = Some(ticket);
        Some(RenderInputs {
            ticket,
            width,
            zoom: self.zoom,
            allow_remote: self.allow_remote,
        })
    }
}

/// Extract raw bytes for inline (`cid:`) images.
pub fn inline_image_bytes(handles: &HashMap<String, image::Handle>) -> HashMap<String, Bytes> {
    handles
        .iter()
        .filter_map(|(cid, handle)| match handle {
            image::Handle::Bytes(_, bytes) => Some((cid.clone(), bytes.clone())),
            _ => None,
        })
        .collect()
}

/// Render one body on a blocking worker. `None` when the engine failed.
pub async fn render(
    html: String,
    inputs: RenderInputs,
    inline_images: HashMap<String, Bytes>,
) -> Option<Arc<RenderedBody>> {
    let remote = inputs
        .allow_remote
        .then(|| remote_fetcher(tokio::runtime::Handle::current()));
    let result = tokio::task::spawn_blocking(move || {
        let mail = mail_html::render(RenderRequest {
            html,
            width: inputs.width,
            zoom: inputs.zoom,
            inline_images,
            remote,
        });
        let tiles = mail
            .tiles
            .into_iter()
            .map(|tile| {
                (
                    image::Handle::from_rgba(tile.px_width, tile.px_height, tile.rgba),
                    tile.height,
                )
            })
            .collect();
        Arc::new(RenderedBody {
            width: mail.width,
            height: mail.height,
            tiles,
            links: mail.links,
            blocked_remote: mail.blocked_remote,
        })
    })
    .await;
    result
        .map_err(|e| log::error!("HTML email render failed: {e}"))
        .ok()
}

fn remote_fetcher(runtime: tokio::runtime::Handle) -> RemoteFetcher {
    Arc::new(move |url: &Url| {
        let client = HTTP.as_ref()?;
        let url = url.clone();
        runtime.block_on(async move {
            let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
            if response
                .content_length()
                .is_some_and(|len| len > REMOTE_MAX_BYTES)
            {
                return None;
            }
            let body = response.bytes().await.ok()?;
            (body.len() as u64 <= REMOTE_MAX_BYTES).then_some(body)
        })
    })
}

/// The HTML body of one message. Reports its available width through
/// `on_width` so the owner can (re)render at the right size.
pub fn view<'a, M: Clone + 'a>(
    state: Option<&'a HtmlBodyState>,
    fallback_text: Option<&'a str>,
    on_width: impl Fn(f32) -> M + 'a,
    on_link: impl Fn(String) -> M + 'a,
    on_show_remote: M,
) -> Element<'a, M> {
    let content: Element<'a, M> = match state {
        Some(HtmlBodyState {
            rendered: Some(body),
            allow_remote,
            desired_width,
            ..
        }) => {
            let overflows = desired_width.is_some_and(|w| body.width > w + 1.0);
            let page = rendered_page(body, overflows, on_link);
            if body.blocked_remote > 0 && !allow_remote {
                column![remote_banner(on_show_remote), page]
                    .spacing(SPACE_XS)
                    .into()
            } else {
                page
            }
        }
        Some(HtmlBodyState { failed: true, .. }) => text(
            fallback_text
                .unwrap_or("This message could not be displayed.")
                .to_string(),
        )
        .size(TEXT_LG)
        .into(),
        _ => Space::new()
            .width(Length::Fill)
            .height(HTML_BODY_PLACEHOLDER_HEIGHT)
            .into(),
    };

    let on_width = Rc::new(on_width);
    let on_resize = Rc::clone(&on_width);
    sensor(container(content).width(Length::Fill))
        .on_show(move |size| on_width(size.width))
        .on_resize(move |size| on_resize(size.width))
        .into()
}

fn rendered_page<'a, M: Clone + 'a>(
    body: &'a RenderedBody,
    overflows: bool,
    on_link: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    let mut tiles = column![].width(body.width);
    for (handle, height) in &body.tiles {
        tiles = tiles.push(
            image(handle.clone())
                .width(body.width)
                .height(*height)
                .content_fit(iced::ContentFit::Fill),
        );
    }

    let mut layers = stack![tiles].width(body.width).height(body.height);
    for link in &body.links {
        let hit = mouse_area(Space::new().width(link.width).height(link.height))
            .on_press(on_link(link.href.clone()))
            .interaction(mouse::Interaction::Pointer);
        layers = layers.push(container(hit).padding(Padding {
            top: link.y.max(0.0),
            left: link.x.max(0.0),
            right: 0.0,
            bottom: 0.0,
        }));
    }

    if !overflows {
        return layers.into();
    }
    scrollable(layers)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default(),
        ))
        .width(Length::Fill)
        .into()
}

fn remote_banner<'a, M: Clone + 'a>(on_show_remote: M) -> Element<'a, M> {
    container(
        row![
            icon::shield()
                .size(ICON_SM)
                .style(theme::TextClass::Muted.style()),
            text("Remote images were blocked to protect your privacy.")
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
            Space::new().width(Length::Fill),
            button(text("Show images").size(TEXT_SM))
                .on_press(on_show_remote)
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::Secondary.style()),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_REMOTE_BANNER)
    .width(Length::Fill)
    .style(theme::ContainerClass::Badge.style())
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(width: f32) -> Option<Arc<RenderedBody>> {
        Some(Arc::new(RenderedBody {
            width,
            height: 10.0,
            tiles: Vec::new(),
            links: Vec::new(),
            blocked_remote: 0,
        }))
    }

    #[test]
    fn first_width_starts_a_render() {
        let mut state = HtmlBodyState::default();
        assert!(state.set_width(600.4));
        let inputs = state.start().expect("width known");
        assert!((inputs.width - 600.0).abs() < f32::EPSILON);
        assert!(!inputs.allow_remote);
    }

    #[test]
    fn width_change_during_render_triggers_one_more_pass() {
        let mut state = HtmlBodyState::default();
        state.set_width(600.0);
        let inputs = state.start().expect("started");
        assert!(!state.set_width(500.0), "no second render while in flight");
        assert!(state.finish(inputs, body(inputs.width)));
        let inputs = state.start().expect("restarted");
        assert!(!state.finish(inputs, body(inputs.width)));
    }

    #[test]
    fn results_for_replaced_state_are_ignored() {
        let mut state = HtmlBodyState::default();
        state.set_width(600.0);
        let first = state.start().expect("started");
        let mut replaced = HtmlBodyState::default();
        assert!(!replaced.finish(first, body(first.width)));
        assert!(replaced.rendered.is_none());
        state.set_width(700.0);
        let newer = state.start().expect("restarted");
        assert!(!state.finish(first, body(first.width)));
        assert!(state.rendered.is_none());
        assert!(!state.finish(newer, body(700.0)));
        assert!(state.rendered.is_some());
    }

    #[test]
    fn allowing_remote_rerenders_once() {
        let mut state = HtmlBodyState::default();
        state.set_width(600.0);
        assert!(state.allow_remote());
        assert!(!state.allow_remote());
        assert!(state.start().expect("started").allow_remote);
    }

    #[test]
    fn zoom_change_rerenders_at_the_new_zoom() {
        let mut state = HtmlBodyState::default();
        assert!(!state.set_zoom(0.9), "no width yet, nothing to render");
        state.set_width(600.0);
        let inputs = state.start().expect("started");
        assert!((inputs.zoom - 0.9).abs() < f32::EPSILON);
        assert!(!state.set_zoom(1.25), "no second render while in flight");
        assert!(state.finish(inputs, body(600.0)));
        let inputs = state.start().expect("restarted");
        assert!((inputs.zoom - 1.25).abs() < f32::EPSILON);
        assert!(!state.finish(inputs, body(600.0)));
    }
}
