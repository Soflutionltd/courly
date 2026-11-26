//! Print a message, as Outlook's Print does.
//!
//! iced has no printing, so the message is laid out by WebKit, Apple's own
//! HTML engine, and handed to the system print panel: the user gets the
//! printer choice, the preview, page setup and Save as PDF exactly as in
//! Apple Mail. The page carries the message's headers above its body, with
//! inline images embedded and scripts off.

use iced::Task;
use rust_i18n::t;

use crate::{Message, ReadyApp};

impl ReadyApp {
    /// Print the open conversation's latest message through the system
    /// print panel.
    pub(crate) fn print_open_message(&mut self) -> Task<Message> {
        let Some(message) = self.reading_pane.thread_messages.first() else {
            return Task::none();
        };
        let images = self.reading_pane.inline_images();
        let body_html = message.body_html.as_deref().map(|html| {
            crate::signature_html::embed_inline_images(html, |content_id| {
                images.get(content_id).map(AsRef::as_ref)
            })
        });
        let from = match (
            message.from_name.as_deref(),
            message.from_address.as_deref(),
        ) {
            (Some(name), Some(address)) if !name.is_empty() => format!("{name} <{address}>"),
            (_, Some(address)) => address.to_string(),
            (Some(name), None) => name.to_string(),
            (None, None) => String::new(),
        };
        let date = message.date.map(format_date).unwrap_or_default();
        let html = printable_html(
            &PrintableMessage {
                subject: message.subject.as_deref().unwrap_or_default(),
                from: &from,
                to: message.to_addresses.as_deref().unwrap_or_default(),
                cc: message.cc_addresses.as_deref().unwrap_or_default(),
                date: &date,
                body_html: body_html.as_deref(),
                body_text: message.body_text.as_deref(),
            },
            &HeaderLabels {
                from: t!("main.print.from").into_owned(),
                to: t!("main.print.to").into_owned(),
                cc: t!("main.print.cc").into_owned(),
                date: t!("main.print.date").into_owned(),
            },
        );
        if let Err(error) = print_html(&html) {
            self.status_bar
                .show_confirmation(t!("main.print.failed", error = error).into_owned());
        }
        Task::none()
    }
}

/// A message's date, in milliseconds since the epoch, as local day and time.
fn format_date(millis: i64) -> String {
    jiff::Timestamp::from_millisecond(millis)
        .map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%d/%m/%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// What a printed page shows of a message.
pub(crate) struct PrintableMessage<'a> {
    pub subject: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub cc: &'a str,
    pub date: &'a str,
    /// The body as HTML, its inline images already embedded.
    pub body_html: Option<&'a str>,
    /// The body as plain text, used when there is no HTML part.
    pub body_text: Option<&'a str>,
}

/// The labels of the header block, in the user's language.
pub(crate) struct HeaderLabels {
    pub from: String,
    pub to: String,
    pub cc: String,
    pub date: String,
}

