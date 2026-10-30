use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use super::accounts::tr;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

/// Sentinel value used by the account picker to represent "Local" (no
/// account). Outside the picker the absence is `Option<String>::None`;
/// we round-trip through the select widget which only deals in `String`
/// values.
const CONTACT_LOCAL_VALUE: &str = "";

pub(super) fn contact_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.contact_editor else {
        return column![].into();
    };

    let is_new = editor.contact_id.is_none();
    let title = if is_new {
        t!("settings.contacts.new_title")
    } else {
        t!("settings.contacts.edit_title")
    };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill.max(SETTINGS_CONTENT_MAX_WIDTH));

    col = col.push(
        text(title)
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );

    let picker_open = state.open_select == Some(SelectField::ContactAccount);
    col = col.push(section_untitled(vec![contact_account_selector(
        editor,
        &state.managed_accounts,
        picker_open,
    )]));

    col = col.push(contact_editor_fields(editor));
    col = col.push(contact_editor_buttons(
        editor,
        &state.confirm_delete_contact,
    ));

    col.into()
}

fn contact_account_selector<'a>(
    editor: &'a ContactEditorState,
    accounts: &'a [ManagedAccount],
    open: bool,
) -> RowBuilder<'a> {
    let selected_value = match editor.account_id.as_deref() {
        Some(id) => id,
        None => CONTACT_LOCAL_VALUE,
    };

    let mut options: Vec<widgets::SelectOption<'a>> = Vec::with_capacity(accounts.len() + 1);
    options.push(widgets::SelectOption {
        value: CONTACT_LOCAL_VALUE.to_owned(),
        label: tr(t!("settings.contacts.local")),
        icon: None,
    });
    for account in accounts {
        let label = account
            .account_name
            .as_deref()
            .or(account.display_name.as_deref())
            .unwrap_or(&account.email);
        let icon = account
            .account_color
            .as_deref()
            .map(|hex| widgets::SelectIcon::ColorDot(theme::hex_to_color(hex)));
        options.push(widgets::SelectOption {
            value: account.id.clone(),
            label,
            icon,
        });
    }

    let dropdown = widgets::select_with_icons(
        options,
        Some(selected_value),
        open,
        true,
        tr(t!("settings.contacts.choose_account")),
        SettingsMessage::ToggleSelect(SelectField::ContactAccount),
        |value| {
            SettingsMessage::ContactEditorAccountChanged(if value == CONTACT_LOCAL_VALUE {
                None
            } else {
                Some(value)
            })
        },
    );

    setting_row_with_description(
        tr(t!("settings.contacts.account")),
        Some(tr(t!("settings.contacts.account_description"))),
        dropdown,
        SettingsMessage::ToggleSelect(SelectField::ContactAccount),
    )
}

fn contact_editor_fields(editor: &ContactEditorState) -> Element<'_, SettingsMessage> {
    let fields = vec![
        contact_field_input(
            "contact-display-name",
            &t!("settings.contacts.display_name"),
            &t!("settings.contacts.display_name_placeholder"),
            editor.display_name.text(),
            ContactField::DisplayName,
            InputField::ContactDisplayName,
        ),
        contact_field_input(
            "contact-email",
            &t!("settings.contacts.email"),
            "email@example.com",
            editor.email.text(),
            ContactField::Email,
            InputField::ContactEmail,
        ),
        contact_field_input(
            "contact-email2",
            &t!("settings.contacts.email2"),
            &t!("settings.contacts.email2_placeholder"),
            editor.email2.text(),
            ContactField::Email2,
            InputField::ContactEmail2,
        ),
        contact_field_input(
            "contact-phone",
            &t!("settings.contacts.phone"),
            &t!("settings.contacts.phone_placeholder"),
            editor.phone.text(),
            ContactField::Phone,
            InputField::ContactPhone,
        ),
        contact_field_input(
            "contact-company",
            &t!("settings.contacts.company"),
            &t!("settings.contacts.company_placeholder"),
            editor.company.text(),
            ContactField::Company,
            InputField::ContactCompany,
        ),
        contact_field_input(
            "contact-notes",
            &t!("settings.contacts.notes"),
            &t!("settings.contacts.notes_placeholder"),
            editor.notes.text(),
            ContactField::Notes,
            InputField::ContactNotes,
        ),
    ];
    section(tr(t!("settings.contacts.details")), fields)
}

fn contact_field_input(
    id: &str,
    label: &str,
    placeholder: &str,
    value: &str,
    contact_field: ContactField,
    input_field: InputField,
) -> RowBuilder<'static> {
    input_row(
        id,
        label,
        placeholder,
        value,
        move |v| SettingsMessage::ContactEditorFieldChanged(contact_field.clone(), v),
        input_field,
    )
}

fn contact_editor_buttons<'a>(
    editor: &'a ContactEditorState,
    confirm_delete: &'a Option<String>,
) -> Element<'a, SettingsMessage> {
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ref id) = editor.contact_id {
        let is_confirming = confirm_delete.as_deref() == Some(id.as_str());
        if is_confirming {
            btn_row = btn_row.push(
                button(
                    text(t!("settings.contacts.confirm_delete"))
                        .size(TEXT_LG)
                        .style(text::danger),
                )
                .on_press(SettingsMessage::ContactConfirmDelete(id.clone()))
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text(t!("settings.contacts.cancel")).size(TEXT_LG))
                    .on_press(SettingsMessage::ContactCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(
                    text(t!("settings.contacts.delete"))
                        .size(TEXT_LG)
                        .style(text::danger),
                )
                .on_press(SettingsMessage::ContactDelete(id.clone()))
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(Space::new().width(Length::Fill));

    let is_local = editor.source.as_deref().is_none_or(|s| s == "user");
    if is_local {
        if editor.contact_id.is_some() {
            btn_row = btn_row.push(
                text(t!("settings.contacts.auto_saved"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        } else {
            let can_save = !editor.email.text().trim().is_empty();
            let mut save_btn = button(
                container(text(t!("settings.contacts.create")).size(TEXT_LG))
                    .center_x(Length::Fill),
            )
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
            if can_save {
                save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
            }
            btn_row = btn_row.push(save_btn);
        }
    } else {
        let can_save = !editor.email.text().trim().is_empty() && editor.dirty;
        let mut save_btn = button(
            container(text(t!("settings.contacts.save")).size(TEXT_LG)).center_x(Length::Fill),
        )
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
        if can_save {
            save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
        }
        btn_row = btn_row.push(save_btn);
    }

    btn_row.into()
}
