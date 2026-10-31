//! The command toolbar above the mail panes, arranged by the user as in
//! Outlook: buttons that don't fit go under "…", a right-click removes
//! one, and the customize panel adds, reorders and removes them by drag.

mod view;

pub use view::toolbar;

use cmdk::CommandId;
use iced::{Point, Vector};
use serde::{Deserialize, Serialize};

use crate::icon;

/// A button the toolbar can hold. Stored by name in `toolbar_layout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarItem {
    Delete,
    Archive,
    Junk,
    Move,
    Flag,
    ReadUnread,
    Snooze,
    Reply,
    ReplyAll,
    Forward,
    Pin,
    Mute,
    Categorize,
    Sync,
    Undo,
    Unsubscribe,
}

impl ToolbarItem {
    /// Every button, in the customize panel's order.
    pub const ALL: [Self; 16] = [
        Self::Delete,
        Self::Archive,
        Self::Junk,
        Self::Move,
        Self::Flag,
        Self::ReadUnread,
        Self::Snooze,
        Self::Reply,
        Self::ReplyAll,
        Self::Forward,
        Self::Pin,
        Self::Mute,
        Self::Categorize,
        Self::Sync,
        Self::Undo,
        Self::Unsubscribe,
    ];

    pub fn command(self) -> CommandId {
        match self {
            Self::Delete => CommandId::EmailTrash,
            Self::Archive => CommandId::EmailArchive,
            Self::Junk => CommandId::EmailSpam,
            Self::Move => CommandId::EmailMoveToFolder,
            Self::Flag => CommandId::EmailStar,
            Self::ReadUnread => CommandId::EmailMarkRead,
            Self::Snooze => CommandId::EmailSnooze,
            Self::Reply => CommandId::ComposeReply,
            Self::ReplyAll => CommandId::ComposeReplyAll,
            Self::Forward => CommandId::ComposeForward,
            Self::Pin => CommandId::EmailPin,
            Self::Mute => CommandId::EmailMute,
            Self::Categorize => CommandId::EmailAddLabel,
            Self::Sync => CommandId::AppSyncFolder,
            Self::Undo => CommandId::Undo,
            Self::Unsubscribe => CommandId::EmailUnsubscribe,
        }
    }

    fn icon<'a>(self) -> iced::widget::Text<'a> {
        match self {
            Self::Delete => icon::trash(),
            Self::Archive => icon::archive(),
            Self::Junk => icon::shield(),
            Self::Move => icon::folder(),
            Self::Flag => icon::flag(),
            Self::ReadUnread => icon::mail(),
            Self::Snooze => icon::clock(),
            Self::Reply => icon::reply(),
            Self::ReplyAll => icon::reply_all(),
            Self::Forward => icon::forward(),
            Self::Pin => icon::pin(),
            Self::Mute => icon::bell(),
            Self::Categorize => icon::tag(),
            Self::Sync => icon::refresh(),
            Self::Undo => icon::undo(),
            Self::Unsubscribe => icon::x(),
        }
    }
}

/// Whether the buttons show their name next to the icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelMode {
    #[default]
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarLayout {
    pub items: Vec<ToolbarItem>,
    pub labels: LabelMode,
}

