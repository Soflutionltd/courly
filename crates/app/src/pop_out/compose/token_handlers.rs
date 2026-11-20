use iced::Point;

use crate::ui::token_input::{self, TokenId, TokenInputMessage, TokenInputValue};

use super::state::ComposeState;
use super::types::{
    BccNudgeBanner, BulkPasteBanner, ComposeTokenDrag, ContextMenuKind, RecipientField,
    TokenContextMenuState,
};

/// Handle a token input message for a specific recipient field.
/// Intercepts autocomplete keyboard events before delegating to the
/// generic token input handler.
pub(super) fn handle_recipient_token_input(
    state: &mut ComposeState,
    field: RecipientField,
    inner: TokenInputMessage,
) {
    match &inner {
        TokenInputMessage::AutocompleteDown => {
            let len = state.autocomplete.results.len();
            if len > 0 {
                let next = state
                    .autocomplete
                    .highlighted
                    .map_or(0, |h| (h + 1).min(len - 1));
                state.autocomplete.highlighted = Some(next);
            }
            return;
        }
        TokenInputMessage::AutocompleteUp => {
            if let Some(h) = state.autocomplete.highlighted {
                state.autocomplete.highlighted = Some(h.saturating_sub(1));
            }
            return;
        }
        TokenInputMessage::AutocompleteAccept => {
            let idx = state.autocomplete.highlighted.unwrap_or(0);
            if let Some(match_entry) = state.autocomplete.results.get(idx).cloned() {
                let label = match_entry
                    .display_name
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&match_entry.email)
                    .to_string();
                let target = match state.autocomplete.active_field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                let id = target.next_token_id();
                let is_group = match_entry.is_group;
                let token_label = label.clone();
                target.tokens.push(token_input::Token {
                    id,
                    email: match_entry.email,
                    label,
                    is_group,
                    group_id: match_entry.group_id,
                    member_count: match_entry.member_count,
                });
                target.text.clear();
                state.autocomplete.results.clear();
                state.autocomplete.highlighted = None;
                state.autocomplete.query.clear();
                state.draft_dirty = true;

                let active = state.autocomplete.active_field;
                if is_group && (active == RecipientField::To || active == RecipientField::Cc) {
                    state.bcc_nudges.push(BccNudgeBanner {
                        group_name: token_label,
                        token_id: id,
                        source_field: active,
                    });
                }
            }
            return;
        }
        TokenInputMessage::AutocompleteDismissKey => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
            return;
        }
        TokenInputMessage::DragStarted(token_id) => {
            let tid = *token_id;
            let label = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .find(|t| t.id == tid)
            .map(|t| t.label.clone())
            .unwrap_or_default();
            state.drag = Some(ComposeTokenDrag {
                token_id: tid,
                source_field: field,
                label,
                current_position: Point::ORIGIN,
            });
            return;
        }
        TokenInputMessage::DragReleased => {
            drop_dragged_token(state);
            return;
        }
        TokenInputMessage::TokenContextMenu(token_id, position) => {
            let tid = *token_id;
            let pos = *position;
            let is_group = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .any(|t| t.id == tid && t.is_group);

            state.context_menu = Some(TokenContextMenuState {
                field,
                position: pos,
                kind: ContextMenuKind::Token {
                    token_id: tid,
                    is_group,
                },
            });
            return;
        }
        TokenInputMessage::FieldContextMenu(position) => {
            state.context_menu = Some(TokenContextMenuState {
                field,
                position: *position,
                kind: ContextMenuKind::Field,
            });
            return;
        }
        _ => {}
    }

    if let TokenInputMessage::TextChanged(ref t) = inner {
        state.autocomplete.query = t.clone();
        state.autocomplete.active_field = field;
    }

    let is_paste = matches!(&inner, TokenInputMessage::Paste(_));
    let tokens_before = if is_paste {
        match field {
            RecipientField::To => state.to.tokens.len(),
            RecipientField::Cc => state.cc.tokens.len(),
            RecipientField::Bcc => state.bcc.tokens.len(),
        }
    } else {
        0
    };

    match field {
        RecipientField::To => {
            state.selected_cc_token = None;
            state.selected_bcc_token = None;
        }
        RecipientField::Cc => {
            state.selected_to_token = None;
            state.selected_bcc_token = None;
        }
        RecipientField::Bcc => {
            state.selected_to_token = None;
            state.selected_cc_token = None;
        }
    }
    let (value, selected) = match field {
        RecipientField::To => (&mut state.to, &mut state.selected_to_token),
        RecipientField::Cc => (&mut state.cc, &mut state.selected_cc_token),
        RecipientField::Bcc => (&mut state.bcc, &mut state.selected_bcc_token),
    };
    handle_token_input_message(value, inner, selected);
    state.draft_dirty = true;

    if is_paste {
        let field_tokens = match field {
            RecipientField::To => &state.to.tokens,
            RecipientField::Cc => &state.cc.tokens,
            RecipientField::Bcc => &state.bcc.tokens,
        };
        let tokens_after = field_tokens.len();
        let added = tokens_after.saturating_sub(tokens_before);
        if added >= 10 {
            let token_ids = field_tokens
                .iter()
                .skip(tokens_before)
                .map(|t| t.id)
                .collect();
            state.bulk_paste_banner = Some(BulkPasteBanner {
                count: added,
                field,
                token_ids,
            });
        }
    }
}

