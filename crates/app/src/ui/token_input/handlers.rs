use iced::Rectangle;
use iced::advanced::Shell;
use iced::advanced::clipboard;
use iced::keyboard;
use iced::mouse;

use super::layout::{selected_index, text_area_origin};
use super::render::measure_text_width;
use super::types::TokenInputMessage;
use super::widget::{TokenInputState, TokenInputWidget};

/// Byte offset of the caret in the text being typed. The app owns that text
/// and can replace it at any time (a bubble turned back into text, an
/// address validated into a bubble), so a caret remembered for a text of
/// another length falls back to the end.
pub(super) fn caret_offset(state: &TokenInputState, text: &str) -> usize {
    match state.caret {
        Some((offset, length)) if length == text.len() && text.is_char_boundary(offset) => offset,
        _ => text.len(),
    }
}

fn previous_boundary(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, offset: usize) -> usize {
    text[offset..]
        .chars()
        .next()
        .map_or(offset, |character| offset + character.len_utf8())
}

/// The caret position nearest to `x`, measured from the start of the text.
fn caret_at(text: &str, x: f32) -> usize {
    let mut nearest = (f32::INFINITY, 0);
    let mut offset = 0;
    loop {
        let distance = (measure_text_width(&text[..offset]) - x).abs();
        if distance < nearest.0 {
            nearest = (distance, offset);
        }
        if offset == text.len() {
            return nearest.1;
        }
        offset = next_boundary(text, offset);
    }
}

/// Publish the text with `replacement` in place of `range`, and leave the
/// caret after it.
fn replace_text<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    range: std::ops::Range<usize>,
    replacement: &str,
    shell: &mut Shell<'_, M>,
) {
    let mut text = widget.text.to_string();
    let caret = range.start + replacement.len();
    text.replace_range(range, replacement);
    state.caret = Some((caret, text.len()));
    shell.publish((widget.on_message)(TokenInputMessage::TextChanged(text)));
}

pub(super) fn handle_left_click<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    cursor: mouse::Cursor,
    bounds: Rectangle,
    shell: &mut Shell<'_, M>,
) {
    let Some(pos) = cursor.position() else {
        return;
    };

    if bounds.contains(pos) {
        // Hit-test tokens
        for (i, token) in widget.tokens.iter().enumerate() {
            if let Some(chip) = state.token_bounds.get(i) {
                let abs = Rectangle {
                    x: bounds.x + chip.x,
                    y: bounds.y + chip.y,
                    width: chip.width,
                    height: chip.height,
                };
                if abs.contains(pos) {
                    if !state.is_focused {
                        state.is_focused = true;
                        shell.publish((widget.on_message)(TokenInputMessage::Focused));
                    }
                    let click = iced::advanced::mouse::Click::new(
                        pos,
                        mouse::Button::Left,
                        state.last_click,
                    );
                    state.last_click = Some(click);
                    // One click selects the bubble; a second one on it turns
                    // it back into text to edit.
                    let message = if click.kind() == iced::advanced::mouse::click::Kind::Double {
                        state.drag_tracking = None;
                        TokenInputMessage::EditToken(token.id)
                    } else {
                        TokenInputMessage::SelectToken(token.id)
                    };
                    shell.publish((widget.on_message)(message));
                    shell.capture_event();
                    return;
                }
            }
        }

        // Clicked in field, not on token - focus, and put the caret where
        // the click landed in the text being typed.
        if !state.is_focused {
            state.is_focused = true;
            shell.publish((widget.on_message)(TokenInputMessage::Focused));
        }
        let (text_x, _) = text_area_origin(&state.token_bounds, bounds.width, state.chip_v_offset);
        let caret = caret_at(widget.text, pos.x - bounds.x - text_x);
        state.caret = Some((caret, widget.text.len()));
        shell.publish((widget.on_message)(TokenInputMessage::DeselectTokens));
        shell.capture_event();
        return;
    }

    // Clicked outside - blur
    if state.is_focused {
        state.is_focused = false;
        shell.publish((widget.on_message)(TokenInputMessage::Blurred));
    }
}

