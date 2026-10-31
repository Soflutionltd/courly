use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length, Padding};
use rust_i18n::t;
use std::borrow::Cow;

use crate::db::ThreadAttachment;
use crate::icon;
use crate::ui::anchored_overlay::anchored_overlay;
use crate::ui::layout::{
    ATTACHMENT_CHIP_HEIGHT, ATTACHMENT_CHIP_NAME_MAX_CHARS, ATTACHMENT_CHIP_NAME_MAX_WIDTH,
    ATTACHMENT_MENU_WIDTH, DROPDOWN_ITEM_HEIGHT, ICON_MD, ICON_SM, ICON_XS, PAD_DROPDOWN,
    PAD_NAV_ITEM, SLOT_DROPDOWN, SPACE_XS, SPACE_XXS, TEXT_MD, TEXT_SM, TEXT_XS,
};
use crate::ui::theme;

/// Unicode bidirectional controls. In a sender-chosen filename they can
/// reorder the visible text, e.g. `invoice<U+202E>fdp.app` renders as
/// `invoiceppa.pdf`.
pub(crate) fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Attachment filename as shown to the user, without bidi controls.
pub(crate) fn display_filename(name: &str) -> Cow<'_, str> {
    if name.chars().any(is_bidi_control) {
        Cow::Owned(name.chars().filter(|&ch| !is_bidi_control(ch)).collect())
    } else {
        Cow::Borrowed(name)
    }
}

/// Click targets of an attachment chip.
pub struct AttachmentChipActions<M> {
    pub on_preview: M,
    pub on_open: M,
    pub on_save: M,
    pub on_toggle_menu: M,
    pub on_close_menu: M,
}

/// Outlook-style attachment chip: icon, name and size; a click previews the
/// file and the chevron unfolds Preview / Open / Download.
pub fn attachment_chip<'a, M: 'a + Clone>(
    attachment: &'a ThreadAttachment,
    menu_open: bool,
    actions: AttachmentChipActions<M>,
) -> Element<'a, M> {
    let filename = attachment
        .filename
        .as_deref()
        .map_or_else(|| t!("compose.unnamed_attachment"), display_filename);
    let name_width = if filename.chars().count() > ATTACHMENT_CHIP_NAME_MAX_CHARS {
        Length::Fixed(ATTACHMENT_CHIP_NAME_MAX_WIDTH)
    } else {
        Length::Shrink
    };

    let body = button(
        row![
            file_type_icon(attachment.mime_type.as_deref())
                .size(ICON_MD)
                .style(text::secondary),
            text(filename)
                .size(TEXT_SM)
                .style(text::base)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::Middle)
                .width(name_width),
            text(format_file_size(attachment.size))
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(actions.on_preview.clone())
    .padding(Padding::from([0.0, SPACE_XS]))
    .height(Length::Fill)
    .style(theme::ButtonClass::Ghost.style());

    let chevron = button(
        container(
            icon::chevron_down()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        )
        .center_y(Length::Fill),
    )
    .on_press(actions.on_toggle_menu)
    .padding(Padding::from([0.0, SPACE_XXS]))
    .height(Length::Fill)
    .style(theme::ButtonClass::Ghost.style());

    let chip = container(row![body, chevron].align_y(Alignment::Center))
        .height(Length::Fixed(ATTACHMENT_CHIP_HEIGHT))
        .style(theme::ContainerClass::AttachmentChip.style());

    let mut overlay = anchored_overlay(chip).on_dismiss(actions.on_close_menu);
    if menu_open {
        let menu = container(
            iced::widget::column![
                menu_item(
                    icon::eye(),
                    t!("main.attachments.preview"),
                    actions.on_preview
                ),
                menu_item(
                    icon::external_link(),
                    t!("main.attachments.open"),
                    actions.on_open
                ),
                menu_item(
                    icon::download(),
                    t!("main.attachments.download"),
                    actions.on_save
                ),
            ]
            .spacing(SPACE_XXS),
        )
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style());
        overlay = overlay.popup(menu).popup_width(ATTACHMENT_MENU_WIDTH);
    }
    overlay.into()
}

fn menu_item<'a, M: 'a + Clone>(
    ico: iced::widget::Text<'a>,
    label: Cow<'a, str>,
    on_press: M,
) -> Element<'a, M> {
    button(
        row![
            container(ico.size(ICON_SM).style(text::secondary))
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .width(Length::Fill)
    .into()
}

fn file_type_icon<'a>(mime_type: Option<&str>) -> iced::widget::Text<'a> {
    match mime_type.unwrap_or("") {
        t if t.starts_with("image/") => icon::image(),
        t if t.contains("pdf") => icon::file_text(),
        t if t.contains("spreadsheet") || t.contains("excel") => icon::file_spreadsheet(),
        _ => icon::file(),
    }
}

fn format_file_size(size: Option<i64>) -> String {
    match size {
        None => "\u{2014}".to_string(),
        Some(b) if b < 1024 => t!("compose.size_bytes", size = b).into_owned(),
        Some(b) if b < 1024 * 1024 => {
            let size = format!("{:.0}", b as f64 / 1024.0);
            t!("compose.size_kb", size = size).into_owned()
        }
        Some(b) => {
            let size = format!("{:.1}", b as f64 / (1024.0 * 1024.0));
            t!("compose.size_mb", size = size).into_owned()
        }
    }
}
