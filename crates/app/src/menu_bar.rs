//! The macOS menu bar, laid out like Outlook's. Every item runs something
//! Courly already does and is greyed out whenever that cannot run; there is
//! no item for a feature Courly lacks (print, save as, fonts...).
//!
//! Key equivalents take the key before iced sees it, so none is given to a
//! key a text field needs (⌘C, ⌘V, ⌘X, ⌘A, ⌘Z, ⌘⌫). A greyed-out item lets
//! its key through, which keeps the single-key shortcuts working.

use cmdk::{CommandContext, CommandId};
use iced::Task;
use iced::futures::SinkExt;
use muda::accelerator::KeyAccelerator;
use muda::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::pop_out::compose::ComposeMessage;
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::ui::settings::{SettingsMessage, Tab};
use crate::ui::sidebar::{FolderCommand, SelectedFolder, SidebarMessage};
use crate::ui::toolbar::ToolbarMessage;
use crate::updater::UpdaterMessage;
use rtsk::scope::ViewScope;

const ID_PREFIX: &str = "courly-menu-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuAction {
    Command(CommandId),
    Settings(Tab),
    CheckForUpdates,
    NewFolder,
    RenameFolder,
    MoveFolder,
    DeleteFolder,
    MarkFolderRead,
    EmptyFolder,
    ImportContacts,
    CustomizeToolbar,
    Theme(&'static str),
    Format(Format),
    AllAccounts,
    Account(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    BulletList,
    NumberedList,
    Link,
    Emoji,
    Attach,
}

impl Format {
    fn message(self) -> ComposeMessage {
        match self {
            Self::Bold => ComposeMessage::FormatBold,
            Self::Italic => ComposeMessage::FormatItalic,
            Self::Underline => ComposeMessage::FormatUnderline,
            Self::Strikethrough => ComposeMessage::FormatStrikethrough,
            Self::BulletList => ComposeMessage::FormatList,
            Self::NumberedList => ComposeMessage::FormatOrderedList,
            Self::Link => ComposeMessage::FormatLink,
            Self::Emoji => ComposeMessage::ToggleEmojiPicker,
            Self::Attach => ComposeMessage::AttachFiles,
        }
    }
}

/// What an item shows after a state change.
#[derive(Debug, Clone, PartialEq)]
struct ItemState {
    enabled: bool,
    checked: bool,
    /// Replaces the label, for the item that flips between two actions.
    label: Option<String>,
}

enum Handle {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

struct Entry {
    action: MenuAction,
    handle: Handle,
    state: ItemState,
}

/// One account row of the Profiles menu: index in the sidebar and label.
type ProfileRow = (usize, String);

pub(crate) struct MenuBar {
    /// Owns the native menu, which lives as long as this value.
    _menu: Menu,
    entries: Vec<Entry>,
    profiles: Vec<ProfileRow>,
}

impl MenuBar {
    fn entry(&self, id: &str) -> Option<&Entry> {
        let index: usize = id.strip_prefix(ID_PREFIX)?.parse().ok()?;
        self.entries.get(index)
    }
}

struct Builder {
    entries: Vec<Entry>,
}

impl Builder {
    fn next_id(&self) -> String {
        format!("{ID_PREFIX}{}", self.entries.len())
    }

    fn item(&mut self, menu: &Submenu, action: MenuAction, label: &str, keys: Option<&str>) {
        let item = MenuItem::with_id(self.next_id(), label, false, None);
        if let Some(keys) = keys {
            log_failure(item.set_key_accelerator(Some(accelerator(keys))));
        }
        log_failure(menu.append(&item));
        self.entries.push(Entry {
            action,
            handle: Handle::Plain(item),
            state: ItemState {
                enabled: false,
                checked: false,
                label: None,
            },
        });
    }

    fn check(&mut self, menu: &Submenu, action: MenuAction, label: &str) {
        let item = CheckMenuItem::with_id(self.next_id(), label, false, false, None);
        log_failure(menu.append(&item));
        self.entries.push(Entry {
            action,
            handle: Handle::Check(item),
            state: ItemState {
                enabled: false,
                checked: false,
                label: None,
            },
        });
    }
}

fn accelerator(keys: &str) -> KeyAccelerator {
    keys.parse()
        .unwrap_or_else(|error| panic!("menu shortcut {keys:?} does not parse: {error}"))
}

fn log_failure<E: std::fmt::Display>(outcome: Result<(), E>) {
    if let Err(error) = outcome {
        log::error!("Menu bar: {error}");
    }
}

fn separator(menu: &Submenu) {
    log_failure(menu.append(&PredefinedMenuItem::separator()));
}

fn submenu(parent: &Submenu, label: &str) -> Submenu {
    let menu = Submenu::new(label, true);
    log_failure(parent.append(&menu));
    menu
}

/// Every shortcut the menus show, so a test can parse them all.
const SHORTCUTS: &[&str] = &[
    "cmd+,",
    "cmd+n",
    "cmd+shift+n",
    "cmd+alt+f",
    "cmd+1",
    "cmd+2",
    "cmd+r",
    "cmd+shift+r",
    "cmd+j",
    "cmd+shift+t",
    "cmd+alt+t",
    "ctrl+p",
    "ctrl+e",
    "cmd+shift+m",
    "cmd+b",
    "cmd+i",
    "cmd+u",
    "cmd+k",
    "cmd+=",
    "cmd+-",
    "cmd+0",
    "ctrl+cmd+k",
    "cmd+;",
    "cmd+shift+e",
];

fn shortcut(keys: &'static str) -> Option<&'static str> {
    debug_assert!(SHORTCUTS.contains(&keys), "{keys} missing from SHORTCUTS");
    Some(keys)
}

fn build(profiles: &[ProfileRow]) -> (Menu, Vec<Entry>) {
    use CommandId as C;
    use MenuAction as A;

    let mut b = Builder {
        entries: Vec::new(),
    };
    let menu = Menu::new();
    let tr = |key: &str| t!(key).into_owned();

    // Courly
    let app = Submenu::new("Courly", true);
    let about = AboutMetadata {
        name: Some("Courly".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..Default::default()
    };
    log_failure(app.append(&PredefinedMenuItem::about(
        Some(&tr("main.menu_bar.about")),
        Some(about),
    )));
    separator(&app);
    b.item(
        &app,
        A::Settings(Tab::General),
        &tr("main.menu_bar.settings"),
        shortcut("cmd+,"),
    );
    b.item(&app, A::CheckForUpdates, &tr("main.menu_bar.check_updates"), None);
    separator(&app);
    log_failure(app.append(&PredefinedMenuItem::services(Some(&tr("main.menu_bar.services")))));
    separator(&app);
    log_failure(app.append(&PredefinedMenuItem::hide(Some(&tr("main.menu_bar.hide")))));
    log_failure(app.append(&PredefinedMenuItem::hide_others(Some(&tr(
        "main.menu_bar.hide_others",
    )))));
    log_failure(app.append(&PredefinedMenuItem::show_all(Some(&tr("main.menu_bar.show_all")))));
    separator(&app);
    log_failure(app.append(&PredefinedMenuItem::quit(Some(&tr("main.menu_bar.quit")))));

    // Fichier
    let file = Submenu::new(tr("main.menu_bar.file"), true);
    let new = submenu(&file, &tr("main.menu_bar.new"));
    b.item(&new, A::Command(C::ComposeNew), &tr("main.menu_bar.new_message"), shortcut("cmd+n"));
    b.item(&new, A::Command(C::CalendarCreateEvent), &tr("main.menu_bar.new_event"), None);
    b.item(&new, A::NewFolder, &tr("main.menu_bar.new_folder"), shortcut("cmd+shift+n"));
    log_failure(file.append(&PredefinedMenuItem::close_window(Some(&tr("main.menu_bar.close")))));
    separator(&file);
    let folder = submenu(&file, &tr("main.menu_bar.folder"));
    b.item(&folder, A::RenameFolder, &tr("main.sidebar.rename_folder"), None);
    b.item(&folder, A::MoveFolder, &tr("main.sidebar.move_folder"), None);
    b.item(&folder, A::DeleteFolder, &tr("main.sidebar.delete_folder"), None);
    separator(&folder);
    b.item(&folder, A::MarkFolderRead, &tr("main.sidebar.mark_all_read"), None);
    b.item(&folder, A::EmptyFolder, &tr("main.sidebar.empty_folder"), None);
    separator(&file);
    let import = submenu(&file, &tr("main.menu_bar.import"));
    b.item(&import, A::ImportContacts, &tr("main.menu_bar.import_contacts"), None);

    // Modifier
    let edit = Submenu::new(tr("main.menu_bar.edit"), true);
    b.item(&edit, A::Command(C::Undo), &tr("main.menu_bar.undo"), None);
    separator(&edit);
    b.item(&edit, A::Command(C::EmailSelectAll), &tr("main.menu_bar.select_all"), None);
    separator(&edit);
    b.item(&edit, A::Command(C::EmailTrash), &tr("main.menu_bar.delete"), None);
    b.item(
        &edit,
        A::Command(C::EmailPermanentDelete),
        &tr("main.menu_bar.delete_permanently"),
        None,
    );
    b.item(&edit, A::EmptyFolder, &tr("main.sidebar.empty_folder"), None);
    separator(&edit);
    b.item(&edit, A::Command(C::AppSearch), &tr("main.menu_bar.search"), shortcut("cmd+alt+f"));

    // Affichage
    let view = Submenu::new(tr("main.menu_bar.view"), true);
    b.item(&view, A::Command(C::NavPrev), &tr("main.menu_bar.previous"), None);
    b.item(&view, A::Command(C::NavNext), &tr("main.menu_bar.next"), None);
    separator(&view);
    let go = submenu(&view, &tr("main.menu_bar.go_to"));
    b.item(&go, A::Command(C::SwitchToMail), &tr("main.menu_bar.mail"), shortcut("cmd+1"));
    b.item(&go, A::Command(C::SwitchToCalendar), &tr("main.menu_bar.calendar"), shortcut("cmd+2"));
    separator(&go);
    for (command, key) in [
        (C::NavGoInbox, "main.folder.inbox"),
        (C::NavGoStarred, "main.folder.starred"),
        (C::NavGoSnoozed, "main.folder.snoozed"),
        (C::NavGoSent, "main.folder.sent"),
        (C::NavGoDrafts, "main.folder.drafts"),
        (C::NavGoTrash, "main.folder.trash"),
        (C::NavGoAllMail, "main.folder.all_mail"),
    ] {
        b.item(&go, A::Command(command), &tr(key), None);
    }
    separator(&view);
    let theme = submenu(&view, &tr("main.menu_bar.theme"));
    b.check(&theme, A::Theme("Light"), &tr("main.menu_bar.theme_light"));
    b.check(&theme, A::Theme("Dark"), &tr("main.menu_bar.theme_dark"));
    b.check(&theme, A::Theme("System"), &tr("main.menu_bar.theme_system"));
    b.check(&view, A::Command(C::ViewToggleSidePanel), &tr("main.menu_bar.side_panel"));
    b.item(&view, A::CustomizeToolbar, &tr("main.menu_bar.customize_toolbar"), None);
    separator(&view);
    log_failure(view.append(&PredefinedMenuItem::fullscreen(Some(&tr(
        "main.menu_bar.fullscreen",
    )))));

    // Message
    let message = Submenu::new(tr("main.menu_bar.message"), true);
    b.item(&message, A::Command(C::ComposeReply), &tr("main.row_menu.reply"), shortcut("cmd+r"));
    b.item(
        &message,
        A::Command(C::ComposeReplyAll),
        &tr("main.row_menu.reply_all"),
        shortcut("cmd+shift+r"),
    );
    b.item(&message, A::Command(C::ComposeForward), &tr("main.row_menu.forward"), shortcut("cmd+j"));
    separator(&message);
    b.item(
        &message,
        A::Command(C::EmailMarkRead),
        &tr("main.row_menu.mark_read"),
        shortcut("cmd+shift+t"),
    );
    b.item(&message, A::MarkFolderRead, &tr("main.sidebar.mark_all_read"), shortcut("cmd+alt+t"));
    separator(&message);
    b.item(&message, A::Command(C::EmailSnooze), &tr("main.row_menu.snooze"), None);
    b.item(&message, A::Command(C::EmailStar), &tr("main.row_menu.flag"), None);
    b.item(&message, A::Command(C::EmailPin), &tr("main.row_menu.pin"), shortcut("ctrl+p"));
    b.item(&message, A::Command(C::EmailAddLabel), &tr("main.row_menu.categorize"), None);
    separator(&message);
    b.item(&message, A::Command(C::EmailSpam), &tr("main.row_menu.junk"), None);
    b.item(&message, A::Command(C::EmailUnsubscribe), &tr("main.menu_bar.unsubscribe"), None);
    b.item(&message, A::Command(C::EmailMute), &tr("main.row_menu.mute"), None);
    separator(&message);
    b.item(&message, A::Command(C::EmailArchive), &tr("main.row_menu.archive"), shortcut("ctrl+e"));
    b.item(
        &message,
        A::Command(C::EmailMoveToFolder),
        &tr("main.row_menu.move"),
        shortcut("cmd+shift+m"),
    );

    // Format
    let format = Submenu::new(tr("main.menu_bar.format"), true);
    b.item(&format, A::Format(Format::Bold), &tr("main.menu_bar.bold"), shortcut("cmd+b"));
    b.item(&format, A::Format(Format::Italic), &tr("main.menu_bar.italic"), shortcut("cmd+i"));
    b.item(&format, A::Format(Format::Underline), &tr("main.menu_bar.underline"), shortcut("cmd+u"));
    b.item(&format, A::Format(Format::Strikethrough), &tr("main.menu_bar.strikethrough"), None);
    separator(&format);
    b.item(&format, A::Format(Format::BulletList), &tr("main.menu_bar.bullet_list"), None);
    b.item(&format, A::Format(Format::NumberedList), &tr("main.menu_bar.numbered_list"), None);
    separator(&format);
    b.item(&format, A::Format(Format::Link), &tr("main.menu_bar.link"), shortcut("cmd+k"));
    b.item(&format, A::Format(Format::Emoji), &tr("main.menu_bar.emoji"), None);
    b.item(&format, A::Format(Format::Attach), &tr("main.menu_bar.attach"), None);
    separator(&format);
    b.item(&format, A::Command(C::ViewEmailZoomIn), &tr("main.menu_bar.zoom_in"), shortcut("cmd+="));
    b.item(&format, A::Command(C::ViewEmailZoomOut), &tr("main.menu_bar.zoom_out"), shortcut("cmd+-"));
    b.item(
        &format,
        A::Command(C::ViewEmailZoomActualSize),
        &tr("main.menu_bar.actual_size"),
        shortcut("cmd+0"),
    );

    // Profils
    let profiles_menu = Submenu::new(tr("main.menu_bar.profiles"), true);
    b.check(&profiles_menu, A::AllAccounts, &tr("main.sidebar.all_accounts"));
    separator(&profiles_menu);
    for (index, label) in profiles {
        b.check(&profiles_menu, A::Account(*index), label);
    }
    separator(&profiles_menu);
    b.item(
        &profiles_menu,
        A::Settings(Tab::Accounts),
        &tr("main.menu_bar.manage_accounts"),
        None,
    );

    // Outils
    let tools = Submenu::new(tr("main.menu_bar.tools"), true);
    b.item(&tools, A::Command(C::AppSyncFolder), &tr("main.menu_bar.sync"), shortcut("ctrl+cmd+k"));
    separator(&tools);
    b.item(&tools, A::Settings(Tab::Filters), &tr("main.menu_bar.rules"), None);
    b.item(&tools, A::Settings(Tab::Labels), &tr("main.menu_bar.categories"), None);
    b.item(&tools, A::Settings(Tab::Accounts), &tr("main.menu_bar.accounts"), None);
    b.item(&tools, A::Settings(Tab::Shortcuts), &tr("main.menu_bar.shortcuts"), shortcut("cmd+;"));
    separator(&tools);
    // ⌘K stays with Hyperlink; while that is greyed out the key reaches
    // iced, which opens the palette.
    b.item(&tools, A::Command(C::AppOpenPalette), &tr("main.menu_bar.palette"), None);
    b.item(
        &tools,
        A::Command(C::AppRebuildSearchIndex),
        &tr("main.menu_bar.rebuild_index"),
        None,
    );

    // Fenêtre
    let window = Submenu::new(tr("main.menu_bar.window"), true);
    log_failure(window.append(&PredefinedMenuItem::minimize(Some(&tr("main.menu_bar.minimize")))));
    log_failure(window.append(&PredefinedMenuItem::maximize(Some(&tr("main.menu_bar.zoom")))));
    separator(&window);
    b.item(
        &window,
        A::Command(C::CalendarPopOut),
        &tr("main.menu_bar.calendar_window"),
        None,
    );
    separator(&window);
    log_failure(window.append(&PredefinedMenuItem::bring_all_to_front(Some(&tr(
        "main.menu_bar.bring_all_to_front",
    )))));

    // Aide
    let help = Submenu::new(tr("main.menu_bar.help"), true);
    b.item(&help, A::Settings(Tab::Shortcuts), &tr("main.menu_bar.shortcuts"), None);
    b.item(&help, A::CheckForUpdates, &tr("main.menu_bar.check_updates"), None);

    for top in [
        &app,
        &file,
        &edit,
        &view,
        &message,
        &format,
        &profiles_menu,
        &tools,
        &window,
        &help,
    ] {
        log_failure(menu.append(top));
    }
    window.set_as_windows_menu_for_nsapp();
    help.set_as_help_menu_for_nsapp();
    (menu, b.entries)
}

/// The menu clicks, as `Message::MenuBar` with the item id.
pub(crate) fn events() -> iced::Subscription<Message> {
    iced::Subscription::run(|| {
        iced::stream::channel(32, async |mut output| {
            let (sender, mut receiver) = iced::futures::channel::mpsc::unbounded();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                // The receiver only goes away with the subscription.
                let _ = sender.unbounded_send(event.id.0);
            }));
            while let Some(id) = iced::futures::StreamExt::next(&mut receiver).await {
                if output.send(Message::MenuBar(id)).await.is_err() {
                    break;
                }
            }
        })
    })
}

/// What the menu needs to know about the app, read once per refresh.
struct Snapshot {
    commands: CommandContext,
    folder: Option<SelectedFolder>,
    compose: Option<iced::window::Id>,
    busy: bool,
}

impl ReadyApp {
    pub(crate) fn install_menu_bar(&mut self) {
        let profiles = self.menu_profiles();
        let (menu, entries) = build(&profiles);
        menu.init_for_nsapp();
        self.menu_bar = Some(MenuBar {
            _menu: menu,
            entries,
            profiles,
        });
        self.refresh_menu_bar();
    }

    fn menu_profiles(&self) -> Vec<ProfileRow> {
        self.sidebar
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, account)| !account.is_deleting)
            .map(|(index, account)| {
                let label = account
                    .account_name
                    .as_deref()
                    .or(account.display_name.as_deref())
                    .unwrap_or(&account.email);
                (index, label.to_owned())
            })
            .collect()
    }

    /// Bring every item's enabled, checked and label state up to date.
    pub(crate) fn refresh_menu_bar(&mut self) {
        let Some(bar) = &self.menu_bar else {
            return;
        };
        if bar.profiles != self.menu_profiles() {
            self.install_menu_bar();
            return;
        }
        let snapshot = Snapshot {
            commands: crate::command_dispatch::build_context(self),
            folder: self.sidebar.selected_folder(),
            compose: self.active_compose(),
            busy: self.folder_dialog.is_some(),
        };
        let states: Vec<ItemState> = bar
            .entries
            .iter()
            .map(|entry| self.menu_item_state(entry.action, &snapshot))
            .collect();
        let Some(bar) = &mut self.menu_bar else {
            return;
        };
        for (entry, state) in bar.entries.iter_mut().zip(states) {
            if entry.state == state {
                continue;
            }
            match &entry.handle {
                Handle::Plain(item) => {
                    item.set_enabled(state.enabled);
                    if let Some(label) = &state.label {
                        item.set_text(label);
                    }
                }
                Handle::Check(item) => {
                    item.set_enabled(state.enabled);
                    item.set_checked(state.checked);
                }
            }
            entry.state = state;
        }
    }

    /// The compose the Format menu acts on: the focused compose window,
    /// or the compose inside the main window when that one is focused.
    fn active_compose(&self) -> Option<iced::window::Id> {
        let is_compose = |id: &iced::window::Id| {
            matches!(self.pop_out_windows.get(id), Some(PopOutWindow::Compose(_)))
        };
        if is_compose(&self.focused_window) {
            return Some(self.focused_window);
        }
        if self.focused_window == self.main_window_id && !self.show_settings {
            return self.inline_compose.filter(is_compose);
        }
        None
    }

    fn menu_item_state(&self, action: MenuAction, snapshot: &Snapshot) -> ItemState {
        let mut state = ItemState {
            enabled: !snapshot.busy,
            checked: false,
            label: None,
        };
        let own_folder = snapshot.folder.as_ref().is_some_and(|f| f.own);
        match action {
            MenuAction::Command(id) => {
                state.enabled &= self
                    .registry
                    .get(id)
                    .is_some_and(|command| (command.is_available)(&snapshot.commands));
                if id == CommandId::ViewToggleSidePanel {
                    state.checked = self.side_panel.open;
                }
                if id == CommandId::EmailMarkRead {
                    let key = if snapshot.commands.thread_is_read == Some(true) {
                        "main.row_menu.mark_unread"
                    } else {
                        "main.row_menu.mark_read"
                    };
                    state.label = Some(t!(key).into_owned());
                }
            }
            MenuAction::NewFolder => {
                state.enabled &= matches!(self.sidebar.selected_scope, ViewScope::Account(_));
            }
            MenuAction::RenameFolder
            | MenuAction::MoveFolder
            | MenuAction::DeleteFolder
            | MenuAction::EmptyFolder => state.enabled &= own_folder,
            MenuAction::MarkFolderRead => {
                state.enabled &= snapshot.folder.as_ref().is_some_and(|f| f.holds_messages);
            }
            MenuAction::Format(_) => state.enabled &= snapshot.compose.is_some(),
            MenuAction::Theme(theme) => state.checked = self.settings.theme == theme,
            MenuAction::AllAccounts => {
                state.checked = matches!(self.sidebar.selected_scope, ViewScope::AllAccounts);
            }
            MenuAction::Account(index) => {
                state.checked = self.sidebar.accounts.get(index).is_some_and(|account| {
                    self.sidebar.selected_scope == ViewScope::Account(account.id.clone())
                });
            }
            MenuAction::Settings(_)
            | MenuAction::CheckForUpdates
            | MenuAction::ImportContacts
            | MenuAction::CustomizeToolbar => {}
        }
        state
    }

    pub(crate) fn handle_menu_bar(&mut self, id: &str) -> Task<Message> {
        // A click can land after the item was greyed out.
        let Some(action) = self
            .menu_bar
            .as_ref()
            .and_then(|bar| bar.entry(id))
            .filter(|entry| entry.state.enabled)
            .map(|entry| entry.action)
        else {
            return Task::none();
        };
        let folder = self.sidebar.selected_folder();
        match action {
            MenuAction::Command(command) => self.update(Message::ExecuteCommand(command)),
            MenuAction::Settings(tab) => {
                self.open_settings(tab);
                Task::none()
            }
            MenuAction::CheckForUpdates => {
                self.open_settings(Tab::About);
                self.update(Message::Updater(UpdaterMessage::CheckRequestedByUser))
            }
            MenuAction::NewFolder => {
                let ViewScope::Account(account_id) = self.sidebar.selected_scope.clone() else {
                    return Task::none();
                };
                let parent = folder
                    .filter(|f| f.own)
                    .map(|f| (f.folder_id, f.name));
                self.handle_folder_command(FolderCommand::Create { account_id, parent })
            }
            MenuAction::RenameFolder
            | MenuAction::MoveFolder
            | MenuAction::DeleteFolder
            | MenuAction::EmptyFolder
            | MenuAction::MarkFolderRead => {
                match folder.and_then(|folder| folder_command(action, folder)) {
                    Some(command) => self.handle_folder_command(command),
                    None => Task::none(),
                }
            }
            MenuAction::ImportContacts => {
                self.open_settings(Tab::People);
                self.update(Message::Settings(SettingsMessage::ImportContactsOpen))
            }
            MenuAction::CustomizeToolbar => self.update(Message::Toolbar(ToolbarMessage::Customize)),
            MenuAction::Theme(theme) => self.update(Message::SetTheme(theme.to_owned())),
            MenuAction::Format(format) => match self.active_compose() {
                Some(window) => self.update(Message::PopOut(
                    window,
                    PopOutMessage::Compose(format.message()),
                )),
                None => Task::none(),
            },
            MenuAction::AllAccounts => self.update(Message::Sidebar(SidebarMessage::SelectAllAccounts)),
            MenuAction::Account(index) => {
                self.update(Message::Sidebar(SidebarMessage::SelectAccount(index)))
            }
        }
    }
}

