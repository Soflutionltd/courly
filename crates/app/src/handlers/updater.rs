use iced::Task;
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::updater::{self, InstallEvent, Phase, UpdaterMessage};

impl ReadyApp {
    pub(crate) fn handle_updater(&mut self, msg: UpdaterMessage) -> Task<Message> {
        match msg {
            UpdaterMessage::Check { forced } => self.start_update_check(forced),
            UpdaterMessage::CheckRequestedByUser => {
                if updater::installed_bundle().is_none() {
                    self.settings.update_check_note =
                        Some(t!("updater.error.not_installed").into_owned());
                    return Task::none();
                }
                let task = self.start_update_check(true);
                if self.updater.checking {
                    self.updater.report_outcome = true;
                    self.settings.update_check_note = Some(t!("updater.checking").into_owned());
                }
                task
            }
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
        let report = std::mem::take(&mut self.updater.report_outcome);
        let info = match result {
            Ok(Some(info)) => {
                self.settings.update_check_note = report.then(|| {
                    t!("updater.available", version = info.version).into_owned()
                });
                info
            }
            Ok(None) => {
                if matches!(self.updater.phase, Phase::Available(_)) {
                    self.updater.phase = Phase::Idle;
                }
                if report {
                    self.settings.update_check_note = Some(
                        t!("updater.up_to_date", version = updater::current_version())
                            .into_owned(),
                    );
                }
                return Task::none();
            }
            Err(error) => {
                log::info!("update check failed: {error}");
                if report {
                    self.settings.update_check_note = Some(t!("updater.check_failed").into_owned());
                }
                return Task::none();
            }
        };
        if !matches!(self.updater.phase, Phase::Idle | Phase::Available(_)) {
            return Task::none();
        }
        let dismissed = self.updater.dismissed_version.as_deref() == Some(info.version.as_str());
        if dismissed && !report {
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
