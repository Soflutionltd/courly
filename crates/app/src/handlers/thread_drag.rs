//! Dragging messages from the list onto a sidebar folder.

use iced::{Point, Task};
use service_api::actions::FolderId;

use crate::action_resolve::MailActionIntent;
use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::sidebar::DropFolder;
use crate::ui::thread_list::ThreadListMessage;

/// Movement below this many pixels is a click, not a drag.
const DRAG_THRESHOLD: f32 = 4.0;

/// A thread, by account id and thread id: indices shift when sync
/// reloads the list mid-drag.
type ThreadKey = (String, String);

/// A left press on a row, followed until release.
#[derive(Debug, Clone)]
pub(crate) struct ThreadDrag {
    pressed: ThreadKey,
    origin: Option<Point>,
    cursor: Option<Point>,
    /// The dragged messages, once the pointer has moved far enough.
    threads: Vec<ThreadKey>,
}

/// What follows the pointer while messages are dragged.
pub(crate) struct DragGhost<'a> {
    pub count: usize,
    /// (account id, thread id) of the row the drag started on.
    pub pressed: (&'a str, &'a str),
    pub cursor: Point,
}

impl ThreadDrag {
    pub(crate) fn ghost(&self) -> Option<DragGhost<'_>> {
        if self.threads.is_empty() {
            return None;
        }
        Some(DragGhost {
            count: self.threads.len(),
            pressed: (&self.pressed.0, &self.pressed.1),
            cursor: self.cursor?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ThreadDragMessage {
    Moved(Point),
    Released,
    Cancel,
}

impl ReadyApp {
    pub(crate) fn start_thread_drag(&mut self, index: usize) {
        self.thread_drag = self
            .thread_list
            .threads
            .get(index)
            .filter(|thread| !thread.is_local_draft)
            .map(|thread| ThreadDrag {
                pressed: (thread.account_id.clone(), thread.id.clone()),
                origin: None,
                cursor: None,
                threads: Vec::new(),
            });
    }

    pub(crate) fn handle_thread_drag(&mut self, message: ThreadDragMessage) -> Task<Message> {
        match message {
            ThreadDragMessage::Moved(position) => {
                let Some(drag) = self.thread_drag.as_mut() else {
                    return Task::none();
                };
                drag.cursor = Some(position);
                let origin = *drag.origin.get_or_insert(position);
                if drag.threads.is_empty() && origin.distance(position) > DRAG_THRESHOLD {
                    self.begin_thread_drag();
                }
                Task::none()
            }
            ThreadDragMessage::Released => self.drop_threads(),
            ThreadDragMessage::Cancel => {
                self.end_thread_drag();
                Task::none()
            }
        }
    }

    /// The selection moves when the pressed row is part of it; any other
    /// row moves alone, as in Outlook.
    pub(crate) fn begin_thread_drag(&mut self) {
        let selected = self.selected_thread_keys();
        let Some(drag) = self.thread_drag.as_mut().filter(|d| d.threads.is_empty()) else {
            return;
        };
        drag.threads = if selected.contains(&drag.pressed) {
            selected
        } else {
            vec![drag.pressed.clone()]
        };
        let mut accounts: Vec<String> = drag.threads.iter().map(|(a, _)| a.clone()).collect();
        accounts.sort_unstable();
        accounts.dedup();
        self.thread_list.cancel_press();
        self.sidebar.start_message_drop(accounts);
    }

    fn end_thread_drag(&mut self) -> Option<(ThreadDrag, Option<DropFolder>)> {
        let drag = self.thread_drag.take()?;
        let target = self
            .sidebar
            .message_drop
            .take()
            .and_then(|drop| drop.target);
        Some((drag, target))
    }

    fn drop_threads(&mut self) -> Task<Message> {
        let Some((drag, Some(target))) = self.end_thread_drag() else {
            return Task::none();
        };
        if drag.threads.is_empty()
            || self
                .sidebar
                .selected_folder()
                .is_some_and(|folder| folder.folder_id == target.folder_id)
        {
            return Task::none();
        }
        // The move acts on the selection: a row dragged from outside it
        // becomes the selection first.
        let select = if drag.threads == self.selected_thread_keys() {
            Task::none()
        } else {
            let Some(index) = self
                .thread_list
                .threads
                .iter()
                .position(|t| t.account_id == drag.pressed.0 && t.id == drag.pressed.1)
            else {
                return Task::none();
            };
            self.handle_thread_list(ThreadListMessage::SelectThread(index))
        };
        let action = self.update(Message::EmailAction(MailActionIntent::move_to(
            FolderId::from(target.folder_id),
        )));
        Task::batch([select, action])
    }

    fn selected_thread_keys(&self) -> Vec<ThreadKey> {
        self.thread_list
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.thread_list.threads.get(index))
            .map(|thread| (thread.account_id.clone(), thread.id.clone()))
            .collect()
    }
}
