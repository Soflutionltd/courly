use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;

pub(super) fn shortcuts_tab<'a>() -> Element<'a, SettingsMessage> {
    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            tr(t!("settings.reference.navigation")),
            &[
                (tr(t!("settings.reference.next_thread")), "j"),
                (tr(t!("settings.reference.previous_thread")), "k"),
                (tr(t!("settings.reference.go_to_inbox")), "g i"),
                (tr(t!("settings.reference.search")), "/"),
                (tr(t!("settings.reference.close_dismiss")), "Esc"),
            ],
        ),
        (
            tr(t!("settings.reference.thread")),
            &[
                (tr(t!("settings.reference.archive")), "e"),
                (
                    tr(t!("settings.reference.delete")),
                    tr(t!("settings.reference.delete_keys")),
                ),
                (tr(t!("settings.reference.reply")), "r"),
                (tr(t!("settings.reference.reply_all")), "a"),
                (tr(t!("settings.reference.forward")), "f"),
                (tr(t!("settings.reference.star_unstar")), "s"),
                (tr(t!("settings.reference.mute_thread")), "m"),
                (tr(t!("settings.reference.mark_unread")), "u"),
            ],
        ),
        (
            tr(t!("settings.reference.composing")),
            &[(tr(t!("settings.reference.new_message")), "c")],
        ),
    ];

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill.max(SETTINGS_CONTENT_MAX_WIDTH));

    for (category, items) in sections {
        let rows: Vec<RowBuilder<'_>> = items
            .iter()
            .map(|(desc, key)| {
                settings_row_container(
                    SETTINGS_ROW_HEIGHT,
                    row![
                        container(text(*desc).size(TEXT_LG).style(text::secondary))
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        container(text(*key).size(TEXT_SM).style(text::secondary))
                            .padding(PAD_ICON_BTN)
                            .style(theme::ContainerClass::KeyBadge.style()),
                    ]
                    .align_y(Alignment::Center),
                )
            })
            .collect();

        col = col.push(section(category, rows));
    }

    col.into()
}

pub(super) fn about_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill.max(SETTINGS_CONTENT_MAX_WIDTH));

    col = col.push(section(
        tr(t!("settings.reference.application")),
        vec![
            info_row(
                &t!("settings.reference.version"),
                crate::updater::current_version(),
            ),
            info_row(&t!("settings.reference.iced_version"), "0.15.0-dev"),
            info_row(&t!("settings.reference.platform"), std::env::consts::OS),
            info_row(
                &t!("settings.reference.architecture"),
                std::env::consts::ARCH,
            ),
        ],
    ));

    col = col.push(section(
        tr(t!("settings.reference.updates")),
        vec![action_row(
            tr(t!("settings.reference.software_updates")),
            Some(
                state
                    .update_check_note
                    .as_deref()
                    .unwrap_or(tr(t!("settings.reference.check_for_updates"))),
            ),
            None,
            ActionKind::InApp,
            SettingsMessage::CheckForUpdates,
        )],
    ));

    col = col.push(section(
        tr(t!("settings.reference.license")),
        vec![static_row(
            container(column![
                text("PolyForm Shield 1.0.0")
                    .size(TEXT_LG)
                    .style(text::base),
                Space::new().height(SPACE_XS),
                text(t!("settings.reference.license_body"))
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
                Space::new().height(SPACE_XXS),
                text("https://polyformproject.org/licenses/shield/1.0.0")
                    .size(TEXT_SM)
                    .style(text::primary),
                Space::new().height(SPACE_SM),
                text(t!("settings.reference.copyright"))
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ])
            .padding(PAD_SETTINGS_ROW),
        )],
    ));

    col = col.push(section(
        tr(t!("settings.reference.links")),
        vec![action_row(
            tr(t!("settings.reference.github_repository")),
            Some(crate::updater::REPOSITORY),
            Some(icon::globe()),
            ActionKind::Url,
            SettingsMessage::OpenGithub,
        )],
    ));

    col.into()
}
