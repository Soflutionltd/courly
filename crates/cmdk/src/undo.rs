//! Generic undo and redo stacks.
//!
//! The palette crate owns the stack structure. The payload type is
//! defined by the consumer (e.g., the app crate defines `MailUndoPayload`).
//! Each entry carries a description string and one or more payloads
//! (C4: one user action = one undo step, even for mixed-direction toggles),
//! plus what it takes to perform the action again once it was undone.

use std::collections::VecDeque;

/// A single undo entry: description + compensation payloads + redo.
///
/// `payloads` is a `Vec<T>` because one user action (e.g., toggle star
/// on 5 threads with mixed prior state) may produce multiple compensation
/// items that should all execute on a single Ctrl+Z.
#[derive(Debug, Clone)]
pub struct UndoEntry<T, R = ()> {
    /// Human-readable description (e.g., "Archived", "Star toggled").
    /// Set at push time (C3), not derived from the payload.
    pub description: String,
    /// Compensation payloads to execute on undo.
    pub payloads: Vec<T>,
    /// What performs the original action again, for redo.
    pub redo: R,
}

/// Bounded FIFO stack of undo entries, with the entries undone so far
/// kept for redo.
///
/// When the stack is full, the oldest entry is evicted. A new action
/// forgets everything that could be redone, as in every editor.
pub struct UndoStack<T, R = ()> {
    entries: VecDeque<UndoEntry<T, R>>,
    undone: Vec<UndoEntry<T, R>>,
    capacity: usize,
}

impl<T, R> UndoStack<T, R> {
    /// Create a new undo stack with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            undone: Vec::new(),
            capacity,
        }
    }

    /// Record a new action: it can be undone, and nothing undone before it
    /// can be redone any more.
    pub fn push_action(&mut self, description: String, payloads: Vec<T>, redo: R) {
        self.undone.clear();
        self.push_entry(UndoEntry {
            description,
            payloads,
            redo,
        });
    }

    /// Record an action performed again by redo: it can be undone, and
    /// the rest of the redo list stays.
    pub fn push_redone(&mut self, description: String, payloads: Vec<T>, redo: R) {
        self.push_entry(UndoEntry {
            description,
            payloads,
            redo,
        });
    }

    fn push_entry(&mut self, entry: UndoEntry<T, R>) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Pop the most recent entry (for undo).
    pub fn pop(&mut self) -> Option<UndoEntry<T, R>> {
        self.entries.pop_back()
    }

    /// Keep an entry that was just undone, so it can be redone.
    pub fn stash_undone(&mut self, entry: UndoEntry<T, R>) {
        if self.undone.len() >= self.capacity {
            self.undone.remove(0);
        }
        self.undone.push(entry);
    }

    /// Take the most recently undone entry (for redo).
    pub fn pop_undone(&mut self) -> Option<UndoEntry<T, R>> {
        self.undone.pop()
    }

    /// Whether there are any entries to undo.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether an undone entry can be redone.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Number of entries in the stack.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Clear all entries, including the ones kept for redo.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.undone.clear();
    }

    /// Peek at the most recent entry without removing it.
    pub fn peek(&self) -> Option<&UndoEntry<T, R>> {
        self.entries.back()
    }
}

impl<T> UndoStack<T, ()> {
    /// Push an entry that has no redo, evicting the oldest if at capacity.
    pub fn push(&mut self, description: String, payloads: Vec<T>) {
        self.push_action(description, payloads, ());
    }
}

impl<T, R> Default for UndoStack<T, R> {
    fn default() -> Self {
        Self::new(20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undone_entry_can_be_redone_once() {
        let mut stack: UndoStack<u8, &str> = UndoStack::default();
        stack.push_action("Archived".into(), vec![1], "archive");
        let entry = stack.pop().expect("undo");
        stack.stash_undone(entry);
        assert!(stack.can_redo());
        let redo = stack.pop_undone().expect("redo");
        assert_eq!(redo.redo, "archive");
        assert!(!stack.can_redo());
    }

    #[test]
    fn a_new_action_forgets_what_could_be_redone() {
        let mut stack: UndoStack<u8, &str> = UndoStack::default();
        stack.push_action("Archived".into(), vec![1], "archive");
        let entry = stack.pop().expect("undo");
        stack.stash_undone(entry);
        stack.push_action("Starred".into(), vec![2], "star");
        assert!(!stack.can_redo());
    }

    #[test]
    fn a_redone_action_keeps_the_rest_of_the_redo_list() {
        let mut stack: UndoStack<u8, &str> = UndoStack::default();
        stack.push_action("A".into(), vec![1], "a");
        stack.push_action("B".into(), vec![2], "b");
        for _ in 0..2 {
            let entry = stack.pop().expect("undo");
            stack.stash_undone(entry);
        }
        let first = stack.pop_undone().expect("redo a");
        assert_eq!(first.redo, "a");
        stack.push_redone(first.description, first.payloads, first.redo);
        assert!(stack.can_redo(), "b can still be redone");
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn the_oldest_entry_is_evicted_at_capacity() {
        let mut stack: UndoStack<u8> = UndoStack::new(2);
        stack.push("1".into(), vec![1]);
        stack.push("2".into(), vec![2]);
        stack.push("3".into(), vec![3]);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.peek().map(|e| e.description.as_str()), Some("3"));
    }
}
