use iced::Task;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::updater::{self, InstallEvent, Phase, UpdaterMessage};

impl ReadyApp {
    pub(crate) fn handle_updater(&mut self, msg: UpdaterMessage) -> Task<Message> {
        match msg {
            UpdaterMessage::Check { forced } => self.start_update_check(forced),
            UpdaterMessage::Checked(result) => self.handle_update_checked(result),
            UpdaterMessage::Install => self.start_update_install(),
            UpdaterMessage::Later => {
                if let Phase::Available(info) | Phase::Failed { info, .. } = &self.updater.phase {
                    self.updater.dismissed_version = Some(info.version.clone());
                    self.updater.phase = Phase::Idle;
                }
                Task::none()
            }
            UpdaterMessage::Event(event) => self.handle_install_event(event),
            UpdaterMessage::Retry => {
                if matches!(self.updater.phase, Phase::Failed { .. }) {
                    self.start_update_install()
                } else {
                    Task::none()
                }
            }
            UpdaterMessage::Quit => {
                if matches!(self.updater.phase, Phase::Installed(_)) {
                    self.handle_window_close(self.main_window_id)
                } else {
                    Task::none()
                }
            }
        }
    }

    fn start_update_check(&mut self, forced: bool) -> Task<Message> {
        let state = &mut self.updater;
        let busy = matches!(
            state.phase,
            Phase::Downloading { .. }
                | Phase::Installing(_)
                | Phase::Restarting(_)
                | Phase::Installed(_)
        );
        let throttled = state
            .last_check
            .is_some_and(|at| at.elapsed() < updater::FOCUS_CHECK_THROTTLE);
        if state.checking || busy || (!forced && throttled) {
            return Task::none();
        }
        state.checking = true;
        state.last_check = Some(std::time::Instant::now());
        Task::perform(updater::check(), |result| {
            Message::Updater(UpdaterMessage::Checked(result))
        })
    }

    fn handle_update_checked(
        &mut self,
        result: Result<Option<updater::UpdateInfo>, String>,
    ) -> Task<Message> {
        self.updater.checking = false;
        let info = match result {
            Ok(Some(info)) => info,
            Ok(None) => {
                if matches!(self.updater.phase, Phase::Available(_)) {
                    self.updater.phase = Phase::Idle;
                }
                return Task::none();
            }
            Err(error) => {
                log::info!("update check failed: {error}");
                return Task::none();
            }
        };
        if !matches!(self.updater.phase, Phase::Idle | Phase::Available(_)) {
            return Task::none();
        }
        if self.updater.dismissed_version.as_deref() == Some(info.version.as_str()) {
            return Task::none();
        }
        log::info!(
            "update available: {} -> {}",
            updater::current_version(),
            info.version
        );
        self.updater.attempts = 0;
        self.updater.phase = Phase::Available(info);
        if self.settings.auto_update {
            return self.start_update_install();
        }
        Task::none()
    }

    fn start_update_install(&mut self) -> Task<Message> {
        let info = match &self.updater.phase {
            Phase::Available(info) | Phase::Failed { info, .. } => info.clone(),
            _ => return Task::none(),
        };
        self.updater.phase = Phase::Downloading {
            info: info.clone(),
            downloaded: 0,
            total: None,
        };
        Task::stream(iced::stream::channel(
            32,
            move |output: iced::futures::channel::mpsc::Sender<InstallEvent>| {
                updater::install(info, output)
            },
        ))
        .map(|event| Message::Updater(UpdaterMessage::Event(event)))
    }

    fn handle_install_event(&mut self, event: InstallEvent) -> Task<Message> {
        let info = match &self.updater.phase {
            Phase::Downloading { info, .. } | Phase::Installing(info) => info.clone(),
            _ => return Task::none(),
        };
        match event {
            InstallEvent::Progress { downloaded, total } => {
                self.updater.phase = Phase::Downloading {
                    info,
                    downloaded,
                    total,
                };
                Task::none()
            }
            InstallEvent::Installing => {
                self.updater.phase = Phase::Installing(info);
                Task::none()
            }
            InstallEvent::Done(Ok(())) => match updater::relaunch_after_exit() {
                Ok(()) => {
                    log::info!("update {} installed, restarting", info.version);
                    self.updater.phase = Phase::Restarting(info);
                    self.handle_window_close(self.main_window_id)
                }
                Err(error) => {
                    log::warn!(
                        "update {} installed, relaunch failed: {error}",
                        info.version
                    );
                    self.updater.phase = Phase::Installed(info);
                    Task::none()
                }
            },
            InstallEvent::Done(Err(error)) => self.fail_update(info, error),
        }
    }

    fn fail_update(&mut self, info: updater::UpdateInfo, error: String) -> Task<Message> {
        log::warn!("update {} failed: {error}", info.version);
        self.updater.attempts += 1;
        self.updater.phase = Phase::Failed { info, error };
        if self.updater.attempts <= updater::AUTO_RETRIES {
            return Task::perform(tokio::time::sleep(updater::RETRY_DELAY), |()| {
                Message::Updater(UpdaterMessage::Retry)
            });
        }
        Task::none()
    }
}
