use iced::Length;
use iced::widget::{Space, column, text};
use rust_i18n::t;

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

/// Display name of a canonical select value on the General tab.
fn option_label(value: &str) -> Option<&'static str> {
    Some(tr(match value {
        "System" => t!("settings.general.option.system"),
        "Light" => t!("settings.general.option.light"),
        "Dark" => t!("settings.general.option.dark"),
        "Theme" => t!("settings.general.option.theme"),
        "Compact" => t!("settings.general.option.compact"),
        "Default" => t!("settings.general.option.default"),
        "Spacious" => t!("settings.general.option.spacious"),
        "XSmall" => t!("settings.general.option.xsmall"),
        "Small" => t!("settings.general.option.small"),
        "Large" => t!("settings.general.option.large"),
        "XLarge" => t!("settings.general.option.xlarge"),
        "Always White" => t!("settings.general.option.always_white"),
        "Match Theme" => t!("settings.general.option.match_theme"),
        "Auto" => t!("settings.general.option.auto"),
        "Relative Offset" => t!("settings.general.option.relative_offset"),
        "Absolute" => t!("settings.general.option.absolute"),
        "None" => t!("settings.general.option.none"),
        "5 seconds" => t!("settings.general.option.seconds_5"),
        "10 seconds" => t!("settings.general.option.seconds_10"),
        "30 seconds" => t!("settings.general.option.seconds_30"),
        "Instantly" => t!("settings.general.option.instantly"),
        "After 2 Seconds" => t!("settings.general.option.after_2_seconds"),
        "Manually" => t!("settings.general.option.manually"),
        "2" => t!("settings.general.option.mail_check_realtime"),
        "15" => t!("settings.general.option.mail_check_15_seconds"),
        "60" => t!("settings.general.option.mail_check_minute"),
        _ => return None,
    }))
}