fn recipients_mut(state: &mut ComposeState, field: RecipientField) -> &mut TokenInputValue {
    match field {
        RecipientField::To => &mut state.to,
        RecipientField::Cc => &mut state.cc,
        RecipientField::Bcc => &mut state.bcc,
    }
}

/// End a drag: the bubble moves to the recipient row under the pointer. A
/// release on its own row, or outside every row, leaves it where it was.
fn drop_dragged_token(state: &mut ComposeState) {
    let Some(drag) = state.drag.take() else {
        return;
    };
    let Some(target) = state
        .drop_target
        .filter(|target| *target != drag.source_field)
    else {
        return;
    };
    let source = recipients_mut(state, drag.source_field);
    let Some(index) = source.tokens.iter().position(|t| t.id == drag.token_id) else {
        return;
    };
    let mut token = source.tokens.remove(index);
    let destination = recipients_mut(state, target);
    token.id = destination.next_token_id();
    destination.tokens.push(token);

    match target {
        RecipientField::To => {}
        RecipientField::Cc => state.show_cc = true,
        RecipientField::Bcc => state.show_bcc = true,
    }
    state.selected_to_token = None;
    state.selected_cc_token = None;
    state.selected_bcc_token = None;
    // The nudge offered to move this bubble; it has moved.
    state
        .bcc_nudges
        .retain(|nudge| nudge.token_id != drag.token_id);
    state.draft_dirty = true;
}

