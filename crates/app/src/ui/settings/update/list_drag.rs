use iced::{Point, Task};

use crate::ui::layout::*;
use crate::ui::settings::types::*;

impl Settings {
    pub(super) fn handle_drag_move(
        &mut self,
        list_id: &str,
        point: Point,
    ) -> Task<SettingsMessage> {
        let has_drag = self
            .drag_state
            .as_ref()
            .is_some_and(|d| d.list_id == list_id);
        if !has_drag {
            return Task::none();
        }

        if let Some(ref mut drag) = self.drag_state
            && drag.start_y < 0.0
        {
            drag.start_y = point.y;
            return Task::none();
        }

        let Some(drag_ref) = self.drag_state.as_ref() else {
            return Task::none();
        };
        let (from, start_y) = (drag_ref.dragging_index, drag_ref.start_y);

        if !drag_ref.is_dragging {
            if (point.y - start_y).abs() < DRAG_START_THRESHOLD {
                return Task::none();
            }
            if let Some(ref mut drag) = self.drag_state {
                drag.is_dragging = true;
            }
        }

        // The only reorderable list is Settings > Labels > groups. The +1.0
        // accounts for the divider rule between rows.
        let row_step = SETTINGS_TOGGLE_ROW_HEIGHT + 1.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let raw_target = (point.y / row_step).max(0.0) as usize;

        let count = self.label_groups.len();
        if from >= count {
            return Task::none();
        }
        let target = raw_target.min(count.saturating_sub(1));
        if target != from {
            self.label_groups.swap(from, target);
            if let Some(ref mut drag) = self.drag_state {
                drag.dragging_index = target;
            }
        }
        Task::none()
    }
}
