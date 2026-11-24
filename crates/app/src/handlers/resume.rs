//! Come back, after a relaunch, to where the reader was in the main window.
//!
//! The session file records the place (folder, open conversation, scroll
//! positions, search, message being written). It is put back in steps, as
//! what each step needs has loaded: the accounts for the folder, the list
//! for the conversation, the conversation for its scroll position.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;

use crate::pop_out::PopOutWindow;
use crate::pop_out::compose::ComposeState;
use crate::pop_out::session::{
    CaretEntry, ComposeDetail, InlineComposeEntry, MainViewEntry, SearchEntry,
};
use crate::ui::thread_list::{THREAD_LIST_SCROLL_ID, ThreadListMessage, ThreadListMode};
use crate::{Message, ReadyApp};

/// How long the open conversation keeps going back to the recorded scroll
/// position. The conversation is loaded again as each store finishes
/// opening at startup; past this delay the position is the reader's own.
const READING_RESUME: Duration = Duration::from_secs(20);

/// What is left to put back once the list the reader was in has loaded.
pub(crate) struct ResumePoint {
    open_thread: Option<(String, String)>,
    thread_list_offset: f32,
    reading_offset: f32,
    reading_anchor: Option<(usize, f32)>,
    /// The search to run again, once the folder under it has loaded.
    search: Option<SearchEntry>,
    /// The place is in the results of that search, not in the folder.
    awaits_search: bool,
    inline_compose: Option<InlineComposeEntry>,
}

/// The scroll position to give back to one conversation.
pub(crate) struct ReadingResume {
    account_id: String,
    thread_id: String,
    offset: f32,
    anchor: Option<(usize, f32)>,
    until: Instant,
}

impl ReadyApp {
    /// Go back to the folder the reader was in. Called once the accounts
    /// are known, before their mail is loaded: an account removed since
    /// leaves the default view in place.
    pub(crate) fn resume_main_view(&mut self) {
        let Some(view) = self.resume_view.take() else {
            return;
        };
        let account_exists = view
            .scope
            .account_id()
            .is_none_or(|id| self.sidebar.accounts.iter().any(|account| account.id == id));
        if !account_exists {
            return;
        }
        self.sidebar.selected_scope = view.scope;
        self.sidebar.selection = view.selection;
        self.update_thread_list_context_from_sidebar();
        self.resume_point = Some(ResumePoint {
            open_thread: view.open_thread,
            thread_list_offset: view.thread_list_offset,
            reading_offset: view.reading_offset,
            reading_anchor: view.reading_anchor,
            search: view.search,
            awaits_search: false,
            inline_compose: view.inline_compose,
        });
    }

    /// The list has loaded: open the conversation the reader was in and
    /// scroll back to it. `from_search` tells the results of a search from
    /// the mail of a folder; the place is in one of the two.
    pub(crate) fn resume_after_list_loaded(&mut self, from_search: bool) -> Task<Message> {
        let Some(point) = self.resume_point.as_mut() else {
            return Task::none();
        };
        if point.awaits_search != from_search {
            return Task::none();
        }
        if let Some(search) = point.search.take() {
            // The folder is in: run the search over it, and wait for its
            // results before going further.
            point.awaits_search = true;
            self.search_sender_name = search.sender.clone().zip(search.sender_name);
            self.search_sender = search.sender;
            self.search_query.reset(search.query.clone());
            self.thread_list.search_query = search.query;
            return self.search_without_saving(false);
        }
        let Some(point) = self.resume_point.take() else {
            return Task::none();
        };

        let mut tasks = vec![iced::widget::operation::scroll_to(
            THREAD_LIST_SCROLL_ID,
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(point.thread_list_offset),
            },
            iced::widget::operation::Animation::Instant,
        )];
        self.view_offsets.0 = point.thread_list_offset;

        let row = point.open_thread.and_then(|(account_id, thread_id)| {
            self.thread_list
                .threads
                .iter()
                .position(|thread| thread.account_id == account_id && thread.id == thread_id)
                .map(|index| (index, account_id, thread_id))
        });
        if let Some((index, account_id, thread_id)) = row {
            let is_the_draft_being_written = point
                .inline_compose
                .as_ref()
                .is_some_and(|compose| compose.draft_id == thread_id);
            if is_the_draft_being_written {
                // Selecting a draft opens it: the message being written is
                // opened below, with its caret and attachments.
                self.thread_list.selected_thread = Some(index);
            } else {
                self.reading_resume = Some(ReadingResume {
                    account_id,
                    thread_id,
                    offset: point.reading_offset,
                    anchor: point.reading_anchor,
                    until: Instant::now() + READING_RESUME,
                });
                tasks.push(self.handle_thread_list(ThreadListMessage::SelectThread(index)));
            }
        }

