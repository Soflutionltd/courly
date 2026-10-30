use std::collections::HashSet;
use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Padding, Task};
use rust_i18n::t;

use crate::component::Component;
use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::thread_date_groups::{DateGroup, GroupKey};
use crate::ui::thread_menu::{RowMenu, RowMenuItem, RowMenuPage, RowMenuPages};
use crate::ui::widgets;

// ── Auto-advance direction ─────────────────────────────

/// Which direction to advance after an email action removes the current thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Previous variant kept for symmetry; only Next is dispatched today
pub enum AutoAdvanceDirection {
    /// Select the next thread (below) after action.
    Next,
    /// Select the previous thread (above) after action.
    Previous,
}

// ── Typeahead types ───────────────────────────────────

/// A single typeahead suggestion item.
#[derive(Debug, Clone)]
pub struct TypeaheadItem {
    pub label: String,
    pub detail: Option<String>,
    pub insert_value: String,
}

/// Direction for navigating typeahead suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeaheadDirection {
    Up,
    Down,
}

/// State for the typeahead suggestion dropdown.
#[derive(Debug)]
pub struct TypeaheadState {
    pub visible: bool,
    pub items: Vec<TypeaheadItem>,
    pub selected: usize,
    /// Generation counter for stale typeahead result detection.
    pub generation: rtsk::generation::GenerationCounter<rtsk::generation::Typeahead>,
}

impl Default for TypeaheadState {
    fn default() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
            selected: 0,
            generation: rtsk::generation::GenerationCounter::new(),
        }
    }
}

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ThreadListMessage {
    SelectThread(usize),
    /// Ctrl+click: toggle a thread in/out of multi-selection.
    ToggleThread(usize),
    /// Shift+click: range-select from last selected to this index.
    RangeSelectThread(usize),
    /// Select all threads (Ctrl+A).
    SelectAll,
    /// Select from the current thread to the end of the list (Ctrl+Shift+A).
    SelectFromHere,
    /// Clear multi-selection.
    SelectNone,
    /// The search bar text changed.
    SearchInput(String),
    /// The user pressed Enter in the search bar.
    SearchSubmit,
    /// Move selection up by one.
    SelectPrevious,
    /// Move selection down by one.
    SelectNext,
    /// Jump to first thread.
    SelectFirst,
    /// Jump to last thread.
    SelectLast,
    /// Open/activate the selected thread (Enter).
    ActivateSelected,
    /// Deselect current thread (Escape).
    Deselect,
    /// Widen search scope to "All" accounts.
    WidenSearchScope,
    /// Auto-advance after an email action removed the actioned thread(s).
    AutoAdvance,
    /// Navigate typeahead suggestions up or down.
    TypeaheadNavigate(TypeaheadDirection),
    /// Typeahead suggestion items loaded from async query (carries generation).
    TypeaheadItemsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::Typeahead>,
        Vec<TypeaheadItem>,
    ),
    /// User selected a typeahead suggestion by index.
    TypeaheadSelect(usize),
    /// Pin, flag or delete from the buttons on one row. Acts on that row
    /// alone, whatever else is selected.
    RowAction {
        index: usize,
        action: widgets::ThreadRowAction,
    },
    /// Right-click on a row, `offset` measured from the row's corner.
    OpenRowMenu {
        index: usize,
        offset: iced::Vector,
    },
    RowMenuShowPage(RowMenuPage),
    RowMenuPick(RowMenuItem),
    CloseRowMenu,
    /// Fold or unfold the rows under a date header.
    ToggleDateGroup(DateGroup),
    /// Dismiss the typeahead dropdown.
    TypeaheadDismiss,
}

/// Events the thread list emits upward to the App.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // BatchAction / TypeaheadQuery / SearchUndo / SearchRedo land per follow-up
pub enum ThreadListEvent {
    ThreadSelected(usize),
    /// The search query text changed (propagated to App for debounce).
    SearchQueryChanged(String),
    /// The user pressed Enter - execute search immediately.
    SearchExecute,
    /// Thread deselected.
    ThreadDeselected,
    /// User clicked "All" to widen search scope.
    WidenSearchScope,
    /// Multi-selection changed - App may want to update action availability.
    MultiSelectionChanged(usize),
    /// Auto-advance selected a new thread after action.
    AutoAdvance {
        /// The index that was auto-selected (None if list is now empty).
        new_index: Option<usize>,
    },
    /// Batch action: apply email action to all selected thread indices.
    BatchAction(Vec<usize>),
    /// Typeahead operator query needs async data.
    TypeaheadQuery {
        operator: String,
        partial_value: String,
    },
    /// User selected a typeahead suggestion.
    TypeaheadSelected(usize),
    /// Search undo requested.
    SearchUndo,
    /// Search redo requested.
    SearchRedo,
    /// A row button (pin, flag, delete) was pressed.
    RowAction {
        index: usize,
        action: widgets::ThreadRowAction,
    },
    /// The right-click menu opened; the app fills its pages. A row outside
    /// the selection became the only selected one.
    RowMenuOpened {
        index: usize,
        selection_changed: bool,
    },
    RowMenuPicked(RowMenuItem),
}

