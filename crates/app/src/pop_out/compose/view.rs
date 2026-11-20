use iced::widget::{Space, button, column, container, mouse_area, row, text, text_input};
use iced::{Alignment, Element, Length};
use rte::rich_text_editor;
use rust_i18n::t;

use crate::Message;
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::token_input::{self, TokenId, TokenInputMessage, TokenInputValue};
use crate::ui::widgets;
use crate::ui::widgets::Tool;

use crate::pop_out::PopOutMessage;

use super::messages::ComposeMessage;
use super::modals::{
    attachment_list, autocomplete_dropdown, bcc_nudge_banner, bulk_paste_banner_view,
    discard_confirmation, link_dialog, save_as_group_dialog, token_context_menu,
};
use super::quote::{ComposePreview, HtmlBlock};
use super::state::ComposeState;
use super::types::RecipientField;

/// A compose window. `answered_attachments` are the files of the message
/// being answered, when the conversation holding it is at hand: they show
/// on its card under the reply, as they do in the conversation.
pub fn view_compose_window<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
    answered_attachments: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let header = compose_header(window_id, state);
    let toolbar = formatting_toolbar(window_id, state);
    let body = compose_body(window_id, state, answered_attachments);
    let footer = compose_footer(window_id, state);

    let mut content = column![header,].spacing(SPACE_0);

    for nudge in &state.bcc_nudges {
        content = content.push(bcc_nudge_banner(window_id, nudge));
    }

    if let Some(ref banner) = state.bulk_paste_banner {
        content = content.push(bulk_paste_banner_view(window_id, banner));
    }

    content = content
        .push(crate::ui::widgets::divider())
        .push(toolbar)
        .push(crate::ui::widgets::divider())
        .push(body);

    if !state.attachments.is_empty() {
        content = content.push(crate::ui::widgets::divider());
        content = content.push(attachment_list(window_id, state));
    }

    content = content.push(crate::ui::widgets::divider());
    content = content.push(footer);

    let base = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style());

    let with_context_menu: Element<'a, Message> = if let Some(ref ctx) = state.context_menu {
        crate::ui::anchored_overlay::anchored_overlay(base)
            .popup(token_context_menu(window_id, ctx))
            .popup_width(180.0)
            .anchor_point(ctx.position)
            .on_dismiss(Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::DismissContextMenu),
            ))
            .into()
    } else {
        base.into()
    };

    if state.discard_confirm_open {
        let noop = Message::PopOut(window_id, PopOutMessage::Compose(ComposeMessage::Noop));
        crate::ui::modal_overlay::modal_overlay(
            with_context_menu,
            discard_confirmation(window_id),
            crate::ui::modal_overlay::ModalSurface::Modal,
            noop,
        )
    } else if state.link_dialog_open {
        let noop = Message::PopOut(window_id, PopOutMessage::Compose(ComposeMessage::Noop));
        crate::ui::modal_overlay::modal_overlay(
            with_context_menu,
            link_dialog(window_id, state),
            crate::ui::modal_overlay::ModalSurface::Modal,
            noop,
        )
    } else if state.save_group_dialog_open {
        let noop = Message::PopOut(window_id, PopOutMessage::Compose(ComposeMessage::Noop));
        crate::ui::modal_overlay::modal_overlay(
            with_context_menu,
            save_as_group_dialog(window_id, state),
            crate::ui::modal_overlay::ModalSurface::Modal,
            noop,
        )
    } else {
        with_context_menu
    }
}

