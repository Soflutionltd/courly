//! The card above the mail a person sent: who they are, a star that keeps
//! them in the favourites of the folder column, and a new message to them.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::font;
use crate::icon;
use crate::message::Message;
use crate::ui::layout::*;
use crate::ui::search_suggest::SearchSuggestMessage;
use crate::ui::{theme, widgets};

/// The person whose mail the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPerson {
    pub address: String,
    pub name: String,
    pub is_favorite: bool,
}

pub fn card<'a>(person: SearchPerson) -> Element<'a, Message> {
    let star_style: fn(&iced::Theme) -> text::Style = if person.is_favorite {
        text::warning
    } else {
        text::secondary
    };
    let star = button(icon::star().size(ICON_MD).style(star_style))
        .on_press(Message::SearchSuggest(SearchSuggestMessage::ToggleFavorite))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    let compose = button(icon::mail().size(ICON_MD).style(text::secondary))
        .on_press(Message::SearchSuggest(
            SearchSuggestMessage::ComposeToSender,
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let avatar = widgets::sender_avatar(&person.name, Some(&person.address), AVATAR_MESSAGE_CARD);
    let address = (person.name != person.address).then(|| {
        text(person.address)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style())
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End)
    });
    let identity = column![
        text(person.name)
            .size(TEXT_LG)
            .font(font::text_semibold())
            .style(text::base)
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End),
        address,
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill);

    container(
        row![
            avatar,
            identity,
            Space::new().width(SPACE_XXS),
            star,
            compose,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_PANEL_HEADER)
    .width(Length::Fill)
    .into()
}