// ── Thread list mode ───────────────────────────────────

/// What the thread list is currently displaying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadListMode {
    /// Browsing a sidebar scope - threads loaded from scoped DB query.
    Scope,
    /// Displaying search results - threads came from the unified search pipeline.
    Search,
}

// ── State ──────────────────────────────────────────────

pub struct ThreadList {
    pub threads: Vec<Thread>,
    pub selected_thread: Option<usize>,
    /// Multi-selection set (indices into `threads`). Empty when single-select.
    pub selected_threads: HashSet<usize>,
    /// The last index that was clicked/toggled, used as anchor for shift-click range select.
    pub last_selected_anchor: Option<usize>,
    pub folder_name: String,
    pub scope_name: String,
    /// Current display mode (folder view vs search results).
    pub mode: ThreadListMode,
    /// The search query string, set by App before view() is called.
    pub search_query: String,
    /// `updated_at` of the active pinned search (unix seconds), when the
    /// thread list is showing a pinned-search snapshot. Drives the
    /// "Last updated …" indicator under the search bar. `None` means
    /// no pinned search is active.
    pub pinned_search_updated_at: Option<i64>,
    /// Direction to auto-advance after an email action removes a thread.
    pub auto_advance_direction: AutoAdvanceDirection,
    /// Typeahead suggestion state.
    pub typeahead: TypeaheadState,
    /// BIMI logo LRU cache - shared with App for cross-component access.
    pub bimi_cache: Arc<rtsk::bimi::BimiLruCache>,
    /// Show Today / Yesterday / ... headers. Mirrors the user preference.
    pub date_groups: bool,
    /// Date headers folded by the user, for every folder of the session.
    pub collapsed_groups: HashSet<DateGroup>,
    /// Open right-click menu, if any.
    pub row_menu: Option<RowMenu>,
}

impl ThreadList {
    pub fn new(bimi_cache: Arc<rtsk::bimi::BimiLruCache>) -> Self {
        Self {
            threads: Vec::new(),
            selected_thread: None,
            selected_threads: HashSet::new(),
            last_selected_anchor: None,
            folder_name: t!("main.folder.inbox").into_owned(),
            scope_name: t!("main.thread_list.scope_all").into_owned(),
            mode: ThreadListMode::Scope,
            search_query: String::new(),
            pinned_search_updated_at: None,
            auto_advance_direction: AutoAdvanceDirection::Next,
            typeahead: TypeaheadState::default(),
            bimi_cache,
            date_groups: true,
            collapsed_groups: HashSet::new(),
            row_menu: None,
        }
    }

    /// The date group of each row, `None` when the list shows no headers.
    fn row_groups(&self) -> Option<Vec<DateGroup>> {
        if !self.date_groups {
            return None;
        }
        let keys: Vec<GroupKey> = self
            .threads
            .iter()
            .map(|thread| GroupKey {
                pinned: thread.is_pinned,
                last_message_at: thread.last_message_at,
            })
            .collect();
        let now = jiff::Zoned::now();
        super::thread_date_groups::groups(&keys, now.date(), now.time_zone())
    }

    /// Whether each row is shown, i.e. not under a folded header.
    fn visible_rows(&self) -> Vec<bool> {
        match self.row_groups() {
            Some(groups) => groups
                .iter()
                .map(|group| !self.collapsed_groups.contains(group))
                .collect(),
            None => vec![true; self.threads.len()],
        }
    }

    fn toggle_date_group(&mut self, group: DateGroup) -> Option<ThreadListEvent> {
        if self.collapsed_groups.remove(&group) {
            return None;
        }
        self.collapsed_groups.insert(group);
        let visible = self.visible_rows();
        let is_hidden = |idx: &usize| !visible.get(*idx).copied().unwrap_or(false);
        if self.row_menu.as_ref().is_some_and(|menu| is_hidden(&menu.index)) {
            self.row_menu = None;
        }
        // Actions must not reach rows the user no longer sees. As in
        // Outlook, folding the open conversation away closes it.
        if self.selected_thread.as_ref().is_some_and(is_hidden) {
            self.selected_thread = None;
            self.clear_multi_select();
            return Some(ThreadListEvent::ThreadDeselected);
        }
        let before = self.selected_threads.len();
        self.selected_threads.retain(|idx| !is_hidden(idx));
        (self.selected_threads.len() != before)
            .then_some(ThreadListEvent::MultiSelectionChanged(self.selected_threads.len()))
    }

    pub fn set_row_menu_pages(&mut self, pages: RowMenuPages) {
        if let Some(menu) = self.row_menu.as_mut() {
            menu.pages = pages;
        }
    }