/// The folder command a Folder menu item runs on the open folder. Only
/// marking read covers several accounts; the rest need one own folder.
fn folder_command(action: MenuAction, folder: SelectedFolder) -> Option<FolderCommand> {
    let SelectedFolder {
        account_ids,
        folder_id,
        name,
        own,
        ..
    } = folder;
    if action == MenuAction::MarkFolderRead {
        return Some(FolderCommand::MarkAllRead {
            account_ids,
            folder_id,
        });
    }
    let [account_id] = <[String; 1]>::try_from(account_ids).ok()?;
    if !own {
        return None;
    }
    Some(match action {
        MenuAction::RenameFolder => FolderCommand::Rename {
            account_id,
            folder_id,
            name,
        },
        MenuAction::MoveFolder => FolderCommand::Move {
            account_id,
            folder_id,
            name,
        },
        MenuAction::DeleteFolder => FolderCommand::Delete {
            account_id,
            folder_id,
            name,
        },
        MenuAction::EmptyFolder => FolderCommand::Empty {
            account_id,
            folder_id,
            name,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_parses() {
        for keys in SHORTCUTS {
            assert!(keys.parse::<KeyAccelerator>().is_ok(), "{keys} does not parse");
        }
    }

    #[test]
    fn no_shortcut_takes_a_text_editing_key() {
        for taken in ["cmd+c", "cmd+v", "cmd+x", "cmd+a", "cmd+z", "cmd+shift+z"] {
            assert!(!SHORTCUTS.contains(&taken), "{taken} belongs to text fields");
        }
    }

    #[test]
    fn folder_items_target_the_open_folder() {
        let folder = SelectedFolder {
            account_ids: vec!["acc".into()],
            folder_id: "f1".into(),
            name: "Projets".into(),
            own: true,
            holds_messages: true,
        };
        assert_eq!(
            folder_command(MenuAction::DeleteFolder, folder.clone()),
            Some(FolderCommand::Delete {
                account_id: "acc".into(),
                folder_id: "f1".into(),
                name: "Projets".into(),
            })
        );
        assert_eq!(
            folder_command(MenuAction::MarkFolderRead, folder.clone()),
            Some(FolderCommand::MarkAllRead {
                account_ids: vec!["acc".into()],
                folder_id: "f1".into(),
            })
        );
        let system = SelectedFolder {
            account_ids: vec!["a".into(), "b".into()],
            own: false,
            ..folder
        };
        assert_eq!(folder_command(MenuAction::EmptyFolder, system.clone()), None);
        assert!(folder_command(MenuAction::MarkFolderRead, system).is_some());
    }
}