pub(super) fn handle_key_press<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    key: &keyboard::Key,
    modifiers: &keyboard::Modifiers,
    text: Option<&str>,
    shell: &mut Shell<'_, M>,
) {
    match key {
        // Paste: Ctrl+V / Cmd+V. Clipboard reads are asynchronous: request the
        // contents and mark the pending paste. The `Paste` message is emitted
        // from `update` once the runtime delivers the `clipboard::Event::Read`.
        keyboard::Key::Character(c)
            if (c.as_str() == "v" || c.as_str() == "V") && modifiers.command() =>
        {
            shell.read_clipboard(clipboard::Kind::Text);
            state.pending_paste = true;
            shell.capture_event();
        }

        // Copy: Ctrl+C / Cmd+C - only when a token is selected (text-input
        // copy is handled by the inner text_input). The actual clipboard
        // write happens at the compose layer so group tokens can be
        // expanded asynchronously.
        keyboard::Key::Character(c)
            if (c.as_str() == "c" || c.as_str() == "C") && modifiers.command() =>
        {
            if let Some(selected) = widget.selected_token {
                shell.publish((widget.on_message)(TokenInputMessage::CopyToken(selected)));
                shell.capture_event();
            }
        }

        // Cut: Ctrl+X / Cmd+X - same path as Copy, plus token deletion.
        keyboard::Key::Character(c)
            if (c.as_str() == "x" || c.as_str() == "X") && modifiers.command() =>
        {
            if let Some(selected) = widget.selected_token {
                shell.publish((widget.on_message)(TokenInputMessage::CutToken(selected)));
                shell.capture_event();
            }
        }

        // Delete key: remove selected token. A focused field keeps the key
        // even with nothing to remove, so it never reaches the window-level
        // shortcut that deletes the conversation.
        keyboard::Key::Named(keyboard::key::Named::Delete) => {
            let caret = caret_offset(state, widget.text);
            if let Some(selected) = widget.selected_token {
                shell.publish((widget.on_message)(TokenInputMessage::RemoveToken(
                    selected,
                )));
            } else if caret < widget.text.len() {
                let end = next_boundary(widget.text, caret);
                replace_text(widget, state, caret..end, "", shell);
            }
            shell.capture_event();
        }

        // Backspace
        keyboard::Key::Named(keyboard::key::Named::Backspace) => {
            handle_backspace(widget, state, shell);
        }

        // Home / End: caret to the ends of the text being typed
        keyboard::Key::Named(keyboard::key::Named::Home) if !widget.text.is_empty() => {
            state.caret = Some((0, widget.text.len()));
            shell.request_redraw();
            shell.capture_event();
        }
        keyboard::Key::Named(keyboard::key::Named::End) if !widget.text.is_empty() => {
            state.caret = None;
            shell.request_redraw();
            shell.capture_event();
        }

        // Arrow Up/Down: autocomplete navigation when dropdown open
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) if widget.autocomplete_open => {
            shell.publish((widget.on_message)(TokenInputMessage::AutocompleteUp));
            shell.capture_event();
        }
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) if widget.autocomplete_open => {
            shell.publish((widget.on_message)(TokenInputMessage::AutocompleteDown));
            shell.capture_event();
        }

        // Left arrow: navigate to previous token
        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
            handle_arrow_left(widget, state, shell);
        }

        // Right arrow: navigate to next token or text
        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
            handle_arrow_right(widget, state, shell);
        }

        // Escape: dismiss autocomplete if open, otherwise blur
        keyboard::Key::Named(keyboard::key::Named::Escape) => {
            if widget.autocomplete_open {
                shell.publish((widget.on_message)(
                    TokenInputMessage::AutocompleteDismissKey,
                ));
            } else {
                shell.publish((widget.on_message)(TokenInputMessage::Blurred));
            }
            shell.capture_event();
        }

        // Enter / Tab: accept autocomplete if open, otherwise tokenize
        keyboard::Key::Named(keyboard::key::Named::Enter | keyboard::key::Named::Tab) => {
            if widget.autocomplete_open {
                shell.publish((widget.on_message)(TokenInputMessage::AutocompleteAccept));
                shell.capture_event();
            } else if !widget.text.is_empty() {
                let text = widget.text.to_string();
                shell.publish((widget.on_message)(TokenInputMessage::TokenizeText(text)));
                shell.capture_event();
            }
        }

        // Comma / Semicolon: always tokenize
        keyboard::Key::Character(c)
            if !modifiers.command() && (c.as_str() == "," || c.as_str() == ";") =>
        {
            if !widget.text.is_empty() {
                let text = widget.text.to_string();
                shell.publish((widget.on_message)(TokenInputMessage::TokenizeText(text)));
            }
            shell.capture_event();
        }

        // Space: tokenize if looks like email, else append
        keyboard::Key::Named(keyboard::key::Named::Space) if !modifiers.command() => {
            handle_space(widget, state, shell);
        }

        // Regular character input. Prefer the `text` field from the
        // KeyPressed event (post-modifier, post-IME) over `Key::Character`,
        // which only carries the unmodified logical key. Otherwise typing
        // e.g. Shift+2 on a non-US layout inserts "2" instead of "@".
        keyboard::Key::Character(c) if !modifiers.command() => {
            if widget.selected_token.is_some() {
                shell.publish((widget.on_message)(TokenInputMessage::DeselectTokens));
            }
            let typed = text.unwrap_or(c.as_str());
            let caret = caret_offset(state, widget.text);
            replace_text(widget, state, caret..caret, typed, shell);
            shell.capture_event();
        }

        _ => {}
    }
}