    /// The search field, rendered in the window's title bar rather than
    /// in the list header. Keeps the `search-bar` id that focus commands
    /// target.
    pub fn search_bar(&self) -> Element<'_, ThreadListMessage> {
        text_input(t!("main.search.placeholder"), &self.search_query)
            .id("search-bar")
            .on_input(ThreadListMessage::SearchInput)
            .on_submit(ThreadListMessage::SearchSubmit)
            .size(TEXT_MD)
            .padding(PAD_INPUT)
            .style(theme::TextInputClass::TitleSearch.style())
            .into()
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    /// Replace the list, carrying every selection index over to the same
    /// thread in the new list. Background sync reloads the list while the
    /// user reads, so an index kept as-is would jump to a neighbour as soon
    /// as a thread above it is added or removed.
    pub fn set_threads(&mut self, threads: Vec<Thread>) {
        let key_at = |idx: usize| {
            self.threads
                .get(idx)
                .map(|t| (t.account_id.clone(), t.id.clone()))
        };
        let selected = self.selected_thread.and_then(key_at);
        let anchor = self.last_selected_anchor.and_then(key_at);
        let menu_row = self.row_menu.as_ref().and_then(|menu| key_at(menu.index));
        let multi: Vec<_> = self
            .selected_threads
            .iter()
            .filter_map(|&idx| key_at(idx))
            .collect();
        self.threads = threads;
        let index_of = |key: &(String, String)| {
            self.threads
                .iter()
                .position(|t| t.account_id == key.0 && t.id == key.1)
        };
        self.selected_thread = selected.as_ref().and_then(index_of);
        self.last_selected_anchor = anchor.as_ref().and_then(index_of);
        self.selected_threads = multi.iter().filter_map(index_of).collect();
        match (menu_row.as_ref().and_then(index_of), self.row_menu.as_mut()) {
            (Some(index), Some(menu)) => menu.index = index,
            _ => self.row_menu = None,
        }
    }

    pub fn set_context(&mut self, folder_name: String, scope_name: String) {
        self.folder_name = folder_name;
        self.scope_name = scope_name;
    }

    /// Clear multi-selection state, keeping only single selection.
    pub fn clear_multi_select(&mut self) {
        self.selected_threads.clear();
    }

    /// Number of selected threads (multi-select or single).
    pub fn selection_count(&self) -> usize {
        if self.selected_threads.is_empty() {
            if self.selected_thread.is_some() { 1 } else { 0 }
        } else {
            self.selected_threads.len()
        }
    }

    /// Returns all selected indices (multi-select or single).
    pub fn selected_indices(&self) -> Vec<usize> {
        if self.selected_threads.is_empty() {
            self.selected_thread.into_iter().collect()
        } else {
            let mut indices: Vec<usize> = self.selected_threads.iter().copied().collect();
            indices.sort_unstable();
            indices
        }
    }

    /// Whether a thread at the given index is selected (single or multi).
    fn is_selected(&self, idx: usize) -> bool {
        if self.selected_threads.is_empty() {
            self.selected_thread == Some(idx)
        } else {
            self.selected_threads.contains(&idx)
        }
    }

    /// Move selection to the next thread (down).
    fn select_next(&mut self) -> Option<ThreadListEvent> {
        let visible = self.visible_rows();
        let start = self.selected_thread.map_or(0, |i| i + 1);
        let next = (start..visible.len()).find(|&i| visible[i])?;
        self.select_only(next)
    }

    /// Move selection to the previous thread (up).
    fn select_previous(&mut self) -> Option<ThreadListEvent> {
        let visible = self.visible_rows();
        let prev = match self.selected_thread {
            Some(i) => (0..i).rev().find(|&j| visible[j])?,
            None => visible.iter().position(|&shown| shown)?,
        };
        self.select_only(prev)
    }

    fn select_only(&mut self, idx: usize) -> Option<ThreadListEvent> {
        self.clear_multi_select();
        self.selected_thread = Some(idx);
        self.last_selected_anchor = Some(idx);
        Some(ThreadListEvent::ThreadSelected(idx))
    }

