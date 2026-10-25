//! Linux system font detection via xdg-desktop-portal.
//!
//! Queries `org.freedesktop.portal.Settings.ReadOne` for the
//! `org.gnome.desktop.interface` namespace. Works on GNOME (via
//! xdg-desktop-portal-gnome), KDE (via xdg-desktop-portal-kde), and
//! any other DE with a portal backend that exposes these keys.

use crate::{SystemFonts, parse_font_description};

const NAMESPACE: &str = "org.gnome.desktop.interface";
const KEY_UI_FONT: &str = "font-name";
const KEY_MONOSPACE_FONT: &str = "monospace-font-name";
const KEY_DOCUMENT_FONT: &str = "document-font-name";

/// Read a single setting from xdg-desktop-portal.
///
/// The portal returns `Variant(Variant(String))` - a doubly-wrapped variant.
/// We unwrap both layers to get the inner string.
async fn read_setting(proxy: &zbus::Proxy<'_>, namespace: &str, key: &str) -> Option<String> {
    let reply: zbus::OwnedVariant = proxy.call("ReadOne", &(namespace, key)).await.ok()?;

    // The portal wraps the value in Variant(Variant(value)).
    // First unwrap: OwnedVariant -> Variant (outer variant)
    let inner: zbus::Variant<'_> = reply.downcast_ref::<zbus::Variant<'_>>().ok()?.clone();
    // Second unwrap: Variant -> String
    let s: &str = inner.downcast_ref().ok()?;
    Some(s.to_string())
}

pub(crate) async fn detect() -> SystemFonts {
    let mut fonts = SystemFonts::default();

    let connection = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::debug!("failed to connect to session D-Bus: {e}");
            return fonts;
        }
    };

    let proxy = match zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&connection)
        .interface("org.freedesktop.portal.Settings")
        .path("/org/freedesktop/portal/desktop")
        .destination("org.freedesktop.portal.Desktop")
        .build()
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("failed to create portal settings proxy: {e}");
            return fonts;
        }
    };

    if let Some(desc) = read_setting(&proxy, NAMESPACE, KEY_UI_FONT).await {
        fonts.ui = parse_font_description(&desc);
        if fonts.ui.is_some() {
            tracing::debug!("system UI font: {desc}");
        }
    }

    if let Some(desc) = read_setting(&proxy, NAMESPACE, KEY_MONOSPACE_FONT).await {
        fonts.monospace = parse_font_description(&desc);
        if fonts.monospace.is_some() {
            tracing::debug!("system monospace font: {desc}");
        }
    }

    if let Some(desc) = read_setting(&proxy, NAMESPACE, KEY_DOCUMENT_FONT).await {
        fonts.document = parse_font_description(&desc);
        if fonts.document.is_some() {
            tracing::debug!("system document font: {desc}");
        }
    }

    fonts
}
