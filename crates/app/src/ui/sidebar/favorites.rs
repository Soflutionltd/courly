use iced::Element;
use rust_i18n::t;

use crate::ui::widgets;

use super::{Sidebar, SidebarMessage};

/// The people starred as favourites: a click shows the mail that person
/// sent.
pub(super) fn favorites(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let children: Vec<Element<'_, SidebarMessage>> = sidebar
        .favorites
        .iter()
        .map(|contact| {
            widgets::nav_button(
                None,
                &contact.name,
                false,
                sidebar.text_size.nav(sidebar.density),
                None,
                SidebarMessage::SelectFavorite(contact.address.clone()),
            )
        })
        .collect();

    widgets::collapsible_section(
        t!("main.sidebar.favorites"),
        sidebar.favorites_expanded,
        SidebarMessage::ToggleFavoritesSection,
        children,
    )
}