    /// Auto-advance: select the next (or previous) thread after the current
    /// selection was removed by an email action.
    fn auto_advance(&mut self) -> Option<ThreadListEvent> {
        if self.threads.is_empty() {
            self.selected_thread = None;
            self.clear_multi_select();
            return Some(ThreadListEvent::AutoAdvance { new_index: None });
        }

        let prev_idx = self.selected_thread.unwrap_or(0);
        let visible = self.visible_rows();
        let at_or_after = |from: usize| (from..visible.len()).find(|&i| visible[i]);
        let before = |to: usize| (0..to.min(visible.len())).rev().find(|&i| visible[i]);
        let new_idx = match self.auto_advance_direction {
            AutoAdvanceDirection::Next => at_or_after(prev_idx).or_else(|| before(prev_idx)),
            AutoAdvanceDirection::Previous => before(prev_idx).or_else(|| at_or_after(prev_idx)),
        };
        let Some(new_idx) = new_idx else {
            self.selected_thread = None;
            self.clear_multi_select();
            return Some(ThreadListEvent::AutoAdvance { new_index: None });
        };

        self.selected_thread = Some(new_idx);
        self.clear_multi_select();
        self.last_selected_anchor = Some(new_idx);
        Some(ThreadListEvent::AutoAdvance {
            new_index: Some(new_idx),
        })
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for ThreadList {
    type Message = ThreadListMessage;
    type Event = ThreadListEvent;

    fn update(
        &mut self,
        message: ThreadListMessage,
    ) -> (Task<ThreadListMessage>, Option<ThreadListEvent>) {
        match message {
            ThreadListMessage::SelectThread(idx) => {
                if self.selected_thread == Some(idx) && self.selected_threads.is_empty() {
                    return (Task::none(), None);
                }

                let had_multi_select = !self.selected_threads.is_empty();
                // Plain click: clear multi-select, select single.
                self.clear_multi_select();
                self.selected_thread = Some(idx);
                self.last_selected_anchor = Some(idx);
                if had_multi_select {
                    (
                        Task::none(),
                        Some(ThreadListEvent::MultiSelectionChanged(0)),
                    )
                } else {
                    (Task::none(), Some(ThreadListEvent::ThreadSelected(idx)))
                }
            }
            ThreadListMessage::ToggleThread(idx) => {
                // Ctrl+click: toggle individual thread in/out.
                if self.selected_threads.is_empty() {
                    // Entering multi-select: seed with current selection.
                    if let Some(prev) = self.selected_thread {
                        self.selected_threads.insert(prev);
                    }
                }
                if self.selected_threads.contains(&idx) {
                    self.selected_threads.remove(&idx);
                } else {
                    self.selected_threads.insert(idx);
                }
                self.last_selected_anchor = Some(idx);
                self.selected_thread = Some(idx);
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::RangeSelectThread(idx) => {
                // Shift+click: range select from anchor to idx.
                let anchor = self.last_selected_anchor.unwrap_or(0);
                let (start, end) = if anchor <= idx {
                    (anchor, idx)
                } else {
                    (idx, anchor)
                };
                if self.selected_threads.is_empty()
                    && let Some(prev) = self.selected_thread
                {
                    self.selected_threads.insert(prev);
                }
                let visible = self.visible_rows();
                for i in (start..=end).filter(|&i| visible.get(i).copied().unwrap_or(false)) {
                    self.selected_threads.insert(i);
                }
                self.selected_thread = Some(idx);
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::SelectAll => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                let visible = self.visible_rows();
                self.selected_threads = (0..visible.len()).filter(|&i| visible[i]).collect();
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::SelectFromHere => {
                if self.threads.is_empty() {
                    return (Task::none(), None);
                }
                let start = self
                    .selected_thread
                    .or(self.last_selected_anchor)
                    .unwrap_or(0);
                let visible = self.visible_rows();
                for idx in (start..visible.len()).filter(|&i| visible[i]) {
                    self.selected_threads.insert(idx);
                }
                let count = self.selected_threads.len();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(count)),
                )
            }
            ThreadListMessage::SelectNone => {
                self.clear_multi_select();
                (
                    Task::none(),
                    Some(ThreadListEvent::MultiSelectionChanged(0)),
                )
            }
            ThreadListMessage::AutoAdvance => {
                let event = self.auto_advance();
                (Task::none(), event)
            }
            ThreadListMessage::SearchInput(query) => (
                Task::none(),
                Some(ThreadListEvent::SearchQueryChanged(query)),
            ),
            ThreadListMessage::SearchSubmit => (Task::none(), Some(ThreadListEvent::SearchExecute)),
            ThreadListMessage::SelectNext => {
                let event = self.select_next();
                (Task::none(), event)
            }
            ThreadListMessage::SelectPrevious => {
                let event = self.select_previous();
                (Task::none(), event)
            }
            ThreadListMessage::SelectFirst => {
                let first = self.visible_rows().iter().position(|&shown| shown);
                (Task::none(), first.and_then(|idx| self.select_only(idx)))
            }
            ThreadListMessage::SelectLast => {
                let last = self.visible_rows().iter().rposition(|&shown| shown);
                (Task::none(), last.and_then(|idx| self.select_only(idx)))
            }
            ThreadListMessage::ActivateSelected => {
                if let Some(idx) = self.selected_thread {
                    (Task::none(), Some(ThreadListEvent::ThreadSelected(idx)))
                } else {
                    (Task::none(), None)
                }
            }
            ThreadListMessage::Deselect => {
                self.selected_thread = None;
                self.clear_multi_select();
                (Task::none(), Some(ThreadListEvent::ThreadDeselected))
            }
            ThreadListMessage::WidenSearchScope => {
                (Task::none(), Some(ThreadListEvent::WidenSearchScope))
            }
            ThreadListMessage::TypeaheadNavigate(direction) => {
                if !self.typeahead.items.is_empty() {
                    match direction {
                        TypeaheadDirection::Up => {
                            if self.typeahead.selected > 0 {
                                self.typeahead.selected -= 1;
                            }
                        }
                        TypeaheadDirection::Down => {
                            if self.typeahead.selected + 1 < self.typeahead.items.len() {
                                self.typeahead.selected += 1;
                            }
                        }
                    }
                }
                (Task::none(), None)
            }
            ThreadListMessage::TypeaheadItemsLoaded(load_gen, items) => {
                if !self.typeahead.generation.is_current(load_gen) {
                    return (Task::none(), None); // stale
                }
                self.typeahead.items = items;
                self.typeahead.selected = 0;
                self.typeahead.visible = !self.typeahead.items.is_empty();
                (Task::none(), None)
            }
            ThreadListMessage::TypeaheadSelect(idx) => {
                self.typeahead.visible = false;
                (Task::none(), Some(ThreadListEvent::TypeaheadSelected(idx)))
            }
            ThreadListMessage::TypeaheadDismiss => {
                self.typeahead.visible = false;
                (Task::none(), None)
            }
            ThreadListMessage::RowAction { index, action } => {
                if self.threads.get(index).is_none() {
                    return (Task::none(), None);
                }
                self.clear_multi_select();
                self.selected_thread = Some(index);
                self.last_selected_anchor = Some(index);
                (
                    Task::none(),
                    Some(ThreadListEvent::RowAction { index, action }),
                )
            }
            ThreadListMessage::OpenRowMenu { index, offset } => {
                // Selecting a local draft opens it in a compose window.
                if self.threads.get(index).is_none_or(|t| t.is_local_draft) {
                    return (Task::none(), None);
                }
                // As in Outlook: inside the selection the menu acts on all
                // of it, outside it selects that row alone.
                let selection_changed = !self.is_selected(index);
                if selection_changed {
                    self.clear_multi_select();
                    self.selected_thread = Some(index);
                    self.last_selected_anchor = Some(index);
                }
                self.row_menu = Some(RowMenu::new(index, offset));
                (
                    Task::none(),
                    Some(ThreadListEvent::RowMenuOpened {
                        index,
                        selection_changed,
                    }),
                )
            }
            ThreadListMessage::RowMenuShowPage(page) => {
                if let Some(menu) = self.row_menu.as_mut() {
                    menu.page = page;
                }
                (Task::none(), None)
            }
            ThreadListMessage::RowMenuPick(item) => {
                if self.row_menu.take().is_none() {
                    return (Task::none(), None);
                }
                (Task::none(), Some(ThreadListEvent::RowMenuPicked(item)))
            }
            ThreadListMessage::CloseRowMenu => {
                self.row_menu = None;
                (Task::none(), None)
            }
            ThreadListMessage::ToggleDateGroup(group) => {
                let event = self.toggle_date_group(group);
                (Task::none(), event)
            }
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn view(&self) -> Element<'_, ThreadListMessage> {
        let selection_count = self.selection_count();
        let header = thread_list_header(
            &self.folder_name,
            &self.scope_name,
            &self.mode,
            self.threads.len(),
            selection_count,
            &self.typeahead,
            self.pinned_search_updated_at,
        );

        let body: Element<'_, ThreadListMessage> = if self.threads.is_empty() {
            let (title, subtitle) = match self.mode {
                ThreadListMode::Scope => (
                    t!("main.thread_list.empty_title"),
                    t!("main.thread_list.empty_subtitle"),
                ),
                ThreadListMode::Search => (
                    t!("main.search.no_results_title"),
                    t!("main.search.no_results_subtitle"),
                ),
            };
            widgets::empty_placeholder(title, subtitle)
        } else {
            thread_list_body(self)
        };

        container(column![header, body].spacing(0).width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::ContainerClass::Base.style())
            .into()
    }
}

// ── View helpers ────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn thread_list_header<'a>(
    folder_name: &'a str,
    scope_name: &'a str,
    mode: &ThreadListMode,
    thread_count: usize,
    selection_count: usize,
    typeahead: &'a TypeaheadState,
    pinned_search_updated_at: Option<i64>,
) -> Element<'a, ThreadListMessage> {
    let staleness: Option<Element<'a, ThreadListMessage>> = pinned_search_updated_at.map(|ts| {
        text(t!(
            "main.search.last_updated",
            time = crate::ui::sidebar::format_relative_time(ts)
        ))
        .size(TEXT_XS)
        .style(theme::TextClass::Tertiary.style())
        .into()
    });

    let context_row: Element<'a, ThreadListMessage> = if selection_count > 1 {
        // Multi-selection: show count and deselect link.
        let count_text = text(t!("main.thread_list.selected", count = selection_count))
            .size(TEXT_SM)
            .style(theme::TextClass::Accent.style());

        let deselect_link = button(
            text(t!("main.thread_list.deselect"))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .on_press(ThreadListMessage::SelectNone)
        .padding(0)
        .style(theme::ButtonClass::Ghost.style());

        row![count_text, Space::new().width(Length::Fill), deselect_link,]
            .align_y(iced::Alignment::Center)
            .into()
    } else {
        match mode {
            ThreadListMode::Scope => row![
                text(folder_name)
                    .size(TEXT_TITLE)
                    .font(crate::font::text_semibold()),
                Space::new().width(Length::Fill),
                text(scope_name)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ]
            .align_y(iced::Alignment::Center)
            .into(),
            ThreadListMode::Search => {
                let results_text = text(if thread_count == 1 {
                    t!("main.search.results_one", count = thread_count)
                } else {
                    t!("main.search.results_other", count = thread_count)
                })
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style());

                let all_link = button(
                    text(t!("main.search.widen_all"))
                        .size(TEXT_SM)
                        .style(theme::TextClass::Accent.style()),
                )
                .on_press(ThreadListMessage::WidenSearchScope)
                .padding(0)
                .style(theme::ButtonClass::Ghost.style());

                row![results_text, Space::new().width(Length::Fill), all_link,]
                    .align_y(iced::Alignment::Center)
                    .into()
            }
        }
    };

    let mut header_col = column![].spacing(SPACE_XXS);
    if let Some(label) = staleness {
        header_col = header_col.push(label);
    }
    header_col = header_col.push(context_row);

    // Typeahead popup
    if typeahead.visible && !typeahead.items.is_empty() {
        let mut list = column![].spacing(0);
        for (i, item) in typeahead.items.iter().enumerate() {
            let is_selected = i == typeahead.selected;
            let style = if is_selected {
                theme::ButtonClass::Nav { active: true }
            } else {
                theme::ButtonClass::Action
            };
            let mut item_row = row![text(&item.label).size(TEXT_SM),]
                .spacing(SPACE_XS)
                .align_y(iced::Alignment::Center);
            if let Some(ref detail) = item.detail {
                item_row = item_row.push(
                    text(detail)
                        .size(TEXT_XS)
                        .style(theme::TextClass::Tertiary.style()),
                );
            }
            list = list.push(
                button(
                    container(item_row)
                        .padding(Padding::from([SPACE_XXS, SPACE_SM]))
                        .width(Length::Fill),
                )
                .on_press(ThreadListMessage::TypeaheadSelect(i))
                .padding(0)
                .style(style.style())
                .width(Length::Fill),
            );
        }
        header_col = header_col.push(
            container(list)
                .style(theme::ContainerClass::Elevated.style())
                .width(Length::Fill),
        );
    }

    container(header_col).padding(PAD_PANEL_HEADER).into()
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn thread_list_body(state: &ThreadList) -> Element<'_, ThreadListMessage> {
    let row_groups = state.row_groups();
    let group_headers =
        super::thread_date_groups::headers(row_groups.as_deref(), state.threads.len());
    let is_folded = |i: usize| {
        row_groups
            .as_ref()
            .and_then(|groups| groups.get(i))
            .is_some_and(|group| state.collapsed_groups.contains(group))
    };

    let mut list = column![].spacing(0);
    for (i, thread) in state.threads.iter().enumerate() {
        match group_headers.get(i).copied().flatten() {
            Some(group) => {
                list = list.push(date_group_header(group, is_folded(i)));
            }
            None if is_folded(i) => {}
            None if i > 0 => {
                list = list.push(
                    container(
                        iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                    )
                    .padding(PAD_THREAD_SEPARATOR),
                );
            }
            None => {}
        }
        if is_folded(i) {
            continue;
        }
        // Look up BIMI logo for the sender's domain.
        let bimi_logo = thread
            .from_address
            .as_deref()
            .and_then(|addr| addr.rsplit('@').next())
            .and_then(|domain| {
                // get() returns None on miss, Some(None) for negative cache,
                // Some(Some(path)) for a cached logo.
                match state.bimi_cache.get(domain) {
                    Some(Some(path)) if path.exists() => Some(path),
                    _ => None,
                }
            });
        let card = widgets::thread_card(
            thread,
            i,
            state.is_selected(i),
            &thread.label_paints,
            bimi_logo.as_deref(),
            ThreadListMessage::SelectThread,
            |index, action| ThreadListMessage::RowAction { index, action },
        );
        let mut row_el = super::anchored_overlay::anchored_overlay(card)
            .on_right_press_offset(move |offset| ThreadListMessage::OpenRowMenu { index: i, offset });
        if let Some(menu) = state.row_menu.as_ref().filter(|menu| menu.index == i) {
            row_el = row_el
                .popup(super::thread_menu::row_menu_card(menu))
                .anchor_offset(menu.offset)
                .popup_width(ROW_MENU_WIDTH)
                .on_dismiss(ThreadListMessage::CloseRowMenu);
        }
        list = list.push(row_el);
    }
    scrollable(list)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill)
        .into()
}