fn compose_header<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    // No boxes: each field sits on the header itself and a hairline closes
    // its row, as in Apple Mail and Outlook.
    let field_row = |row: Element<'a, Message>| container(row).padding(PAD_COMPOSE_FIELD_ROW);
    // A recipient row reports the pointer entering and leaving it, so a
    // dragged bubble knows where it lands, and lights up as the landing row.
    let dragged_from = state.drag.as_ref().map(|drag| drag.source_field);
    let recipient_row = |field: RecipientField, row: Element<'a, Message>| {
        let compose = move |message| Message::PopOut(window_id, PopOutMessage::Compose(message));
        let lands_here =
            dragged_from.is_some_and(|from| from != field) && state.drop_target == Some(field);
        let mut slot = container(row)
            .padding(PAD_COMPOSE_FIELD_ROW)
            .width(Length::Fill);
        if lands_here {
            slot = slot.style(theme::ContainerClass::DropTarget.style());
        }
        mouse_area(slot)
            .on_enter(compose(ComposeMessage::RecipientRowEntered(field)))
            .on_exit(compose(ComposeMessage::RecipientRowExited(field)))
    };
    let mut fields = column![].spacing(SPACE_0);

    fields = fields
        .push(field_row(build_from_row(window_id, state)))
        .push(widgets::divider())
        .push(recipient_row(
            RecipientField::To,
            build_to_row(window_id, state),
        ));

    if let Some(err) = state.recipients_error.as_deref() {
        // Inline validation error directly under the recipient rows.
        // Indented to line up with the field column (skipping the label
        // gutter) so it visually belongs to the To/Cc/Bcc cluster
        // rather than floating loose at the bottom of the form.
        fields = fields.push(
            row![
                Space::new().width(COMPOSE_LABEL_WIDTH),
                text(err).size(TEXT_SM).style(text::danger),
            ]
            .spacing(SPACE_XS),
        );
    }
    fields = fields.push(widgets::divider());

    // While a bubble is dragged both rows show, so it can be dropped on a
    // row that was hidden.
    if state.show_cc || dragged_from.is_some() {
        fields = fields
            .push(recipient_row(
                RecipientField::Cc,
                build_cc_row(window_id, state),
            ))
            .push(widgets::divider());
    }

    if state.show_bcc || dragged_from.is_some() {
        fields = fields
            .push(recipient_row(
                RecipientField::Bcc,
                build_bcc_row(window_id, state),
            ))
            .push(widgets::divider());
    }

    let subject_input = text_input(t!("compose.subject"), &state.subject)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::SubjectChanged(s)),
            )
        })
        .size(TEXT_LG)
        .padding(PAD_INPUT)
        .style(theme::TextInputClass::Inline.style());

    let subject_row = row![
        container(
            text(t!("compose.subject"))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_y(Alignment::Center),
        // The arrow stands for the reply or forward mark, which is sent
        // but not shown.
        widgets::subject_mark(state.subject_kind, ICON_SM),
        subject_input,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    // The rule under the header closes the subject row.
    fields = fields.push(field_row(subject_row.into()));

    if let Some(ref status) = state.status {
        fields = fields.push(widgets::divider()).push(
            container(
                text(status.as_str())
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .padding(PAD_INPUT),
        );
    }

    container(fields)
        .padding(PAD_COMPOSE_HEADER)
        .width(Length::Fill)
        .into()
}

fn build_from_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let from_picker = from_account_picker(window_id, state);

    let from_label = container(
        text(t!("compose.from"))
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .width(COMPOSE_LABEL_WIDTH)
    .align_y(Alignment::Center);

    let mut from_row = row![from_label, from_picker]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if !state.show_cc {
        from_row = from_row.push(
            button(text(t!("compose.cc")).size(TEXT_SM))
                .style(theme::ButtonClass::BareIcon.style())
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ShowCc),
                ))
                .padding(PAD_INPUT),
        );
    }
    if !state.show_bcc {
        from_row = from_row.push(
            button(text(t!("compose.bcc")).size(TEXT_SM))
                .style(theme::ButtonClass::BareIcon.style())
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ShowBcc),
                ))
                .padding(PAD_INPUT),
        );
    }

    from_row.into()
}

/// Build the From-account dropdown trigger plus its popover menu.
///
/// Trigger layout: `[ name <email> .... ]  [ account_name ]  [ chevron ]`
/// - name+email is `Length::Fill`, ellipsizes when narrow
/// - account_name is `Length::Shrink`, rendered in a tertiary (muted) color
/// - chevron is fixed
fn from_account_picker<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let main_text = match state.from_account.as_ref() {
        Some(a) => match a.display_name.as_deref() {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
            _ => a.email.clone(),
        },
        None => t!("compose.select_account").into_owned(),
    };
    let account_name = state
        .from_account
        .as_ref()
        .and_then(|a| a.account_name.clone())
        .unwrap_or_default();

    let trigger_row = row![
        container(
            text(main_text)
                .size(TEXT_LG)
                .style(text::base)
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
        container(
            text(account_name)
                .size(TEXT_LG)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            icon::chevron_down()
                .size(ICON_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let trigger = button(trigger_row)
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::ToggleFromDropdown),
        ))
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(theme::ButtonClass::BareIcon.style());

    if !state.from_dropdown_open {
        return trigger.into();
    }

    let selected_id = state.from_account.as_ref().map(|a| a.id.clone());
    let mut items = column![].spacing(SPACE_XXS).width(Length::Fill);
    for account in &state.from_accounts {
        let item_main = match account.display_name.as_deref() {
            Some(n) if !n.is_empty() => format!("{n} <{}>", account.email),
            _ => account.email.clone(),
        };
        let item_meta = account.account_name.clone().unwrap_or_default();
        let is_selected = selected_id.as_deref() == Some(account.id.as_str());
        let acc = account.clone();
        let item = button(
            row![
                container(
                    text(item_main)
                        .size(TEXT_LG)
                        .style(text::base)
                        .wrapping(iced::widget::text::Wrapping::None)
                        .ellipsis(iced::widget::text::Ellipsis::End)
                )
                .width(Length::Fill)
                .align_y(Alignment::Center),
                container(
                    text(item_meta)
                        .size(TEXT_LG)
                        .style(theme::TextClass::Tertiary.style())
                )
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::FromAccountChanged(acc)),
        ))
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(
            theme::ButtonClass::Dropdown {
                selected: is_selected,
            }
            .style(),
        );
        items = items.push(item);
    }

    let menu = container(items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .width(Length::Fill);

    crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .on_dismiss(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::ToggleFromDropdown),
        ))
        .into()
}

