use crate::app::{AppMode, DIVIDER_WIDTH, Divider, ReadyApp};
use crate::command_dispatch;
use crate::component::Component;
use crate::message::Message;
use crate::pop_out::{self, PopOutWindow};
use crate::ui;
use crate::ui::add_account::AddAccountWizard;
use crate::ui::layout::{
    APP_RAIL_WIDTH, PANE_GAP, READING_PANE_MIN_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH,
};
use cmdk::current_platform;
use iced::widget::{Space, column, container, mouse_area, row, stack};
use iced::{Element, Length, Padding, Point, Task};

impl ReadyApp {
    pub(crate) fn view(&self, window_id: iced::window::Id) -> Element<'_, Message> {
        if window_id == self.main_window_id {
            return self.view_main_window();
        }

        if let Some(pop_out) = self.pop_out_windows.get(&window_id) {
            return match pop_out {
                PopOutWindow::MessageView(state) => pop_out::message_view::view_message_window(
                    window_id,
                    state,
                    &self.thread_list.bimi_cache,
                ),
                PopOutWindow::Compose(state) => {
                    pop_out::compose::view_compose_window(window_id, state)
                }
                PopOutWindow::Calendar(_) => ui::calendar::calendar_layout(&self.calendar)
                    .map(|m| Message::Calendar(Box::new(m))),
            };
        }

        ui::widgets::empty_placeholder("Window not found", "")
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn view_main_window(&self) -> Element<'_, Message> {
        if let Some(ref wizard) = self.add_account_wizard {
            if self.no_accounts {
                return self.view_first_launch_modal(wizard);
            }
            return self.view_with_add_account_modal(wizard);
        }

        let layout = self.view_main_layout();

        let full_layout = layout;

        let main_layout: Element<'_, Message> = if self.dragging.is_some() {
            mouse_area(full_layout)
                .on_move(Message::DividerDragMove)
                .on_release(Message::DividerDragEnd)
                .interaction(iced::mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            full_layout.into()
        };

        if self.palette.is_open() {
            let palette_widget = self.palette.view().map(Message::Palette);

            let palette_positioned = container(palette_widget)
                .width(Length::Fill)
                .padding(iced::Padding {
                    top: ui::layout::PALETTE_TOP_OFFSET,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .align_x(iced::Alignment::Center);

            ui::modal_overlay::modal_overlay(
                main_layout,
                palette_positioned,
                ui::modal_overlay::ModalSurface::Modal,
                Message::Noop,
            )
        } else if let Some(ref pending) = self.pending_chord {
            let chord_display = pending.first.display(current_platform());
            let indicator = ui::palette::chord_indicator::<Message>(&chord_display);
            let indicator_positioned = container(indicator)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::Alignment::End);
            stack![main_layout, indicator_positioned].into()
        } else {
            main_layout
        }
    }