/// "Today", "Yesterday", ... with a chevron; a click folds the group.
fn date_group_header<'a>(group: DateGroup, folded: bool) -> Element<'a, ThreadListMessage> {
    let chevron = if folded {
        crate::icon::chevron_right()
    } else {
        crate::icon::chevron_down()
    };
    button(
        row![
            container(
                chevron
                    .size(ICON_XS)
                    .style(theme::TextClass::Tertiary.style())
            )
            .width(THREAD_GROUP_CHEVRON_SLOT)
            .align_x(iced::Alignment::Center),
            text(group.label())
                .size(TEXT_XS)
                .font(crate::font::text_semibold())
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS)
        .align_y(iced::Alignment::Center),
    )
    .on_press(ThreadListMessage::ToggleDateGroup(group))
    .padding(PAD_THREAD_GROUP_HEADER)
    .style(theme::ButtonClass::Ghost.style())
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(account_id: &str, id: &str) -> Thread {
        Thread {
            id: id.to_string(),
            account_id: account_id.to_string(),
            subject: None,
            snippet: None,
            last_message_at: None,
            message_count: 1,
            is_read: true,
            is_starred: false,
            is_replied: false,
            is_forwarded: false,
            is_pinned: false,
            is_muted: false,
            is_flag_complete: false,
            has_attachments: false,
            label_paints: Vec::new(),
            from_name: None,
            from_address: None,
            is_local_draft: false,
            match_kind: None,
            also_matched: Vec::new(),
        }
    }

    fn list(ids: &[&str]) -> Vec<Thread> {
        ids.iter().map(|id| thread("acc", id)).collect()
    }

    #[test]
    fn a_reload_keeps_the_selection_on_the_same_thread() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a", "b", "c", "d"]));
        threads.selected_thread = Some(2);
        threads.last_selected_anchor = Some(2);
        threads.selected_threads = HashSet::from([2, 3]);

        threads.set_threads(list(&["b", "c", "d"]));

        assert_eq!(threads.selected_thread, Some(1));
        assert_eq!(threads.last_selected_anchor, Some(1));
        assert_eq!(threads.selected_threads, HashSet::from([1, 2]));
    }

    #[test]
    fn a_row_action_selects_only_that_thread() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a", "b", "c"]));
        threads.selected_thread = Some(0);
        threads.selected_threads = HashSet::from([0, 1]);

        let (_, event) = threads.update(ThreadListMessage::RowAction {
            index: 2,
            action: widgets::ThreadRowAction::Trash,
        });

        assert_eq!(threads.selected_thread, Some(2));
        assert!(threads.selected_threads.is_empty());
        assert_eq!(
            event,
            Some(ThreadListEvent::RowAction {
                index: 2,
                action: widgets::ThreadRowAction::Trash,
            })
        );
    }

    #[test]
    fn a_row_action_on_a_missing_index_does_nothing() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a"]));
        threads.selected_thread = Some(0);

        let (_, event) = threads.update(ThreadListMessage::RowAction {
            index: 4,
            action: widgets::ThreadRowAction::Pin,
        });

        assert_eq!(threads.selected_thread, Some(0));
        assert_eq!(event, None);
    }

    #[test]
    fn a_thread_deleted_elsewhere_drops_out_of_the_selection() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a", "b", "c"]));
        threads.selected_thread = Some(1);
        threads.selected_threads = HashSet::from([1, 2]);

        threads.set_threads(list(&["a", "c"]));

        assert_eq!(threads.selected_thread, None);
        assert_eq!(threads.selected_threads, HashSet::from([1]));
    }

    #[test]
    fn the_same_thread_id_in_another_account_is_a_different_thread() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(vec![thread("one", "t"), thread("two", "t")]);
        threads.selected_thread = Some(1);

        threads.set_threads(vec![thread("two", "t"), thread("one", "t")]);

        assert_eq!(threads.selected_thread, Some(0));
    }

    fn open_menu(threads: &mut ThreadList, index: usize) -> Option<ThreadListEvent> {
        threads
            .update(ThreadListMessage::OpenRowMenu {
                index,
                offset: iced::Vector::new(10.0, 5.0),
            })
            .1
    }

    #[test]
    fn a_right_click_inside_the_selection_keeps_all_of_it() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a", "b", "c"]));
        threads.selected_thread = Some(0);
        threads.selected_threads = HashSet::from([0, 1]);

        let event = open_menu(&mut threads, 1);

        assert_eq!(threads.selected_threads, HashSet::from([0, 1]));
        assert_eq!(threads.row_menu.as_ref().map(|menu| menu.index), Some(1));
        assert_eq!(
            event,
            Some(ThreadListEvent::RowMenuOpened {
                index: 1,
                selection_changed: false,
            })
        );
    }

    #[test]
    fn a_right_click_outside_the_selection_selects_that_row_alone() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a", "b", "c"]));
        threads.selected_thread = Some(0);
        threads.selected_threads = HashSet::from([0, 1]);

        let event = open_menu(&mut threads, 2);

        assert_eq!(threads.selected_thread, Some(2));
        assert!(threads.selected_threads.is_empty());
        assert_eq!(
            event,
            Some(ThreadListEvent::RowMenuOpened {
                index: 2,
                selection_changed: true,
            })
        );
    }

    #[test]
    fn a_local_draft_has_no_right_click_menu() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        let mut draft = thread("acc", "draft");
        draft.is_local_draft = true;
        threads.set_threads(vec![thread("acc", "a"), draft]);
        threads.selected_thread = Some(0);

        assert_eq!(open_menu(&mut threads, 1), None);
        assert_eq!(threads.selected_thread, Some(0));
        assert!(threads.row_menu.is_none());
    }

    #[test]
    fn picking_an_entry_closes_the_menu_and_reports_it_once() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a"]));
        open_menu(&mut threads, 0);
        let (_, shown) = threads.update(ThreadListMessage::RowMenuShowPage(RowMenuPage::Snooze));
        assert_eq!(shown, None);
        assert_eq!(
            threads.row_menu.as_ref().map(|menu| menu.page),
            Some(RowMenuPage::Snooze)
        );

        let (_, event) = threads.update(ThreadListMessage::RowMenuPick(RowMenuItem::Archive));
        assert_eq!(event, Some(ThreadListEvent::RowMenuPicked(RowMenuItem::Archive)));
        assert!(threads.row_menu.is_none());

        let (_, again) = threads.update(ThreadListMessage::RowMenuPick(RowMenuItem::Archive));
        assert_eq!(again, None);
    }

    #[test]
    fn the_menu_follows_its_row_through_a_reload_and_closes_when_it_goes() {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        threads.set_threads(list(&["a", "b", "c"]));
        open_menu(&mut threads, 2);

        threads.set_threads(list(&["b", "c"]));
        assert_eq!(threads.row_menu.as_ref().map(|menu| menu.index), Some(1));

        threads.set_threads(list(&["b"]));
        assert!(threads.row_menu.is_none());
    }

    /// One pinned thread, then two undated ones in the "Older" group.
    fn pinned_then_older() -> ThreadList {
        let mut threads = ThreadList::new(Arc::new(rtsk::bimi::BimiLruCache::new()));
        let mut pinned = thread("acc", "pinned");
        pinned.is_pinned = true;
        threads.set_threads(vec![pinned, thread("acc", "b"), thread("acc", "c")]);
        threads
    }

    fn fold(threads: &mut ThreadList, group: DateGroup) -> Option<ThreadListEvent> {
        threads.update(ThreadListMessage::ToggleDateGroup(group)).1
    }

    #[test]
    fn the_keyboard_skips_a_folded_group() {
        let mut threads = pinned_then_older();
        fold(&mut threads, DateGroup::Older);

        let (_, first) = threads.update(ThreadListMessage::SelectNext);
        assert_eq!(first, Some(ThreadListEvent::ThreadSelected(0)));
        let (_, past_end) = threads.update(ThreadListMessage::SelectNext);
        assert_eq!(past_end, None);
        let (_, last) = threads.update(ThreadListMessage::SelectLast);
        assert_eq!(last, Some(ThreadListEvent::ThreadSelected(0)));

        fold(&mut threads, DateGroup::Older);
        let (_, unfolded) = threads.update(ThreadListMessage::SelectNext);
        assert_eq!(unfolded, Some(ThreadListEvent::ThreadSelected(1)));
    }

    #[test]
    fn folding_drops_hidden_rows_from_the_multi_selection() {
        let mut threads = pinned_then_older();
        threads.selected_thread = Some(0);
        threads.selected_threads = HashSet::from([0, 1, 2]);

        let event = fold(&mut threads, DateGroup::Older);

        assert_eq!(threads.selected_threads, HashSet::from([0]));
        assert_eq!(event, Some(ThreadListEvent::MultiSelectionChanged(1)));
    }

    #[test]
    fn folding_the_open_conversation_away_closes_it() {
        let mut threads = pinned_then_older();
        threads.selected_thread = Some(1);

        let event = fold(&mut threads, DateGroup::Older);

        assert_eq!(threads.selected_thread, None);
        assert_eq!(threads.selected_indices(), Vec::<usize>::new());
        assert_eq!(event, Some(ThreadListEvent::ThreadDeselected));
    }

    #[test]
    fn select_all_leaves_folded_rows_out() {
        let mut threads = pinned_then_older();
        fold(&mut threads, DateGroup::Pinned);

        let (_, _) = threads.update(ThreadListMessage::SelectAll);

        assert_eq!(threads.selected_threads, HashSet::from([1, 2]));
    }

    #[test]
    fn auto_advance_lands_on_a_shown_row() {
        let mut threads = pinned_then_older();
        fold(&mut threads, DateGroup::Older);
        threads.selected_thread = Some(1);

        let (_, event) = threads.update(ThreadListMessage::AutoAdvance);

        assert_eq!(event, Some(ThreadListEvent::AutoAdvance { new_index: Some(0) }));
    }

    #[test]
    fn without_date_headers_nothing_is_folded() {
        let mut threads = pinned_then_older();
        fold(&mut threads, DateGroup::Older);
        threads.date_groups = false;

        let (_, _) = threads.update(ThreadListMessage::SelectAll);

        assert_eq!(threads.selected_threads, HashSet::from([0, 1, 2]));
    }
}