fn handle_token_input_message(
    value: &mut TokenInputValue,
    msg: TokenInputMessage,
    selected: &mut Option<TokenId>,
) {
    match msg {
        TokenInputMessage::TextChanged(text) => value.text = text,
        TokenInputMessage::RemoveToken(id) => {
            value.tokens.retain(|t| t.id != id);
            *selected = None;
        }
        TokenInputMessage::TokenizeText(text) => validate_text(value, text),
        TokenInputMessage::SelectToken(id) => *selected = Some(id),
        TokenInputMessage::EditToken(id) => edit_token(value, id, selected),
        TokenInputMessage::DeselectTokens => *selected = None,
        TokenInputMessage::BackspaceAtStart => {
            if let Some(last) = value.tokens.last() {
                *selected = Some(last.id);
            }
        }
        TokenInputMessage::Focused => {}
        // Leaving the field validates what was typed, as Enter does, so an
        // address being corrected goes back to a bubble. Text that is not
        // an address stays as typed.
        TokenInputMessage::Blurred => {
            let typed = value.text.clone();
            validate_text(value, typed);
        }
        TokenInputMessage::TokenContextMenu(_, _) | TokenInputMessage::FieldContextMenu(_) => {
            // Handled at the compose level via handle_recipient_token_input
        }
        TokenInputMessage::CopyToken(_) | TokenInputMessage::CutToken(_) => {
            // Handled at the app layer (handlers/pop_out.rs) which translates
            // these to ContextMenuCopy/Cut so the same async DB-expand-then-
            // clipboard-write logic runs whether the trigger is a key chord
            // or the right-click menu.
        }
        TokenInputMessage::ArrowSelectToken(_) => {}
        TokenInputMessage::ArrowToText => {}
        TokenInputMessage::AutocompleteDown
        | TokenInputMessage::AutocompleteUp
        | TokenInputMessage::AutocompleteAccept
        | TokenInputMessage::AutocompleteDismissKey
        | TokenInputMessage::DragStarted(_)
        | TokenInputMessage::DragReleased => {}
        TokenInputMessage::Paste(content) => {
            let parsed = import::parse_recipient_paste(
                &import::RecipientPastePayload::from_plain_text(content),
            );
            let mut recipients = parsed.recipients;
            let existing: std::collections::HashSet<String> = value
                .tokens
                .iter()
                .map(|t| t.email.to_lowercase())
                .collect();
            recipients.retain(|addr| !existing.contains(&addr.email.to_lowercase()));
            if push_parsed_recipients(value, recipients) > 0 {
                value.text.clear();
            }
        }
    }
}

/// Turn `text` into recipient bubbles and clear the text being typed. Text
/// that holds no address is left as it is.
fn validate_text(value: &mut TokenInputValue, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let parsed =
        import::parse_recipient_paste(&import::RecipientPastePayload::from_plain_text(text));
    if push_parsed_recipients(value, parsed.recipients) > 0 {
        value.text.clear();
    }
}

/// Turn the recipient `id` back into the text being typed, so it can be
/// corrected and validated again with Enter. Text already typed is
/// validated first so it is not lost; a group stays a bubble.
fn edit_token(value: &mut TokenInputValue, id: TokenId, selected: &mut Option<TokenId>) {
    let Some(index) = value
        .tokens
        .iter()
        .position(|token| token.id == id && !token.is_group)
    else {
        return;
    };
    let token = value.tokens.remove(index);
    if !value.text.trim().is_empty() {
        let parsed = import::parse_recipient_paste(
            &import::RecipientPastePayload::from_plain_text(std::mem::take(&mut value.text)),
        );
        push_parsed_recipients(value, parsed.recipients);
    }
    value.text = token.email;
    *selected = None;
}