    pub(crate) fn view_first_launch_modal<'a>(
        &'a self,
        wizard: &'a AddAccountWizard,
    ) -> Element<'a, Message> {
        use ui::layout::{ACCOUNT_MODAL_MAX_HEIGHT, ACCOUNT_MODAL_WIDTH};

        let modal_content = wizard.view().map(Message::AddAccount);

        let modal = container(modal_content)
            .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
            .height(Length::Fit.max(ACCOUNT_MODAL_MAX_HEIGHT))
            .padding(ui::layout::PAD_SETTINGS_CONTENT)
            .style(ui::theme::ContainerClass::Elevated.style());

        container(modal)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(ui::theme::ContainerClass::Content.style())
            .into()
    }

    pub(crate) fn view_with_add_account_modal<'a>(
        &'a self,
        wizard: &'a AddAccountWizard,
    ) -> Element<'a, Message> {
        use ui::layout::{ACCOUNT_MODAL_MAX_HEIGHT, ACCOUNT_MODAL_WIDTH};

        let base_layout = self.view_main_layout();

        let modal_content = wizard.view().map(Message::AddAccount);

        // `DialogCard` matches the alert/form dialogs from `ui::dialog`:
        // opaque window-like surface with a soft drop shadow, so the
        // wizard reads as a peer of the GNOME-HIG-style confirmation
        // dialogs rather than a generic Elevated card.
        let modal = container(modal_content)
            .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
            .height(Length::Fit.max(ACCOUNT_MODAL_MAX_HEIGHT))
            .padding(ui::layout::PAD_SETTINGS_CONTENT)
            .style(ui::theme::ContainerClass::DialogCard.style());

        ui::modal_overlay::modal_overlay(
            base_layout,
            modal,
            ui::modal_overlay::ModalSurface::Modal,
            Message::Noop,
        )
    }

    /// The whole main window below any modal: title bar with search, app
    /// rail, and the floating panes for the current mode.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn view_main_layout(&self) -> Element<'_, Message> {
        let search = self.thread_list.search_bar().map(Message::ThreadList);
        let rail = ui::chrome::app_rail(self.app_mode, self.show_settings);

        let body: Element<'_, Message> = if self.show_settings {
            pane(self.settings.view().map(Message::Settings))
        } else {
            match self.app_mode {
                AppMode::Calendar => pane(
                    ui::calendar::calendar_layout(&self.calendar)
                        .map(|m| Message::Calendar(Box::new(m))),
                ),
                AppMode::Mail => self.view_mail_body(),
            }
        };

        let workspace = container(row![rail, body].height(Length::Fill))
            .padding(Padding {
                top: PANE_GAP,
                right: PANE_GAP,
                bottom: PANE_GAP,
                left: 0.0,
            })
            .width(Length::Fill)
            .height(Length::Fill);

        container(column![
            ui::chrome::title_bar(search),
            workspace,
            self.status_bar_view()
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(ui::theme::ContainerClass::Canvas.style())
        .into()
    }

    /// Mail mode: ribbon over folders, message list and reading pane (or
    /// the chat timeline when a chat is open).
    fn view_mail_body(&self) -> Element<'_, Message> {
        let ctx = command_dispatch::build_context(self);
        let ribbon = ui::chrome::ribbon(&self.registry, &self.binding_table, &ctx);
        let sidebar = container(pane(self.sidebar.view().map(Message::Sidebar)))
            .width(self.sidebar_width)
            .height(Length::Fill);
        let divider_sidebar = self.build_divider(Divider::Sidebar);

        let panes: Element<'_, Message> = if self.active_chat.is_some() {
            let chat_view: Element<'_, Message> = if let Some(ref timeline) = self.chat_timeline {
                timeline.view().map(Message::ChatTimeline)
            } else {
                container(
                    iced::widget::text("No chat selected")
                        .style(ui::theme::TextClass::Muted.style()),
                )
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into()
            };
            row![sidebar, divider_sidebar, pane(chat_view)]
                .height(Length::Fill)
                .into()
        } else {
            let thread_list = container(pane(self.thread_list.view().map(Message::ThreadList)))
                .width(self.thread_list_width)
                .height(Length::Fill);
            let divider_thread = self.build_divider(Divider::ThreadList);
            let reading_pane = pane(self.reading_pane.view_with_commands(
                &self.registry,
                &self.binding_table,
                &ctx,
            ));
            let rs_data = ui::right_sidebar::RightSidebarData {
                calendar: &self.calendar,
                threads: &self.thread_list.threads,
            };
            let right_sidebar = ui::right_sidebar::view(self.right_sidebar_open, &rs_data);
            row![
                sidebar,
                divider_sidebar,
                thread_list,
                divider_thread,
                reading_pane,
                right_sidebar
            ]
            .height(Length::Fill)
            .into()
        };

        column![ribbon, panes]
            .spacing(PANE_GAP)
            .height(Length::Fill)
            .into()
    }

    pub(crate) fn handle_divider_drag(&mut self, point: Point) -> Task<Message> {
        // Panes start right of the app rail and stop one gap short of the
        // window edge.
        let point = Point::new(point.x - APP_RAIL_WIDTH, point.y);
        // Available width for the three main panels (excludes right sidebar
        // when open, and both dividers).
        let right_sidebar_used = if self.right_sidebar_open {
            ui::layout::RIGHT_SIDEBAR_WIDTH
        } else {
            0.0
        };
        let available = self.window.width
            - APP_RAIL_WIDTH
            - PANE_GAP
            - 2.0 * DIVIDER_WIDTH
            - right_sidebar_used;

        match self.dragging {
            Some(Divider::Sidebar) => {
                // max: leave room for thread list min + reading pane min
                let max_sidebar = available - THREAD_LIST_MIN_WIDTH - READING_PANE_MIN_WIDTH;
                self.sidebar_width = point
                    .x
                    .clamp(SIDEBAR_MIN_WIDTH, max_sidebar.max(SIDEBAR_MIN_WIDTH));
            }
            Some(Divider::ThreadList) => {
                // max: leave room for reading pane min
                let max_thread_list = available - self.sidebar_width - READING_PANE_MIN_WIDTH;
                let new_width = (point.x - self.sidebar_width - DIVIDER_WIDTH).clamp(
                    THREAD_LIST_MIN_WIDTH,
                    max_thread_list.max(THREAD_LIST_MIN_WIDTH),
                );
                self.thread_list_width = new_width;
            }
            None => {}
        }
        Task::none()
    }

    /// Clamp sidebar and thread-list widths so that all three panels
    /// respect their minimums at the current window size.  Called after
    /// every main-window resize.
    pub(crate) fn clamp_panel_widths(&mut self) {
        let right_sidebar_used = if self.right_sidebar_open {
            ui::layout::RIGHT_SIDEBAR_WIDTH
        } else {
            0.0
        };
        let available = self.window.width
            - APP_RAIL_WIDTH
            - PANE_GAP
            - 2.0 * DIVIDER_WIDTH
            - right_sidebar_used;

        // 1. Ensure sidebar doesn't exceed what leaves room for the other
        //    two panels at their minimums.
        let max_sidebar =
            (available - THREAD_LIST_MIN_WIDTH - READING_PANE_MIN_WIDTH).max(SIDEBAR_MIN_WIDTH);
        self.sidebar_width = self.sidebar_width.clamp(SIDEBAR_MIN_WIDTH, max_sidebar);

        // 2. Ensure thread list doesn't exceed what leaves room for the
        //    reading pane at its minimum.
        let max_thread_list =
            (available - self.sidebar_width - READING_PANE_MIN_WIDTH).max(THREAD_LIST_MIN_WIDTH);
        self.thread_list_width = self
            .thread_list_width
            .clamp(THREAD_LIST_MIN_WIDTH, max_thread_list);
    }

    pub(crate) fn build_divider(&self, divider: Divider) -> Element<'_, Message> {
        let class = if self.hovered_divider == Some(divider) || self.dragging == Some(divider) {
            ui::theme::ContainerClass::DividerHover
        } else {
            ui::theme::ContainerClass::Canvas
        };
        mouse_area(
            container("")
                .width(DIVIDER_WIDTH)
                .height(Length::Fill)
                .style(class.style()),
        )
        .on_press(Message::DividerDragStart(divider))
        .on_release(Message::DividerDragEnd)
        .on_enter(Message::DividerHover(divider))
        .on_exit(Message::DividerUnhover)
        .interaction(iced::mouse::Interaction::ResizingHorizontally)
        .into()
    }

    /// Render the status bar, respecting the `sync_status_bar` setting.
    /// When the setting is off, returns an empty zero-height element.
    pub(crate) fn status_bar_view(&self) -> Element<'_, Message> {
        if self.settings.sync_status_bar {
            self.status_bar
                .view_with_system_status(&self.service_health, self.index_rebuild_progress.as_ref())
                .map(Message::StatusBar)
        } else {
            Space::new().width(0).height(0).into()
        }
    }
}

/// Wraps a pane's content in the rounded card that floats on the canvas.
fn pane<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(ui::theme::ContainerClass::Pane.style())
        .into()
}