fn build_to_row<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::To
        && !state.autocomplete.results.is_empty();
    let autocomplete_dropdown = ac_open.then(|| autocomplete_dropdown(window_id, state));
    build_recipient_row_inner(
        t!("compose.to").into_owned(),
        &state.to,
        state.selected_to_token,
        ac_open,
        autocomplete_dropdown,
        window_id,
        t!("compose.add_recipients").into_owned(),
        ComposeMessage::ToTokenInput,
    )
}

fn build_cc_row<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::Cc
        && !state.autocomplete.results.is_empty();
    let autocomplete_dropdown = ac_open.then(|| autocomplete_dropdown(window_id, state));
    build_recipient_row_inner(
        t!("compose.cc").into_owned(),
        &state.cc,
        state.selected_cc_token,
        ac_open,
        autocomplete_dropdown,
        window_id,
        t!("compose.add_cc").into_owned(),
        ComposeMessage::CcTokenInput,
    )
}

fn build_bcc_row<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::Bcc
        && !state.autocomplete.results.is_empty();
    let autocomplete_dropdown = ac_open.then(|| autocomplete_dropdown(window_id, state));
    build_recipient_row_inner(
        t!("compose.bcc").into_owned(),
        &state.bcc,
        state.selected_bcc_token,
        ac_open,
        autocomplete_dropdown,
        window_id,
        t!("compose.add_bcc").into_owned(),
        ComposeMessage::BccTokenInput,
    )
}

// TODO(refactor): bundle row params (autocomplete + selection state) into a struct.
#[allow(clippy::too_many_arguments)]
fn build_recipient_row_inner<'a>(
    label: String,
    value: &'a TokenInputValue,
    selected: Option<TokenId>,
    autocomplete_open: bool,
    autocomplete_dropdown: Option<Element<'a, Message>>,
    window_id: iced::window::Id,
    placeholder: String,
    wrap: fn(TokenInputMessage) -> ComposeMessage,
) -> Element<'a, Message> {
    let field = token_input::token_input_field(
        &value.tokens,
        &value.text,
        placeholder,
        selected,
        autocomplete_open,
        move |msg| Message::PopOut(window_id, PopOutMessage::Compose(wrap(msg))),
    );

    let mut overlay =
        crate::ui::anchored_overlay::anchored_overlay(field).on_dismiss(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::AutocompleteDismiss),
        ));
    if let Some(dropdown) = autocomplete_dropdown {
        overlay = overlay.popup(dropdown);
    }
    let field: Element<'a, Message> = overlay.into();

    row![
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_y(Alignment::Center),
        field,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn formatting_toolbar<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let compose = move |message| Message::PopOut(window_id, PopOutMessage::Compose(message));
    let bar = crate::ui::widgets::format_bar(
        &state.format_bar,
        crate::ui::widgets::CaretContext::of(&state.body),
        |menu| compose(ComposeMessage::ToggleFormatMenu(menu)),
        move |query| compose(ComposeMessage::FormatQuery(query)),
        |action| compose(ComposeMessage::FormatEdit(action)),
    );
    let tool = |glyph: fn() -> iced::widget::Text<'a>, label, message| {
        Tool::icon(glyph, label, compose(message))
    };

    // Most used first: the bar folds the last ones into its "more" panel
    // when the window is too narrow, so it always holds on one line.
    let tools = vec![
        bar.font,
        bar.size,
        tool(
            icon::bold,
            t!("compose.format.bold"),
            ComposeMessage::FormatBold,
        ),
        tool(
            icon::italic,
            t!("compose.format.italic"),
            ComposeMessage::FormatItalic,
        ),
        tool(
            icon::underline,
            t!("compose.format.underline"),
            ComposeMessage::FormatUnderline,
        ),
        bar.color,
        bar.highlight,
        tool(
            icon::list,
            t!("compose.format.bulleted_list"),
            ComposeMessage::FormatList,
        ),
        tool(
            icon::list_ordered,
            t!("compose.format.numbered_list"),
            ComposeMessage::FormatOrderedList,
        ),
        bar.alignment,
        tool(
            icon::link,
            t!("compose.format.link"),
            ComposeMessage::FormatLink,
        ),
        tool(
            icon::image,
            t!("compose.format.image"),
            ComposeMessage::InsertImage,
        ),
        emoji_tool(window_id, state),
        bar.block,
        tool(
            icon::strikethrough,
            t!("compose.format.strikethrough"),
            ComposeMessage::FormatStrikethrough,
        ),
        bar.line_height,
        bar.outdent,
        bar.indent,
        bar.smaller,
        bar.larger,
        bar.table,
        bar.clear,
    ];

    container(crate::ui::widgets::toolbar(
        tools,
        &state.format_bar,
        |menu| compose(ComposeMessage::ToggleFormatMenu(menu)),
    ))
    .padding(PAD_TOOLBAR)
    .width(Length::Fill)
    .into()
}