fn handle_backspace<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    shell: &mut Shell<'_, M>,
) {
    if widget.text.is_empty() {
        if let Some(selected) = widget.selected_token {
            shell.publish((widget.on_message)(TokenInputMessage::RemoveToken(
                selected,
            )));
            shell.capture_event();
            return;
        }
        if !widget.tokens.is_empty() {
            shell.publish((widget.on_message)(TokenInputMessage::BackspaceAtStart));
        }
        shell.capture_event();
    } else {
        let caret = caret_offset(state, widget.text);
        if caret > 0 {
            let start = previous_boundary(widget.text, caret);
            replace_text(widget, state, start..caret, "", shell);
        }
        shell.capture_event();
    }
}

/// Move the caret one character inside the text being typed. False when no
/// text is being typed, a bubble is selected, or the caret is already at
/// that end, so the arrow goes on to the bubbles.
fn move_caret<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    forward: bool,
    shell: &mut Shell<'_, M>,
) -> bool {
    if widget.text.is_empty() || widget.selected_token.is_some() {
        return false;
    }
    let caret = caret_offset(state, widget.text);
    let moved = if forward {
        next_boundary(widget.text, caret)
    } else {
        previous_boundary(widget.text, caret)
    };
    if moved == caret {
        return false;
    }
    state.caret = Some((moved, widget.text.len()));
    shell.request_redraw();
    shell.capture_event();
    true
}

fn handle_arrow_left<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    shell: &mut Shell<'_, M>,
) {
    if move_caret(widget, state, false, shell) || widget.tokens.is_empty() {
        return;
    }

    match selected_index(widget.tokens, widget.selected_token) {
        Some(idx) if idx > 0 => {
            // Move selection to previous token
            shell.publish((widget.on_message)(TokenInputMessage::ArrowSelectToken(
                widget.tokens[idx - 1].id,
            )));
            shell.capture_event();
        }
        Some(_) => {
            // Already at first token, do nothing
            shell.capture_event();
        }
        None => {
            // The caret is at the start of the text: select the last token
            if let Some(last) = widget.tokens.last() {
                shell.publish((widget.on_message)(TokenInputMessage::ArrowSelectToken(
                    last.id,
                )));
                shell.capture_event();
            }
        }
    }
}

fn handle_arrow_right<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    shell: &mut Shell<'_, M>,
) {
    if move_caret(widget, state, true, shell) || widget.tokens.is_empty() {
        return;
    }

    if let Some(idx) = selected_index(widget.tokens, widget.selected_token) {
        if idx + 1 < widget.tokens.len() {
            // Move selection to next token
            shell.publish((widget.on_message)(TokenInputMessage::ArrowSelectToken(
                widget.tokens[idx + 1].id,
            )));
        } else {
            // At last token: deselect and focus text
            shell.publish((widget.on_message)(TokenInputMessage::ArrowToText));
        }
        shell.capture_event();
    }
}

fn handle_space<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    shell: &mut Shell<'_, M>,
) {
    if !widget.text.is_empty() && widget.text.contains('@') {
        let text = widget.text.to_string();
        shell.publish((widget.on_message)(TokenInputMessage::TokenizeText(text)));
    } else if !widget.text.is_empty() {
        let caret = caret_offset(state, widget.text);
        replace_text(widget, state, caret..caret, " ", shell);
    }
    shell.capture_event();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_caret(offset: usize, length: usize) -> TokenInputState {
        TokenInputState {
            caret: Some((offset, length)),
            ..TokenInputState::default()
        }
    }

    #[test]
    fn the_caret_sits_at_the_end_until_it_is_moved() {
        assert_eq!(
            caret_offset(&TokenInputState::default(), "bob@example.com"),
            15
        );
    }

    #[test]
    fn a_moved_caret_is_kept_while_the_text_keeps_its_length() {
        assert_eq!(caret_offset(&state_with_caret(3, 15), "bob@example.com"), 3);
    }

    #[test]
    fn a_caret_set_for_another_text_falls_back_to_the_end() {
        // The app replaced the text, for instance with a bubble being edited.
        assert_eq!(
            caret_offset(&state_with_caret(3, 15), "alice@example.com"),
            17
        );
    }

    #[test]
    fn the_caret_steps_over_whole_characters() {
        let text = "zoé@example.com";
        assert_eq!(next_boundary(text, 2), 4);
        assert_eq!(previous_boundary(text, 4), 2);
        assert_eq!(previous_boundary(text, 0), 0);
        assert_eq!(next_boundary(text, text.len()), text.len());
    }
}
