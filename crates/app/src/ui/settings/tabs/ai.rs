use iced::widget::column;
use iced::{Element, Length};
use rust_i18n::t;

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::widgets;

pub(super) fn ai_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill.max(SETTINGS_CONTENT_MAX_WIDTH));

    col = col.push(section(
        tr(t!("settings.ai.provider")),
        vec![setting_row(
            tr(t!("settings.ai.ai_provider")),
            widgets::select(
                &["Claude", "OpenAI", "Gemini", "Ollama", "Copilot"],
                &state.ai_provider,
                state.open_select == Some(SelectField::AiProvider),
                SettingsMessage::ToggleSelect(SelectField::AiProvider),
                SettingsMessage::AiProviderChanged,
            ),
            SettingsMessage::ToggleSelect(SelectField::AiProvider),
        )],
    ));

    if state.ai_provider == "Ollama" {
        col = col.push(section(
            tr(t!("settings.ai.local_server")),
            vec![
                input_row(
                    "ollama-url",
                    &t!("settings.ai.server_url"),
                    "http://localhost:11434",
                    state.ai_ollama_url.text(),
                    SettingsMessage::OllamaUrlChanged,
                    InputField::OllamaUrl,
                ),
                input_row(
                    "ollama-model",
                    &t!("settings.ai.model_name"),
                    &t!("settings.ai.model_name_placeholder"),
                    state.ai_ollama_model.text(),
                    SettingsMessage::OllamaModelChanged,
                    InputField::OllamaModel,
                ),
            ],
        ));
    } else {
        let key_label = match state.ai_provider.as_str() {
            "OpenAI" => t!("settings.ai.openai_api_key"),
            "Gemini" => t!("settings.ai.google_api_key"),
            "Copilot" => t!("settings.ai.github_token"),
            _ => t!("settings.ai.anthropic_api_key"),
        };

        let model_options: &[&str] = match state.ai_provider.as_str() {
            "OpenAI" => &["gpt-4o", "gpt-4o-mini", "o4-mini"],
            "Gemini" => &[
                "gemini-2.0-flash",
                "gemini-2.5-flash-preview-05-20",
                "gemini-2.5-pro",
            ],
            "Copilot" => &["openai/gpt-4o", "openai/gpt-4o-mini"],
            _ => &[
                "claude-haiku-4-5-20251001",
                "claude-sonnet-4-5",
                "claude-sonnet-4-6",
                "claude-opus-4-6",
            ],
        };

        col = col.push(section(
            tr(t!("settings.ai.api_key")),
            vec![
                input_row_secure(
                    "ai-api-key",
                    &key_label,
                    "",
                    state.ai_api_key.text(),
                    SettingsMessage::AiApiKeyChanged,
                    InputField::AiApiKey,
                ),
                setting_row(
                    tr(t!("settings.ai.model")),
                    widgets::select(
                        model_options,
                        &state.ai_model,
                        state.open_select == Some(SelectField::AiModel),
                        SettingsMessage::ToggleSelect(SelectField::AiModel),
                        SettingsMessage::AiModelChanged,
                    ),
                    SettingsMessage::ToggleSelect(SelectField::AiModel),
                ),
            ],
        ));
    }

    col = col.push(section_with_subtitle(
        tr(t!("settings.ai.features")),
        tr(t!("settings.ai.features_description")),
        vec![
            toggle_row(
                tr(t!("settings.ai.enable")),
                t!("settings.ai.enable_description"),
                state.ai_enabled,
                SettingsMessage::ToggleAiEnabled,
            ),
            toggle_row(
                tr(t!("settings.ai.auto_categorize")),
                t!("settings.ai.auto_categorize_description"),
                state.ai_auto_categorize,
                SettingsMessage::ToggleAiAutoCategorize,
            ),
            toggle_row(
                tr(t!("settings.ai.auto_summarize")),
                t!("settings.ai.auto_summarize_description"),
                state.ai_auto_summarize,
                SettingsMessage::ToggleAiAutoSummarize,
            ),
        ],
    ));

    col = col.push(section(
        tr(t!("settings.ai.auto_draft_replies")),
        vec![
            toggle_row(
                tr(t!("settings.ai.auto_draft")),
                t!("settings.ai.auto_draft_description"),
                state.ai_auto_draft,
                SettingsMessage::ToggleAiAutoDraft,
            ),
            toggle_row(
                tr(t!("settings.ai.writing_style")),
                t!("settings.ai.writing_style_description"),
                state.ai_writing_style,
                SettingsMessage::ToggleAiWritingStyle,
            ),
        ],
    ));

    col = col.push(section(
        tr(t!("settings.ai.auto_archive_categories")),
        vec![
            toggle_row(
                tr(t!("settings.common.category_updates")),
                t!("settings.ai.archive_updates_description"),
                state.ai_auto_archive_updates,
                SettingsMessage::ToggleAiAutoArchiveUpdates,
            ),
            toggle_row(
                tr(t!("settings.common.category_promotions")),
                t!("settings.ai.archive_promotions_description"),
                state.ai_auto_archive_promotions,
                SettingsMessage::ToggleAiAutoArchivePromotions,
            ),
            toggle_row(
                tr(t!("settings.common.category_social")),
                t!("settings.ai.archive_social_description"),
                state.ai_auto_archive_social,
                SettingsMessage::ToggleAiAutoArchiveSocial,
            ),
            toggle_row(
                tr(t!("settings.common.category_newsletters")),
                t!("settings.ai.archive_newsletters_description"),
                state.ai_auto_archive_newsletters,
                SettingsMessage::ToggleAiAutoArchiveNewsletters,
            ),
        ],
    ));

    col.into()
}