/// The emoji-picker tool, with the picker dropped beneath it when open.
fn emoji_tool<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Tool<'a, Message> {
    let toggle = Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::ToggleEmojiPicker),
    );
    let tool = Tool::icon(icon::smile, t!("compose.format.emoji"), toggle.clone());
    if !state.emoji_picker_open {
        return tool;
    }

    let picker = crate::ui::widgets::emoji_picker(
        &state.emoji_picker_query,
        state.emoji_picker_category,
        move |emoji| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::EmojiPickerSelected(emoji.to_string())),
            )
        },
        move |cat| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::EmojiPickerCategoryChanged(cat)),
            )
        },
        move |q| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::EmojiPickerSearchChanged(q)),
            )
        },
    );

    tool.with_menu(picker, None, toggle)
}

fn compose_body<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
    answered_attachments: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let has_extras = state.signature.is_some() || state.quote.is_some();
    // With a signature or a quote under it, the editor is as tall as its
    // text so the page around both scrolls as one, the way Outlook does.
    let editor_height = if has_extras {
        Length::Shrink.min(COMPOSE_EDITOR_MIN_HEIGHT)
    } else {
        Length::Fill
    };
    let editor = rich_text_editor(&state.body)
        .on_action(move |action| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::BodyChanged(action)),
            )
        })
        .height(editor_height)
        .font(font::text());

    // The page card stays put and the text scrolls inside it: a scrolled
    // card would get its rounded corners cut square by the viewport. The
    // inset keeps the scrolled content inside the rounding.
    let page_card = |page: Element<'a, Message>| -> Element<'a, Message> {
        container(
            container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(SPACE_XXS)
                .style(theme::ContainerClass::EmailBody.style()),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: SPACE_XS,
            right: SPACE_SM,
            bottom: SPACE_XS,
            left: SPACE_SM,
        })
        .into()
    };

    if !has_extras {
        return page_card(container(editor).padding(PAD_CONTENT).into());
    }

    let compose = move |msg| Message::PopOut(window_id, PopOutMessage::Compose(msg));
    let page = theme::ContainerClass::EmailBody;

    // The reply is written in a card of its own, with its signature. Keyed
    // so a signature arriving under the editor gets its own sensor.
    let mut reply = iced::widget::keyed_column(Vec::new()).spacing(SPACE_SM);
    reply = reply.push(None, Element::from(editor));
    if let Some(signature) = state.signature.as_ref() {
        let which = ComposePreview::Signature;
        reply = reply.push(
            Some(which),
            crate::ui::mail_body::view(
                Some(&signature.preview),
                Some(signature.text.as_str()),
                move |width| compose(ComposeMessage::PreviewResized(which, width)),
                move |url| compose(ComposeMessage::PreviewLinkClicked(url)),
                compose(ComposeMessage::PreviewShowRemote(which)),
            ),
        );
    }
    let mut cards = column![widgets::message_card_frame(
        container(container(reply).padding(PAD_HTML_BODY))
            .width(Length::Fill)
            .style(page.style())
            .into(),
    )]
    .spacing(SPACE_XS);

    // The message answered, and each one it quotes, in cards of their own
    // under the reply, as the conversation shows them. They are sent with
    // the reply as they were received: what was said cannot be rewritten.
    if let Some(quote) = state.quote.as_ref() {
        cards = quote_cards(cards, quote, page, compose, answered_attachments);
    }

    iced::widget::scrollable(cards.padding(iced::Padding {
        top: SPACE_XS,
        right: SPACE_SM,
        bottom: SPACE_XS,
        left: SPACE_SM,
    }))
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// The quoted original of a reply as cards pushed under `cards`: the
/// message answered, then the earlier ones it quotes.
fn quote_cards<'a>(
    mut cards: iced::widget::Column<'a, Message>,
    quote: &'a HtmlBlock,
    page: theme::ContainerClass,
    compose: impl Fn(ComposeMessage) -> Message + Copy + 'a,
    answered_attachments: Option<Element<'a, Message>>,
) -> iced::widget::Column<'a, Message> {
    use widgets::QuotedSender;

    let which = ComposePreview::Quote;
    // Read-only, but read like any message: its text can be selected and
    // copied, and its links have their menu.
    let quoted = crate::ui::mail_body::view_with_link_menu(
        Some(&quote.preview),
        Some(quote.text.as_str()),
        &[],
        move |width| compose(ComposeMessage::PreviewResized(which, width)),
        move |action| compose(ComposeMessage::PreviewLink(action)),
        move |action| compose(ComposeMessage::PreviewSelect(action)),
        compose(ComposeMessage::PreviewShowRemote(which)),
    );
    cards = cards.push(widgets::quoted_message_card(
        quoted.message_header.map(QuotedSender::from_header),
        answered_attachments,
        quoted.message,
        page,
        None,
    ));
    for earlier in quoted.quoted {
        cards = cards.push(widgets::quoted_message_card(
            earlier.header.map(QuotedSender::from_header),
            None,
            earlier.body,
            page,
            None,
        ));
    }
    cards
}