/// The whole page WebKit prints: the subject as a title, then the From,
/// To, Cc and Date lines, a rule, and the body.
pub(crate) fn printable_html(message: &PrintableMessage<'_>, labels: &HeaderLabels) -> String {
    let mut rows = String::new();
    for (label, value) in [
        (&labels.from, message.from),
        (&labels.to, message.to),
        (&labels.cc, message.cc),
        (&labels.date, message.date),
    ] {
        if value.trim().is_empty() {
            continue;
        }
        rows.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            escape(label),
            escape(value)
        ));
    }
    let body = match (message.body_html, message.body_text) {
        (Some(html), _) if !html.trim().is_empty() => {
            crate::pop_out::compose::embeddable_body(html)
        }
        (_, Some(text)) => format!("<pre class=\"text\">{}</pre>", escape(text)),
        _ => String::new(),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>\
         body{{font:12pt -apple-system,Helvetica,Arial,sans-serif;color:#000;margin:0}}\
         h1{{font-size:16pt;margin:0 0 8pt}}\
         table.headers{{border-collapse:collapse;margin-bottom:8pt}}\
         table.headers th{{text-align:left;padding:1pt 10pt 1pt 0;color:#555;font-weight:600;vertical-align:top}}\
         table.headers td{{padding:1pt 0}}\
         hr{{border:0;border-top:1px solid #999;margin:8pt 0 12pt}}\
         pre.text{{white-space:pre-wrap;font:inherit}}\
         img{{max-width:100%}}\
         </style></head><body><h1>{title}</h1>\
         <table class=\"headers\">{rows}</table><hr>{body}</body></html>",
        title = escape(message.subject),
    )
}

fn escape(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Lay `html` out in WebKit and open the system print panel on it.
#[cfg(target_os = "macos")]
pub(crate) fn print_html(html: &str) -> Result<(), String> {
    platform::print_html(html)
}

/// Printing goes through the macOS print panel; other systems have none.
#[cfg(not(target_os = "macos"))]
pub(crate) fn print_html(_html: &str) -> Result<(), String> {
    Err("printing is available on macOS only".to_string())
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::NSPrintInfo;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView, WKWebViewConfiguration};

    /// A4 in points: the web view needs a real frame to lay the page out
    /// before the print panel sizes it to the chosen paper.
    const PAGE: CGSize = CGSize {
        width: 595.0,
        height: 842.0,
    };

    thread_local! {
        /// The web view and its delegate, kept alive while the page loads.
        static PENDING: RefCell<Option<(Retained<WKWebView>, Retained<PrintDelegate>)>> =
            const { RefCell::new(None) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and the class
        // does not implement Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "CourlyPrintDelegate"]
        struct PrintDelegate;

        unsafe impl NSObjectProtocol for PrintDelegate {}

        unsafe impl WKNavigationDelegate for PrintDelegate {
            #[unsafe(method(webView:didFinishNavigation:))]
            fn did_finish(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
                let info = NSPrintInfo::sharedPrintInfo();
                // SAFETY: the web view finished loading on the main thread.
                let operation = unsafe { web_view.printOperationWithPrintInfo(&info) };
                operation.setShowsPrintPanel(true);
                operation.setShowsProgressPanel(true);
                // App-modal: returns once the user printed or cancelled.
                operation.runOperation();
                release();
            }

            #[unsafe(method(webView:didFailNavigation:withError:))]
            fn did_fail(
                &self,
                _web_view: &WKWebView,
                _navigation: Option<&WKNavigation>,
                error: &NSError,
            ) {
                log::warn!("print: the message could not be laid out: {error}");
                release();
            }
        }
    );

    impl PrintDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: NSObject's designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    fn release() {
        PENDING.with(|pending| pending.borrow_mut().take());
    }

    pub(super) fn print_html(html: &str) -> Result<(), String> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "printing must start on the main thread".to_string())?;
        if PENDING.with(|pending| pending.borrow().is_some()) {
            return Err("a message is already being prepared for printing".to_string());
        }
        // SAFETY: main thread, plain configuration objects.
        let web_view = unsafe {
            let configuration = WKWebViewConfiguration::new(mtm);
            configuration
                .defaultWebpagePreferences()
                .setAllowsContentJavaScript(false);
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                CGRect::new(CGPoint::new(0.0, 0.0), PAGE),
                &configuration,
            )
        };
        let delegate = PrintDelegate::new(mtm);
        // SAFETY: the delegate outlives the load: both stay in PENDING.
        unsafe {
            web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            web_view.loadHTMLString_baseURL(&NSString::from_str(html), None);
        }
        PENDING.with(|pending| *pending.borrow_mut() = Some((web_view, delegate)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> HeaderLabels {
        HeaderLabels {
            from: "De".into(),
            to: "À".into(),
            cc: "Cc".into(),
            date: "Date".into(),
        }
    }

    #[test]
    fn the_page_shows_the_headers_then_the_html_body() {
        let html = printable_html(
            &PrintableMessage {
                subject: "Devis <2026>",
                from: "Jo <jo@example.com>",
                to: "sam@example.com",
                cc: "",
                date: "26 nov. 2026 10:00",
                body_html: Some("<html><body><p>Bonjour</p></body></html>"),
                body_text: None,
            },
            &labels(),
        );
        assert!(html.contains("<h1>Devis &lt;2026&gt;</h1>"));
        assert!(html.contains("<th>De</th><td>Jo &lt;jo@example.com&gt;</td>"));
        assert!(!html.contains("<th>Cc</th>"), "an empty Cc is left out");
        assert!(html.contains("<p>Bonjour</p>"));
        assert!(!html.contains("<html><body><p>"), "only the body is kept");
    }

    #[test]
    fn a_text_only_message_is_printed_as_escaped_text() {
        let html = printable_html(
            &PrintableMessage {
                subject: "Note",
                from: "a@example.com",
                to: "",
                cc: "",
                date: "",
                body_html: None,
                body_text: Some("1 < 2 & 3"),
            },
            &labels(),
        );
        assert!(html.contains("<pre class=\"text\">1 &lt; 2 &amp; 3</pre>"));
    }
}
