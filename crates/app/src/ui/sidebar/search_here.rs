use rtsk::scope::ViewScope;

use super::Sidebar;

/// Build a scope query prefix for "Search here" from a label/folder name.
pub(super) fn build_search_here_prefix(name: &str, sidebar: &Sidebar, scope: &ViewScope) -> String {
    if let Some(account) = scope_account(sidebar, scope) {
        let account_name = account.display_name.as_deref().unwrap_or(&account.email);
        format!(
            "account:{} label:{} ",
            quote_if_needed(account_name),
            quote_if_needed(name),
        )
    } else {
        format!("label:{} ", quote_if_needed(name))
    }
}

/// Build a scope query prefix for provider folders. These use `folder:`
/// because `label:` is reserved for tag-kind labels in search.
pub(super) fn build_search_here_user_folder_prefix(
    name: &str,
    sidebar: &Sidebar,
    scope: &ViewScope,
) -> String {
    if let Some(account) = scope_account(sidebar, scope) {
        let account_name = account.display_name.as_deref().unwrap_or(&account.email);
        format!(
            "account:{} folder:{} ",
            quote_if_needed(account_name),
            quote_if_needed(name),
        )
    } else {
        format!("folder:{} ", quote_if_needed(name))
    }
}

/// Build a scope query prefix for universal folders (Inbox, Sent, etc.).
pub(super) fn build_search_here_folder_prefix(
    folder_name: &str,
    sidebar: &Sidebar,
    scope: &ViewScope,
) -> String {
    if let Some(account) = scope_account(sidebar, scope) {
        let account_name = account.display_name.as_deref().unwrap_or(&account.email);
        format!(
            "account:{} in:{} ",
            quote_if_needed(account_name),
            folder_name.to_lowercase(),
        )
    } else {
        format!("in:{} ", folder_name.to_lowercase())
    }
}

/// The account a section's search is limited to, if any.
fn scope_account<'a>(sidebar: &'a Sidebar, scope: &ViewScope) -> Option<&'a crate::db::Account> {
    match scope {
        ViewScope::Account(id) => sidebar.accounts.iter().find(|a| a.id == *id),
        _ => None,
    }
}

/// Wrap a value in quotes if it contains spaces.
fn quote_if_needed(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}