impl Default for ToolbarLayout {
    fn default() -> Self {
        Self {
            items: vec![
                ToolbarItem::Delete,
                ToolbarItem::Archive,
                ToolbarItem::Junk,
                ToolbarItem::Move,
                ToolbarItem::Flag,
                ToolbarItem::ReadUnread,
                ToolbarItem::Snooze,
                ToolbarItem::Reply,
                ToolbarItem::ReplyAll,
                ToolbarItem::Forward,
            ],
            labels: LabelMode::Always,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredLayout {
    items: Vec<serde_json::Value>,
    #[serde(default)]
    labels: LabelMode,
}

impl ToolbarLayout {
    /// Read the stored layout. Names this version doesn't know (a button
    /// from a newer release) are skipped rather than failing the whole bar.
    pub fn parse(json: Option<&str>) -> Self {
        let Some(json) = json else {
            return Self::default();
        };
        let stored: StoredLayout = match serde_json::from_str(json) {
            Ok(stored) => stored,
            Err(e) => {
                log::warn!("toolbar_layout unreadable, using the default toolbar: {e}");
                return Self::default();
            }
        };
        let mut items = Vec::new();
        for value in stored.items {
            if let Ok(item) = serde_json::from_value::<ToolbarItem>(value)
                && !items.contains(&item)
            {
                items.push(item);
            }
        }
        Self {
            items,
            labels: stored.labels,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::json!({ "items": self.items, "labels": self.labels }).to_string()
    }

    /// Put `item` at `target`, moving it if it is already on the bar.
    fn place(&mut self, item: ToolbarItem, target: DropTarget) {
        let mut index = match target {
            DropTarget::Before(index) => index,
            DropTarget::End => self.items.len(),
        };
        if let Some(current) = self.items.iter().position(|&other| other == item) {
            self.items.remove(current);
            if current < index {
                index -= 1;
            }
        }
        self.items.insert(index.min(self.items.len()), item);
    }
}

/// Where a drag started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragSource {
    /// A button on the bar, by position.
    Bar(usize),
    /// A tile of the customize panel.
    Catalog(ToolbarItem),
}

/// Where a dragged button would land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    Before(usize),
    End,
}

/// Movement below this many pixels is a click, not a drag.
const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarDrag {
    pub source: DragSource,
    pub target: Option<DropTarget>,
    pub cursor: Option<Point>,
    origin: Option<Point>,
    pub moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolbarMenu {
    /// Right-click on a button, `offset` from its corner.
    Item { index: usize, offset: Vector },
    /// Right-click on empty toolbar space, at this window position.
    Background(Point),
}

#[derive(Debug, Default)]
pub struct Toolbar {
    pub layout: ToolbarLayout,
    pub overflow_open: bool,
    pub menu: Option<ToolbarMenu>,
    pub customizing: bool,
    pub drag: Option<ToolbarDrag>,
    /// Last pointer position while customizing, so a drag measures its
    /// movement from the press rather than from the first move after it.
    pointer: Option<Point>,
}

#[derive(Debug, Clone, Copy)]
pub enum ToolbarMessage {
    ToggleOverflow,
    CloseOverflow,
    /// A button picked from the "…" menu.
    RunFromOverflow(CommandId),
    OpenItemMenu {
        index: usize,
        offset: Vector,
    },
    OpenBackgroundMenu(Point),
    CloseMenu,
    Remove(usize),
    Customize,
    Done,
    Reset,
    SetLabels(LabelMode),
    DragStart(DragSource),
    DragOver(DropTarget),
    DragLeave(DropTarget),
    CursorMoved(Point),
    DragEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolbarEvent {
    Execute(CommandId),
    /// The arrangement changed; carries the `toolbar_layout` JSON.
    LayoutChanged(String),
}

impl Toolbar {
    pub fn update(&mut self, message: ToolbarMessage) -> Option<ToolbarEvent> {
        match message {
            ToolbarMessage::ToggleOverflow => {
                self.overflow_open = !self.overflow_open;
                self.menu = None;
                None
            }
            ToolbarMessage::CloseOverflow => {
                self.overflow_open = false;
                None
            }
            ToolbarMessage::RunFromOverflow(id) => {
                self.overflow_open = false;
                Some(ToolbarEvent::Execute(id))
            }
            ToolbarMessage::OpenItemMenu { index, offset } => {
                if self.customizing || index >= self.layout.items.len() {
                    return None;
                }
                self.overflow_open = false;
                self.menu = Some(ToolbarMenu::Item { index, offset });
                None
            }
            ToolbarMessage::OpenBackgroundMenu(point) => {
                if self.customizing {
                    return None;
                }
                self.overflow_open = false;
                self.menu = Some(ToolbarMenu::Background(point));
                None
            }
            ToolbarMessage::CloseMenu => {
                self.menu = None;
                None
            }
            ToolbarMessage::Remove(index) => {
                self.menu = None;
                if index >= self.layout.items.len() {
                    return None;
                }
                self.layout.items.remove(index);
                self.changed()
            }
            ToolbarMessage::Customize => {
                self.menu = None;
                self.overflow_open = false;
                self.customizing = true;
                None
            }
            ToolbarMessage::Done => {
                self.customizing = false;
                self.drag = None;
                None
            }
            ToolbarMessage::Reset => {
                if self.layout == ToolbarLayout::default() {
                    return None;
                }
                self.layout = ToolbarLayout::default();
                self.changed()
            }
            ToolbarMessage::SetLabels(labels) => {
                if self.layout.labels == labels {
                    return None;
                }
                self.layout.labels = labels;
                self.changed()
            }
            ToolbarMessage::DragStart(source) => {
                if !self.customizing {
                    return None;
                }
                // A press on a bar button is already over its own slot,
                // which only reports entering once the pointer moves.
                let target = match source {
                    DragSource::Bar(index) => Some(DropTarget::Before(index)),
                    DragSource::Catalog(_) => None,
                };
                self.drag = Some(ToolbarDrag {
                    source,
                    target,
                    cursor: None,
                    origin: self.pointer,
                    moved: false,
                });
                None
            }
            ToolbarMessage::DragOver(target) => {
                if let Some(drag) = self.drag.as_mut() {
                    drag.target = Some(target);
                }
                None
            }
            ToolbarMessage::DragLeave(target) => {
                if let Some(drag) = self.drag.as_mut()
                    && drag.target == Some(target)
                {
                    drag.target = None;
                }
                None
            }
            ToolbarMessage::CursorMoved(point) => {
                self.pointer = Some(point);
                if let Some(drag) = self.drag.as_mut() {
                    let origin = *drag.origin.get_or_insert(point);
                    drag.moved |= point.distance(origin) > DRAG_THRESHOLD;
                    drag.cursor = Some(point);
                }
                None
            }
            ToolbarMessage::DragEnd => self.drop_dragged(),
        }
    }

    /// Finish a drag: onto the bar places the button, anywhere else takes
    /// a bar button off (a panel tile dropped outside is just let go).
    fn drop_dragged(&mut self) -> Option<ToolbarEvent> {
        let drag = self.drag.take()?;
        if !drag.moved {
            return None;
        }
        let before = self.layout.items.clone();
        match (drag.source, drag.target) {
            (DragSource::Bar(index), Some(target)) => {
                let item = *self.layout.items.get(index)?;
                self.layout.place(item, target);
            }
            (DragSource::Bar(index), None) => {
                if index < self.layout.items.len() {
                    self.layout.items.remove(index);
                }
            }
            (DragSource::Catalog(item), Some(target)) => self.layout.place(item, target),
            (DragSource::Catalog(_), None) => {}
        }
        if self.layout.items == before {
            return None;
        }
        self.changed()
    }

    fn changed(&mut self) -> Option<ToolbarEvent> {
        self.menu = None;
        Some(ToolbarEvent::LayoutChanged(self.layout.to_json()))
    }

    /// Escape closes the innermost thing open. Returns whether it did.
    pub fn escape(&mut self) -> bool {
        if self.drag.take().is_some() {
            return true;
        }
        if self.menu.take().is_some() {
            return true;
        }
        if self.overflow_open {
            self.overflow_open = false;
            return true;
        }
        if self.customizing {
            self.customizing = false;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ToolbarItem::{Archive, Delete, Flag, Junk, Move, Pin};

    fn bar(items: &[ToolbarItem]) -> Toolbar {
        Toolbar {
            layout: ToolbarLayout {
                items: items.to_vec(),
                labels: LabelMode::Always,
            },
            customizing: true,
            ..Toolbar::default()
        }
    }

    fn drag(
        toolbar: &mut Toolbar,
        source: DragSource,
        over: Option<DropTarget>,
    ) -> Option<ToolbarEvent> {
        toolbar.update(ToolbarMessage::DragStart(source));
        toolbar.update(ToolbarMessage::CursorMoved(Point::new(10.0, 10.0)));
        toolbar.update(ToolbarMessage::CursorMoved(Point::new(60.0, 12.0)));
        if let Some(target) = over {
            toolbar.update(ToolbarMessage::DragOver(target));
        } else if let DragSource::Bar(index) = source {
            toolbar.update(ToolbarMessage::DragLeave(DropTarget::Before(index)));
        }
        toolbar.update(ToolbarMessage::DragEnd)
    }

    #[test]
    fn dragging_a_button_right_reorders_the_bar() {
        let mut toolbar = bar(&[Delete, Archive, Junk, Move]);
        let event = drag(
            &mut toolbar,
            DragSource::Bar(0),
            Some(DropTarget::Before(3)),
        );
        assert_eq!(toolbar.layout.items, vec![Archive, Junk, Delete, Move]);
        assert!(matches!(event, Some(ToolbarEvent::LayoutChanged(_))));
    }

    #[test]
    fn dragging_a_button_left_and_to_the_end() {
        let mut toolbar = bar(&[Delete, Archive, Junk]);
        drag(
            &mut toolbar,
            DragSource::Bar(2),
            Some(DropTarget::Before(0)),
        );
        assert_eq!(toolbar.layout.items, vec![Junk, Delete, Archive]);
        drag(&mut toolbar, DragSource::Bar(0), Some(DropTarget::End));
        assert_eq!(toolbar.layout.items, vec![Delete, Archive, Junk]);
    }

    #[test]
    fn a_button_dropped_off_the_bar_is_removed() {
        let mut toolbar = bar(&[Delete, Archive, Junk]);
        let event = drag(&mut toolbar, DragSource::Bar(1), None);
        assert_eq!(toolbar.layout.items, vec![Delete, Junk]);
        assert!(event.is_some());
    }

    #[test]
    fn a_click_on_a_bar_button_while_customizing_changes_nothing() {
        let mut toolbar = bar(&[Delete, Archive]);
        toolbar.update(ToolbarMessage::DragStart(DragSource::Bar(0)));
        toolbar.update(ToolbarMessage::CursorMoved(Point::new(10.0, 10.0)));
        toolbar.update(ToolbarMessage::CursorMoved(Point::new(12.0, 11.0)));
        toolbar.update(ToolbarMessage::DragLeave(DropTarget::Before(0)));
        assert_eq!(toolbar.update(ToolbarMessage::DragEnd), None);
        assert_eq!(toolbar.layout.items, vec![Delete, Archive]);
    }

    #[test]
    fn a_pointer_that_jumps_in_one_event_still_drags() {
        let mut toolbar = bar(&[Delete, Archive, Junk]);
        toolbar.update(ToolbarMessage::CursorMoved(Point::new(80.0, 10.0)));
        toolbar.update(ToolbarMessage::DragStart(DragSource::Bar(1)));
        toolbar.update(ToolbarMessage::DragLeave(DropTarget::Before(1)));
        toolbar.update(ToolbarMessage::DragOver(DropTarget::Before(0)));
        toolbar.update(ToolbarMessage::CursorMoved(Point::new(5.0, 10.0)));
        assert!(toolbar.update(ToolbarMessage::DragEnd).is_some());
        assert_eq!(toolbar.layout.items, vec![Archive, Delete, Junk]);
    }

    #[test]
    fn a_panel_tile_adds_its_button_where_it_is_dropped() {
        let mut toolbar = bar(&[Delete, Archive]);
        drag(
            &mut toolbar,
            DragSource::Catalog(Pin),
            Some(DropTarget::Before(1)),
        );
        assert_eq!(toolbar.layout.items, vec![Delete, Pin, Archive]);

        // Already on the bar: the tile moves it instead of doubling it.
        drag(
            &mut toolbar,
            DragSource::Catalog(Delete),
            Some(DropTarget::End),
        );
        assert_eq!(toolbar.layout.items, vec![Pin, Archive, Delete]);

        assert_eq!(drag(&mut toolbar, DragSource::Catalog(Flag), None), None);
        assert_eq!(toolbar.layout.items, vec![Pin, Archive, Delete]);
    }

    #[test]
    fn dragging_needs_the_customize_panel() {
        let mut toolbar = bar(&[Delete, Archive]);
        toolbar.customizing = false;
        assert_eq!(drag(&mut toolbar, DragSource::Bar(0), None), None);
        assert_eq!(toolbar.layout.items, vec![Delete, Archive]);
    }

    #[test]
    fn right_click_remove_takes_the_button_off() {
        let mut toolbar = bar(&[Delete, Archive]);
        toolbar.customizing = false;
        toolbar.update(ToolbarMessage::OpenItemMenu {
            index: 1,
            offset: Vector::new(4.0, 4.0),
        });
        let event = toolbar.update(ToolbarMessage::Remove(1));
        assert_eq!(toolbar.layout.items, vec![Delete]);
        assert!(toolbar.menu.is_none());
        assert!(event.is_some());
    }

    #[test]
    fn the_layout_survives_a_round_trip_and_skips_unknown_buttons() {
        let layout = ToolbarLayout {
            items: vec![Pin, Delete],
            labels: LabelMode::Never,
        };
        assert_eq!(ToolbarLayout::parse(Some(&layout.to_json())), layout);

        let newer = r#"{"items":["delete","teleport","delete","pin"],"labels":"never"}"#;
        assert_eq!(
            ToolbarLayout::parse(Some(newer)),
            layout_of(&[Delete, Pin], LabelMode::Never)
        );
        assert_eq!(
            ToolbarLayout::parse(Some("not json")),
            ToolbarLayout::default()
        );
        assert_eq!(ToolbarLayout::parse(None), ToolbarLayout::default());
    }

    fn layout_of(items: &[ToolbarItem], labels: LabelMode) -> ToolbarLayout {
        ToolbarLayout {
            items: items.to_vec(),
            labels,
        }
    }

    #[test]
    fn reset_restores_the_default_bar() {
        let mut toolbar = bar(&[Pin]);
        assert!(toolbar.update(ToolbarMessage::Reset).is_some());
        assert_eq!(toolbar.layout, ToolbarLayout::default());
        assert_eq!(toolbar.update(ToolbarMessage::Reset), None);
    }

    #[test]
    fn every_button_runs_a_registered_command() {
        let registry = cmdk::CommandRegistry::new();
        for item in ToolbarItem::ALL {
            let desc = registry.get(item.command());
            assert!(desc.is_some(), "{item:?} has no registered command");
            assert!(
                desc.is_some_and(|desc| !desc.label.is_empty()),
                "{item:?} has no label"
            );
        }
    }
}