pub(super) fn general_tab(state: &Settings) -> iced::Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill.max(SETTINGS_CONTENT_MAX_WIDTH));

    col = col.push(section(
        tr(t!("settings.general.appearance")),
        vec![
            setting_row(
                tr(t!("settings.general.theme")),
                translated_select(
                    &["System", "Light", "Dark", "Theme"],
                    &state.theme,
                    option_label,
                    state.open_select == Some(SelectField::Theme),
                    SettingsMessage::ToggleSelect(SelectField::Theme),
                    SettingsMessage::ThemeChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::Theme),
            ),
            setting_row(
                tr(t!("settings.general.email_density")),
                translated_select(
                    &["Compact", "Default", "Spacious"],
                    &state.density,
                    option_label,
                    state.open_select == Some(SelectField::Density),
                    SettingsMessage::ToggleSelect(SelectField::Density),
                    SettingsMessage::DensityChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::Density),
            ),
            setting_row(
                tr(t!("settings.general.font_size")),
                translated_select(
                    &["XSmall", "Small", "Default", "Large", "XLarge"],
                    &state.font_size,
                    option_label,
                    state.open_select == Some(SelectField::FontSize),
                    SettingsMessage::ToggleSelect(SelectField::FontSize),
                    SettingsMessage::FontSizeChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::FontSize),
            ),
            setting_row(
                tr(t!("settings.general.email_body_background")),
                translated_select(
                    &["Always White", "Match Theme", "Auto"],
                    state.email_body_background.label(),
                    option_label,
                    state.open_select == Some(SelectField::EmailBodyBg),
                    SettingsMessage::ToggleSelect(SelectField::EmailBodyBg),
                    SettingsMessage::EmailBodyBgChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::EmailBodyBg),
            ),
            setting_row_with_description(
                tr(t!("settings.general.email_zoom")),
                Some(tr(t!("settings.general.email_zoom_description"))),
                widgets::select(
                    EMAIL_ZOOM_LEVELS,
                    &state.email_zoom,
                    state.open_select == Some(SelectField::EmailZoom),
                    SettingsMessage::ToggleSelect(SelectField::EmailZoom),
                    SettingsMessage::EmailZoomChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::EmailZoom),
            ),
            slider_row(
                tr(t!("settings.general.scale")),
                None,
                None,
                1.0..=4.0,
                state.scale_preview.unwrap_or(state.scale),
                1.0,
                0.125,
                SettingsMessage::ScaleDragged,
                Some(SettingsMessage::ScaleReleased),
            ),
            setting_row_with_description(
                tr(t!("settings.general.message_dates")),
                Some(tr(t!("settings.general.message_dates_description"))),
                translated_select(
                    &["Relative Offset", "Absolute"],
                    match state.date_display {
                        crate::db::DateDisplay::RelativeOffset => "Relative Offset",
                        crate::db::DateDisplay::Absolute => "Absolute",
                    },
                    option_label,
                    state.open_select == Some(SelectField::DateDisplay),
                    SettingsMessage::ToggleSelect(SelectField::DateDisplay),
                    SettingsMessage::DateDisplayChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::DateDisplay),
            ),
            toggle_row(
                tr(t!("settings.general.group_by_date")),
                t!("settings.general.group_by_date_description"),
                state.thread_date_groups,
                SettingsMessage::ToggleThreadDateGroups,
            ),
            toggle_row(
                tr(t!("settings.general.show_status_bar")),
                t!("settings.general.show_status_bar_description"),
                state.sync_status_bar,
                SettingsMessage::ToggleSyncStatusBar,
            ),
        ],
    ));

    col = col.push(section(
        tr(t!("settings.general.behavior")),
        vec![
            setting_row(
                tr(t!("settings.general.undo_send_delay")),
                translated_select(
                    &["None", "5 seconds", "10 seconds", "30 seconds"],
                    &state.undo_delay,
                    option_label,
                    state.open_select == Some(SelectField::UndoDelay),
                    SettingsMessage::ToggleSelect(SelectField::UndoDelay),
                    SettingsMessage::UndoDelayChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::UndoDelay),
            ),
            setting_row(
                tr(t!("settings.general.mark_as_read")),
                translated_select(
                    &["Instantly", "After 2 Seconds", "Manually"],
                    &state.mark_as_read,
                    option_label,
                    state.open_select == Some(SelectField::MarkAsRead),
                    SettingsMessage::ToggleSelect(SelectField::MarkAsRead),
                    SettingsMessage::MarkAsReadChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::MarkAsRead),
            ),
            setting_row_with_description(
                tr(t!("settings.general.mail_check")),
                Some(tr(t!("settings.general.mail_check_description"))),
                translated_select(
                    MAIL_CHECK_INTERVALS,
                    &state.mail_check_interval,
                    option_label,
                    state.open_select == Some(SelectField::MailCheckInterval),
                    SettingsMessage::ToggleSelect(SelectField::MailCheckInterval),
                    SettingsMessage::MailCheckIntervalChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::MailCheckInterval),
            ),
        ],
    ));

    col = col.push(section(
        tr(t!("settings.general.reading_pane")),
        radio_group(
            &[
                (tr(t!("settings.general.pane_right")), None, "Right"),
                (tr(t!("settings.general.pane_bottom")), None, "Bottom"),
                (tr(t!("settings.general.pane_hidden")), None, "Hidden"),
            ],
            Some(state.reading_pane_position.as_str()),
            |v| SettingsMessage::ReadingPaneChanged(v.to_string()),
        ),
    ));

    col = col.push(section(
        tr(t!("settings.general.updates")),
        vec![toggle_row(
            tr(t!("settings.general.auto_update")),
            t!(
                "settings.general.auto_update_description",
                version = env!("CARGO_PKG_VERSION")
            ),
            state.auto_update,
            SettingsMessage::ToggleAutoUpdate,
        )],
    ));

    let privacy_help_id = "privacy-security";
    let privacy_help_visible = state.hovered_help.as_deref() == Some(privacy_help_id);
    col = col.push(section_with_help(
        tr(t!("settings.general.privacy_security")),
        SectionHelp {
            id: privacy_help_id,
            content: column![
                text(t!("settings.general.remote_images_help"))
                    .size(TEXT_SM)
                    .style(theme::TextClass::OnPrimary.style()),
                Space::new().height(SPACE_XS),
                text(t!("settings.general.phishing_help"))
                    .size(TEXT_SM)
                    .style(theme::TextClass::OnPrimary.style()),
            ]
            .into(),
            visible: privacy_help_visible,
        },
        vec![
            toggle_row(
                tr(t!("settings.general.block_remote_images")),
                t!("settings.general.block_remote_images_description"),
                state.block_remote_images,
                SettingsMessage::ToggleBlockRemoteImages,
            ),
            toggle_row(
                tr(t!("settings.general.phishing_detection")),
                t!("settings.general.phishing_detection_description"),
                state.phishing_detection,
                SettingsMessage::TogglePhishingDetection,
            ),
        ],
    ));

    col.into()
}