fn compose_footer<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let discard_msg = if state.has_user_content() {
        ComposeMessage::ToggleDiscardConfirm
    } else {
        ComposeMessage::Discard
    };

    let discard_btn = button(
        row![
            icon::trash().size(ICON_LG),
            text(t!("compose.discard")).size(TEXT_LG),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Action.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(discard_msg),
    ))
    .padding(PAD_BUTTON);

    let send_btn = button(
        row![
            icon::send().size(ICON_LG).color(theme::ON_AVATAR),
            text(t!("compose.send"))
                .size(TEXT_LG)
                .color(theme::ON_AVATAR),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Primary.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::Send),
    ))
    .padding(PAD_BUTTON);

    let attach_btn = button(
        row![
            icon::paperclip().size(ICON_LG),
            text(t!("compose.attach")).size(TEXT_LG),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Action.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::AttachFiles),
    ))
    .padding(PAD_BUTTON);

    let save_btn = button(
        row![
            icon::download().size(ICON_LG),
            text(t!("compose.save")).size(TEXT_LG),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Action.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::SaveDraftNow),
    ))
    .padding(PAD_BUTTON);

    let receipt_btn = button(
        row![
            icon::eye().size(ICON_LG),
            text(t!("compose.read_receipt")).size(TEXT_LG),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(
        theme::ButtonClass::Chip {
            active: state.request_read_receipt,
        }
        .style(),
    )
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::ToggleReadReceipt),
    ))
    .padding(PAD_BUTTON);

    let mut footer_row =
        row![discard_btn, attach_btn, save_btn, receipt_btn].align_y(Alignment::Center);
    if state.inline {
        footer_row = footer_row.push(
            button(
                row![
                    icon::external_link().size(ICON_LG),
                    text(t!("compose.pop_out")).size(TEXT_LG)
                ]
                .spacing(SPACE_XXS)
                .align_y(Alignment::Center),
            )
            .style(theme::ButtonClass::Action.style())
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::PopOut),
            ))
            .padding(PAD_BUTTON),
        );
    }
    let footer_row = footer_row
        .push(Space::new().width(Length::Fill))
        .push(send_btn);

    container(footer_row)
        .padding(iced::Padding {
            top: PAD_CONTENT.top,
            right: PAD_CONTENT.right,
            bottom: PAD_CONTENT.bottom,
            left: PAD_CONTENT.left - PAD_BUTTON.left,
        })
        .width(Length::Fill)
        .into()
}