        if let Some(compose) = point.inline_compose {
            let db = Arc::clone(&self.db);
            let InlineComposeEntry { draft_id, detail } = compose;
            tasks.push(Task::perform(
                async move {
                    let core_db = db.read_db_state();
                    rtsk::db::queries_extra::db_get_local_draft(&core_db, draft_id).await
                },
                move |result| Message::InlineComposeResumed {
                    detail: detail.clone(),
                    result,
                },
            ));
        }
        Task::batch(tasks)
    }

    /// The place to give back to the conversation that has just loaded,
    /// when it is the one the reader was in: the bare offset, and the
    /// message at the top with the distance into it.
    pub(crate) fn reading_offset_to_resume(
        &mut self,
        account_id: &str,
        thread_id: &str,
    ) -> Option<(f32, Option<(usize, f32)>)> {
        let resume = self.reading_resume.as_ref()?;
        let same = resume.account_id == account_id && resume.thread_id == thread_id;
        if !same || Instant::now() > resume.until {
            self.reading_resume = None;
            return None;
        }
        // Kept until the place is the reader's own: the conversation is
        // loaded again as each store opens, and each load goes back there.
        self.view_offsets.1 = resume.offset;
        self.reading_anchor = resume.anchor;
        Some((resume.offset, resume.anchor))
    }

    /// Put back the message that was being written in the main window.
    pub(crate) fn handle_inline_compose_resumed(
        &mut self,
        detail: &ComposeDetail,
        result: Result<Option<rtsk::db::types::DbLocalDraft>, String>,
    ) -> Task<Message> {
        let draft = match result {
            Ok(Some(draft)) => draft,
            Ok(None) => {
                log::info!("The message being written at the last close has no draft left");
                return Task::none();
            }
            Err(error) => {
                log::error!("Failed to load the message being written at the last close: {error}");
                return Task::none();
            }
        };
        let mut state = ComposeState::from_local_draft(&self.sidebar.accounts, &draft);
        apply_compose_detail(&mut state, detail);
        self.open_local_draft(state)
    }

    /// The place to record for the next launch.
    pub(crate) fn main_view_entry(&self, data_dir: &std::path::Path) -> MainViewEntry {
        let open_thread = self
            .thread_list
            .selected_thread
            .and_then(|index| self.thread_list.threads.get(index))
            .filter(|thread| !thread.is_held_send())
            .map(|thread| (thread.account_id.clone(), thread.id.clone()));
        let searching =
            self.thread_list.mode == ThreadListMode::Search || self.search_sender.is_some();
        let search = searching.then(|| SearchEntry {
            query: self.search_query.text().to_string(),
            sender: self.search_sender.clone(),
            sender_name: self
                .search_sender_name
                .as_ref()
                .filter(|(address, _)| Some(address) == self.search_sender.as_ref())
                .map(|(_, name)| name.clone()),
        });
        let inline_compose = self
            .inline_compose
            .and_then(|id| self.pop_out_windows.get(&id))
            .and_then(|window| match window {
                PopOutWindow::Compose(state) => Some(InlineComposeEntry {
                    draft_id: state.draft_id.clone(),
                    detail: compose_detail(state, data_dir),
                }),
                _ => None,
            });
        MainViewEntry {
            scope: self.sidebar.selected_scope.clone(),
            selection: self.sidebar.selection.clone(),
            open_thread,
            thread_list_offset: self.view_offsets.0,
            reading_offset: self.view_offsets.1,
            reading_anchor: self.reading_anchor,
            search,
            inline_compose,
        }
    }
}

/// What the session records about a message being written beyond its draft.
pub(crate) fn compose_detail(state: &ComposeState, data_dir: &std::path::Path) -> ComposeDetail {
    // Inside a table cell the selection counts from that cell: it is not
    // recorded, and the message reopens with the caret at its start.
    let caret = state.body.scope().is_empty().then(|| {
        let selection = state.body.selection;
        CaretEntry {
            anchor: (selection.anchor.block_index, selection.anchor.offset),
            focus: (selection.focus.block_index, selection.focus.offset),
            focused: state.body.is_focused(),
        }
    });
    let recipient_names = [&state.to, &state.cc, &state.bcc]
        .into_iter()
        .flat_map(|field| &field.tokens)
        .filter(|token| !token.is_group && token.label != token.email)
        .map(|token| (token.email.clone(), token.label.clone()))
        .collect();
    ComposeDetail {
        caret,
        attachments: crate::pop_out::session::store_attachments(data_dir, &state.attachments),
        recipient_names,
    }
}

/// Give a message read back from its draft its caret and its attachments.
pub(crate) fn apply_compose_detail(state: &mut ComposeState, detail: &ComposeDetail) {
    if let Some(data_dir) = crate::APP_DATA_DIR.get() {
        state.attachments =
            crate::pop_out::session::load_attachments(data_dir, &detail.attachments);
    }
    for token in [&mut state.to, &mut state.cc, &mut state.bcc]
        .into_iter()
        .flat_map(|field| field.tokens.iter_mut())
    {
        if let Some((_, name)) = detail
            .recipient_names
            .iter()
            .find(|(address, _)| *address == token.email)
        {
            token.label.clone_from(name);
        }
    }
    let Some(caret) = detail.caret else {
        return;
    };
    let anchor = rte::DocPosition::new(caret.anchor.0, caret.anchor.1);
    let focus = rte::DocPosition::new(caret.focus.0, caret.focus.1);
    let document = &state.body.document;
    if document.is_valid_position(anchor) && document.is_valid_position(focus) {
        state
            .body
            .set_selection(rte::DocSelection::range(anchor, focus));
    }
    if caret.focused {
        state.body.perform(rte::Action::Focus);
    }
}