fn push_parsed_recipients(
    value: &mut TokenInputValue,
    parsed: Vec<import::ParsedRecipient>,
) -> usize {
    let mut added = 0usize;
    for addr in parsed {
        let label = addr
            .display_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(&addr.email)
            .to_string();
        let id = value.next_token_id();
        value.tokens.push(token_input::Token {
            id,
            email: addr.email,
            label,
            is_group: false,
            group_id: None,
            member_count: None,
        });
        added += 1;
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(addresses: &[&str]) -> TokenInputValue {
        let mut value = TokenInputValue::new();
        let parsed = import::parse_recipient_list(&addresses.join(", "));
        push_parsed_recipients(&mut value, parsed);
        value
    }

    #[test]
    fn a_double_clicked_bubble_becomes_the_text_being_typed() {
        let mut value = field(&["alice@example.com", "bob@example.com"]);
        let bob = value.tokens[1].id;
        let mut selected = Some(bob);

        edit_token(&mut value, bob, &mut selected);

        assert_eq!(value.text, "bob@example.com");
        assert_eq!(value.tokens.len(), 1);
        assert_eq!(value.tokens[0].email, "alice@example.com");
        assert_eq!(selected, None);
    }

    #[test]
    fn text_already_typed_is_validated_before_a_bubble_is_edited() {
        let mut value = field(&["alice@example.com"]);
        value.text = "carol@example.com".to_string();
        let alice = value.tokens[0].id;
        let mut selected = None;

        edit_token(&mut value, alice, &mut selected);

        assert_eq!(value.text, "alice@example.com");
        assert_eq!(value.tokens.len(), 1);
        assert_eq!(value.tokens[0].email, "carol@example.com");
    }

    #[test]
    fn leaving_the_field_turns_a_corrected_address_back_into_a_bubble() {
        let mut value = field(&["alice@example.com", "bob@exmaple.com"]);
        let bob = value.tokens[1].id;
        let mut selected = Some(bob);
        edit_token(&mut value, bob, &mut selected);
        value.text = "bob@example.com".to_string();

        handle_token_input_message(&mut value, TokenInputMessage::Blurred, &mut selected);

        assert_eq!(value.text, "");
        let emails: Vec<&str> = value.tokens.iter().map(|t| t.email.as_str()).collect();
        assert_eq!(emails, ["alice@example.com", "bob@example.com"]);
    }

    /// A compose with two recipients in To, the second one being dragged.
    fn dragging_bob() -> ComposeState {
        let mut state = ComposeState::new(&[]);
        state.to = field(&["alice@example.com", "bob@example.com"]);
        let bob = state.to.tokens[1].id;
        state.drag = Some(ComposeTokenDrag {
            token_id: bob,
            source_field: RecipientField::To,
            label: "bob@example.com".to_string(),
            current_position: Point::ORIGIN,
        });
        state
    }

    fn emails(value: &TokenInputValue) -> Vec<&str> {
        value.tokens.iter().map(|t| t.email.as_str()).collect()
    }

    #[test]
    fn a_bubble_dropped_on_another_row_moves_there() {
        let mut state = dragging_bob();
        state.drop_target = Some(RecipientField::Bcc);

        handle_recipient_token_input(
            &mut state,
            RecipientField::To,
            TokenInputMessage::DragReleased,
        );

        assert_eq!(emails(&state.to), ["alice@example.com"]);
        assert_eq!(emails(&state.bcc), ["bob@example.com"]);
        assert!(state.show_bcc, "the row it landed on stays visible");
        assert!(state.drag.is_none());
    }

    #[test]
    fn a_bubble_released_on_its_own_row_or_nowhere_stays_put() {
        for target in [Some(RecipientField::To), None] {
            let mut state = dragging_bob();
            state.drop_target = target;

            handle_recipient_token_input(
                &mut state,
                RecipientField::To,
                TokenInputMessage::DragReleased,
            );

            assert_eq!(emails(&state.to), ["alice@example.com", "bob@example.com"]);
            assert!(state.drag.is_none());
        }
    }

    #[test]
    fn the_row_left_does_not_clear_the_row_just_entered() {
        use crate::pop_out::compose::{ComposeMessage, update_compose};

        let mut state = dragging_bob();
        update_compose(
            &mut state,
            ComposeMessage::RecipientRowEntered(RecipientField::Cc),
        );
        update_compose(
            &mut state,
            ComposeMessage::RecipientRowExited(RecipientField::To),
        );
        assert_eq!(state.drop_target, Some(RecipientField::Cc));
    }

    #[test]
    fn leaving_the_field_keeps_text_that_is_not_an_address() {
        let mut value = field(&["alice@example.com"]);
        value.text = "bo".to_string();
        let mut selected = None;

        handle_token_input_message(&mut value, TokenInputMessage::Blurred, &mut selected);

        assert_eq!(value.text, "bo");
        assert_eq!(value.tokens.len(), 1);
    }
}
