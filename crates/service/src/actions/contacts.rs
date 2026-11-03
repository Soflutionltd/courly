//! Contact writes: the local row, then every address book holding the
//! contact (the Mac's address book and the accounts' cards).

use std::sync::Arc;

use bifrost_types::{
    AccountId, ContactCard, ContactCreate, ContactEmail, ContactId, ContactOrganization,
    ContactPatch, ContactPhone,
};
use db::db::queries_extra::contacts::{
    UpsertContactParams, db_delete_contact, db_upsert_contact_full,
};
use db::db::queries_extra::{
    AccountContactHolder, ContactHolders, StoredContact, contact_holders_sync,
    forget_contact_holders_sync, get_stored_contact_sync, rename_contact_claim_sync,
    set_contact_origin_sync, upsert_contact_claim_sync,
};

use super::contact_fields::{ContactFields, ValueEdit, same_address, same_phone};
use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use crate::bifrost::resident::ResidentActionAccount;
use crate::sync::SyncRuntime;

const LOCAL_SOURCE: &str = "user";
const DEVICE_SOURCE: &str = "macos";

#[derive(Debug, Clone)]
pub struct ContactSaveInput {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    /// For a new contact, the account whose address book receives it.
    pub account_id: Option<String>,
    /// For a new contact, where it is created: `user` (Courly only),
    /// `macos` (the Mac's address book), or the account's source.
    pub source: Option<String>,
    pub server_id: Option<String>,
}

impl ContactSaveInput {
    fn fields(&self) -> ContactFields {
        ContactFields {
            display_name: self.display_name.clone(),
            email: self.email.clone(),
            email2: self.email2.clone(),
            phone: self.phone.clone(),
            company: self.company.clone(),
            notes: self.notes.clone(),
        }
    }
}

fn stored_fields(stored: &StoredContact) -> ContactFields {
    ContactFields {
        display_name: stored.display_name.clone(),
        email: stored.email.clone(),
        email2: stored.email2.clone(),
        phone: stored.phone.clone(),
        company: stored.company.clone(),
        notes: stored.notes.clone(),
    }
}

/// Reaches the accounts holding a contact.
pub(crate) struct ContactAccounts(pub(crate) Option<Arc<SyncRuntime>>);

impl ContactAccounts {
    async fn get(&self, account_id: &str) -> Result<ResidentActionAccount, String> {
        match &self.0 {
            Some(runtime) => runtime.resident_action_account(account_id).await,
            None => Err("the account is not connected yet".to_owned()),
        }
    }
}

/// An address book holding a contact.
enum Holder {
    Account(AccountContactHolder),
    Device(String),
}

/// Where a contact lives: its claims, or for a row synced before claims
/// existed, the row's own account and card.
fn holders_of(stored: &StoredContact, holders: ContactHolders) -> Vec<Holder> {
    let mut found: Vec<Holder> = holders
        .accounts
        .into_iter()
        .map(Holder::Account)
        .chain(holders.device_ids.into_iter().map(Holder::Device))
        .collect();
    if found.is_empty()
        && let (Some(account_id), Some(server_id)) = (&stored.account_id, &stored.server_id)
        && stored.source != LOCAL_SOURCE
        && stored.source != DEVICE_SOURCE
    {
        found.push(Holder::Account(AccountContactHolder {
            account_id: account_id.clone(),
            source: stored.source.clone(),
            server_id: server_id.clone(),
        }));
    }
    found
}

/// A write that failed in one address book while the others went through.
struct BookFailure {
    book: String,
    error: ActionError,
}

fn book_name(source: &str) -> &str {
    match source {
        DEVICE_SOURCE => "Apple Contacts",
        "graph" => "Outlook",
        "google" => "Google",
        other => other,
    }
}

fn outcome_of(failures: &[BookFailure]) -> ActionOutcome {
    if failures.is_empty() {
        return ActionOutcome::Success;
    }
    let retryable = failures.iter().any(|failure| failure.error.is_retryable());
    let message = failures
        .iter()
        .map(|failure| format!("{}: {}", failure.book, failure.error.user_message()))
        .collect::<Vec<_>>()
        .join("; ");
    ActionOutcome::LocalOnly {
        reason: ActionError::remote(message),
        retryable,
    }
}

/// Saves a contact. An edit reaches every address book holding it; a new
/// contact is created in the one `input.source` names. When an address
/// book refuses the write, the contact stays in Courly and the outcome
/// says which book and why.
pub(crate) async fn save_contact(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    input: ContactSaveInput,
) -> ActionOutcome {
    let mlog = MutationLog::begin(
        "save_contact",
        input.account_id.as_deref().unwrap_or(""),
        &input.id,
    );
    let outcome = match save(ctx, accounts, input).await {
        Ok(outcome) => outcome,
        Err(error) => ActionOutcome::Failed { error },
    };
    mlog.emit(&outcome);
    outcome
}

async fn save(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    input: ContactSaveInput,
) -> Result<ActionOutcome, ActionError> {
    let id = input.id.clone();
    let current = ctx
        .write_db
        .clone()
        .with_write_mapped(
            move |conn| {
                let Some(stored) = get_stored_contact_sync(conn, &id)? else {
                    return Ok(None);
                };
                let holders = contact_holders_sync(conn, &stored.email)?;
                Ok(Some((stored, holders)))
            },
            std::convert::identity,
        )
        .await
        .map_err(ActionError::db)?;

    match current {
        Some((stored, holders)) => {
            write_local(ctx, &input, stored.account_id.as_deref(), &stored.source).await?;
            let old = stored_fields(&stored);
            let new = input.fields();
            if old == new {
                return Ok(ActionOutcome::Success);
            }
            let mut failures = Vec::new();
            let mut updated_cards = Vec::new();
            for holder in holders_of(&stored, holders) {
                match update_holder(ctx, accounts, &holder, &old, &new).await {
                    Some(failure) => failures.push(failure),
                    None => {
                        if let Holder::Account(card) = holder {
                            updated_cards.push(card);
                        }
                    }
                }
            }
            if !same_address(&old.email, &new.email) && !updated_cards.is_empty() {
                let (old_email, new_email) = (old.email.clone(), new.email.trim().to_lowercase());
                ctx.write_db
                    .clone()
                    .with_write_mapped(
                        move |conn| {
                            for card in &updated_cards {
                                rename_contact_claim_sync(
                                    conn,
                                    &card.account_id,
                                    &card.server_id,
                                    &old_email,
                                    &new_email,
                                )?;
                            }
                            Ok(())
                        },
                        std::convert::identity,
                    )
                    .await
                    .map_err(ActionError::db)?;
            }
            Ok(outcome_of(&failures))
        }
        None => {
            write_local(ctx, &input, None, LOCAL_SOURCE).await?;
            let destination = input.source.as_deref().unwrap_or(LOCAL_SOURCE);
            if destination == LOCAL_SOURCE {
                return Ok(ActionOutcome::Success);
            }
            let created = create_in(ctx, accounts, destination, &input).await;
            Ok(outcome_of(created.err().as_slice()))
        }
    }
}

async fn write_local(
    ctx: &ActionContext,
    input: &ContactSaveInput,
    account_id: Option<&str>,
    source: &str,
) -> Result<(), ActionError> {
    let local = input.clone();
    let account_id = account_id.map(str::to_owned);
    let source = source.to_owned();
    ctx.write_db
        .clone()
        .with_write_mapped(
            move |conn| {
                db_upsert_contact_full(
                    conn,
                    UpsertContactParams {
                        id: &local.id,
                        email: &local.email,
                        display_name: local.display_name.as_deref(),
                        email2: local.email2.as_deref(),
                        phone: local.phone.as_deref(),
                        company: local.company.as_deref(),
                        notes: local.notes.as_deref(),
                        account_id: account_id.as_deref(),
                        source: &source,
                    },
                )
                .map_err(ActionError::db)
            },
            ActionError::db,
        )
        .await
}

async fn update_holder(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    holder: &Holder,
    old: &ContactFields,
    new: &ContactFields,
) -> Option<BookFailure> {
    match holder {
        Holder::Account(holder) => {
            let result = match accounts.get(&holder.account_id).await {
                Ok(account) => update_card(&account, &holder.server_id, old, new).await,
                Err(error) => Err(ActionError::remote(error)),
            };
            result.err().map(|error| BookFailure {
                book: book_name(&holder.source).to_owned(),
                error,
            })
        }
        Holder::Device(identifier) => {
            update_device(ctx, identifier.clone(), old.clone(), new.clone())
                .await
                .err()
                .map(|error| BookFailure {
                    book: book_name(DEVICE_SOURCE).to_owned(),
                    error,
                })
        }
    }
}

#[cfg(target_os = "macos")]
async fn update_device(
    ctx: &ActionContext,
    identifier: String,
    old: ContactFields,
    new: ContactFields,
) -> Result<(), ActionError> {
    crate::device_contacts::update(&ctx.write_db, identifier, old, new)
        .await
        .map_err(ActionError::remote)
}

#[cfg(not(target_os = "macos"))]
async fn update_device(
    _ctx: &ActionContext,
    _identifier: String,
    _old: ContactFields,
    _new: ContactFields,
) -> Result<(), ActionError> {
    Err(ActionError::not_implemented(
        "the Mac's address book exists only on macOS",
    ))
}

/// Creates a new contact in `destination` and records it there.
async fn create_in(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    destination: &str,
    input: &ContactSaveInput,
) -> Result<(), BookFailure> {
    let failure = |error| BookFailure {
        book: book_name(destination).to_owned(),
        error,
    };
    let fields = input.fields();
    let (account_id, server_id) = if destination == DEVICE_SOURCE {
        (
            None,
            create_device(ctx, fields.clone()).await.map_err(failure)?,
        )
    } else {
        let account_id = input.account_id.clone().ok_or_else(|| {
            failure(ActionError::invalid_state(
                "a contact created in an account needs that account",
            ))
        })?;
        let account = accounts
            .get(&account_id)
            .await
            .map_err(|error| failure(ActionError::remote(error)))?;
        let server_id = account
            .engine
            .contact_create(&AccountId(account_id.clone()), new_card(&fields))
            .await
            .map_err(|error| failure(map_engine_error(error)))?
            .0;
        (Some(account_id), server_id)
    };

    let id = input.id.clone();
    let source = destination.to_owned();
    let emails = fields.emails();
    ctx.write_db
        .clone()
        .with_write_mapped(
            move |conn| {
                set_contact_origin_sync(
                    conn,
                    &id,
                    &source,
                    account_id.as_deref(),
                    Some(&server_id),
                )?;
                if let Some(account_id) = &account_id {
                    for email in &emails {
                        upsert_contact_claim_sync(
                            conn,
                            account_id,
                            &source,
                            &server_id,
                            &email.trim().to_lowercase(),
                            None,
                            "main",
                        )?;
                    }
                }
                Ok(())
            },
            std::convert::identity,
        )
        .await
        .map_err(|error| failure(ActionError::db(error)))
}

#[cfg(target_os = "macos")]
async fn create_device(ctx: &ActionContext, fields: ContactFields) -> Result<String, ActionError> {
    crate::device_contacts::create(&ctx.write_db, fields)
        .await
        .map_err(ActionError::remote)
}

#[cfg(not(target_os = "macos"))]
async fn create_device(
    _ctx: &ActionContext,
    _fields: ContactFields,
) -> Result<String, ActionError> {
    Err(ActionError::not_implemented(
        "the Mac's address book exists only on macOS",
    ))
}

fn new_card(fields: &ContactFields) -> ContactCreate {
    ContactCreate {
        display_name: fields.display_name.clone(),
        emails: fields
            .emails()
            .into_iter()
            .enumerate()
            .map(|(index, value)| ContactEmail {
                value,
                kind: None,
                is_primary: index == 0,
            })
            .collect(),
        phones: fields
            .phone
            .iter()
            .map(|value| ContactPhone {
                value: value.clone(),
                kind: Some("mobile".to_owned()),
                is_primary: true,
            })
            .collect(),
        organizations: fields
            .company
            .iter()
            .map(|name| ContactOrganization {
                name: name.clone(),
                title: None,
            })
            .collect(),
        notes: fields.notes.clone(),
        ..ContactCreate::default()
    }
}

/// Applies an edit to an account's card. Only the fields the user changed
/// are sent, merged into the card's current values so the numbers,
/// addresses and job title Courly does not show stay as they are.
async fn update_card(
    account: &ResidentActionAccount,
    server_id: &str,
    old: &ContactFields,
    new: &ContactFields,
) -> Result<(), ActionError> {
    let account_id = AccountId(account.account_id.clone());
    let card = account
        .engine
        .contact_get(&account_id, ContactId(server_id.to_owned()))
        .await
        .map_err(map_engine_error)?;
    let patch = card_patch(&card, old, new);
    if patch == ContactPatch::default() {
        return Ok(());
    }
    account
        .engine
        .contact_update(&account_id, ContactId(server_id.to_owned()), patch)
        .await
        .map_err(map_engine_error)
}

fn card_patch(card: &ContactCard, old: &ContactFields, new: &ContactFields) -> ContactPatch {
    let mut patch = ContactPatch::default();
    if old.display_name != new.display_name {
        patch.display_name = Some(new.display_name.clone());
    }
    if old.notes != new.notes {
        patch.notes = Some(new.notes.clone());
    }

    let mut emails = card.emails.clone();
    for edit in [
        ValueEdit::between(Some(&old.email), Some(&new.email)),
        ValueEdit::between(old.email2.as_deref(), new.email2.as_deref()),
    ] {
        edit.apply(
            &mut emails,
            |email| email.value.clone(),
            |email, value| value.clone_into(&mut email.value),
            |value| ContactEmail {
                value: value.to_owned(),
                kind: None,
                is_primary: false,
            },
            same_address,
        );
    }
    if emails != card.emails {
        if !emails.iter().any(|email| email.is_primary)
            && let Some(first) = emails.first_mut()
        {
            first.is_primary = true;
        }
        patch.emails = Some(emails);
    }

    let mut phones = card.phones.clone();
    let has_mobile = phones
        .iter()
        .any(|phone| phone.kind.as_deref() == Some("mobile"));
    ValueEdit::between(old.phone.as_deref(), new.phone.as_deref()).apply(
        &mut phones,
        |phone| phone.value.clone(),
        |phone, value| value.clone_into(&mut phone.value),
        |value| ContactPhone {
            value: value.to_owned(),
            kind: (!has_mobile).then(|| "mobile".to_owned()),
            is_primary: false,
        },
        same_phone,
    );
    if phones != card.phones {
        patch.phones = Some(phones);
    }

    if old.company != new.company {
        let mut organizations = card.organizations.clone();
        match (organizations.first_mut(), new.company.as_deref()) {
            (Some(first), Some(name)) => name.clone_into(&mut first.name),
            (Some(first), None) if first.title.is_some() => first.name.clear(),
            (Some(_), None) => {
                organizations.remove(0);
            }
            (None, Some(name)) => organizations.push(ContactOrganization {
                name: name.to_owned(),
                title: None,
            }),
            (None, None) => {}
        }
        if organizations != card.organizations {
            patch.organizations = Some(organizations);
        }
    }
    patch
}

/// Deletes a contact from every address book holding it, then from
/// Courly. Stops at the first book that refuses, so the contact never
/// disappears from Courly while it still exists somewhere.
pub(crate) async fn delete_contact(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    contact_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("delete_contact", "", contact_id);
    let outcome = match delete(ctx, accounts, contact_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(error) => ActionOutcome::Failed { error },
    };
    mlog.emit(&outcome);
    outcome
}

async fn delete(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    contact_id: &str,
) -> Result<(), ActionError> {
    let id = contact_id.to_owned();
    let (stored, holders) = ctx
        .write_db
        .clone()
        .with_write_mapped(
            move |conn| {
                let stored = get_stored_contact_sync(conn, &id)?
                    .ok_or_else(|| "contact not found".to_owned())?;
                let holders = contact_holders_sync(conn, &stored.email)?;
                Ok((stored, holders))
            },
            std::convert::identity,
        )
        .await
        .map_err(ActionError::db)?;

    for holder in holders_of(&stored, holders) {
        match holder {
            Holder::Account(holder) => {
                let account = accounts
                    .get(&holder.account_id)
                    .await
                    .map_err(ActionError::remote)?;
                account
                    .engine
                    .contact_delete(
                        &AccountId(holder.account_id.clone()),
                        ContactId(holder.server_id.clone()),
                    )
                    .await
                    .map_err(map_engine_error)?;
            }
            Holder::Device(identifier) => delete_device(ctx, identifier).await?,
        }
    }

    let email = stored.email.clone();
    ctx.write_db
        .clone()
        .with_write_mapped(
            move |conn| forget_contact_holders_sync(conn, &email),
            std::convert::identity,
        )
        .await
        .map_err(ActionError::db)?;
    db_delete_contact(&ctx.write_db.writer_pool(), contact_id.to_owned())
        .await
        .map_err(ActionError::db)
}

#[cfg(target_os = "macos")]
async fn delete_device(ctx: &ActionContext, identifier: String) -> Result<(), ActionError> {
    crate::device_contacts::delete(&ctx.write_db, identifier)
        .await
        .map_err(ActionError::remote)
}

#[cfg(not(target_os = "macos"))]
async fn delete_device(_ctx: &ActionContext, _identifier: String) -> Result<(), ActionError> {
    Err(ActionError::not_implemented(
        "the Mac's address book exists only on macOS",
    ))
}

/// Convert a bifrost engine error into an `ActionError` without leaking the
/// raw `AccountError` across the IPC wire: `AccountError`s route through the
/// shared mapper (stable `message_key` + retryable classification), and only
/// engine-internal errors (never attached, shutting down, ...) fall back to a
/// `Display` string.
fn map_engine_error(error: bifrost_sync::Error) -> ActionError {
    match error {
        bifrost_sync::Error::Account(ref account_error) => {
            crate::bifrost::account_error_to_action_error(account_error)
        }
        other => ActionError::remote(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use bifrost_types::{ContactCorpus, ContactProvenance, ProtocolKind};

    use super::*;

    fn card(phones: &[(&str, Option<&str>)], organization: Option<(&str, &str)>) -> ContactCard {
        ContactCard {
            id: ContactId("c1".to_owned()),
            address_book_id: None,
            native_id: "c1".to_owned(),
            etag: None,
            provenance: ContactProvenance {
                provider: ProtocolKind::Graph,
                native: "c1".to_owned(),
                address_book_native: None,
            },
            corpus: ContactCorpus::Main,
            display_name: Some("Ada Lovelace".to_owned()),
            emails: vec![
                ContactEmail {
                    value: "ada@work.test".to_owned(),
                    kind: Some("work".to_owned()),
                    is_primary: true,
                },
                ContactEmail {
                    value: "ada@home.test".to_owned(),
                    kind: Some("home".to_owned()),
                    is_primary: false,
                },
            ],
            phones: phones
                .iter()
                .map(|(value, kind)| ContactPhone {
                    value: (*value).to_owned(),
                    kind: kind.map(str::to_owned),
                    is_primary: false,
                })
                .collect(),
            organizations: organization
                .map(|(name, title)| ContactOrganization {
                    name: name.to_owned(),
                    title: Some(title.to_owned()),
                })
                .into_iter()
                .collect(),
            addresses: Vec::new(),
            notes: None,
            photo_url: None,
            photo: None,
        }
    }

    fn fields() -> ContactFields {
        ContactFields {
            display_name: Some("Ada Lovelace".to_owned()),
            email: "ada@work.test".to_owned(),
            phone: Some("+44 1".to_owned()),
            company: Some("Engines".to_owned()),
            ..ContactFields::default()
        }
    }

    #[test]
    fn a_phone_edit_keeps_the_other_numbers_and_the_job_title() {
        let card = card(
            &[("+44 1", Some("mobile")), ("+44 2", Some("home"))],
            Some(("Engines", "Analyst")),
        );
        let new = ContactFields {
            phone: Some("+44 3".to_owned()),
            ..fields()
        };
        let patch = card_patch(&card, &fields(), &new);
        assert_eq!(
            patch.phones.map(|phones| phones
                .into_iter()
                .map(|phone| (phone.value, phone.kind))
                .collect::<Vec<_>>()),
            Some(vec![
                ("+44 3".to_owned(), Some("mobile".to_owned())),
                ("+44 2".to_owned(), Some("home".to_owned())),
            ])
        );
        assert_eq!(patch.organizations, None);
        assert_eq!(patch.emails, None);
    }

    #[test]
    fn a_company_edit_keeps_the_job_title() {
        let card = card(&[("+44 1", Some("mobile"))], Some(("Engines", "Analyst")));
        let new = ContactFields {
            company: Some("Looms".to_owned()),
            ..fields()
        };
        assert_eq!(
            card_patch(&card, &fields(), &new).organizations,
            Some(vec![ContactOrganization {
                name: "Looms".to_owned(),
                title: Some("Analyst".to_owned()),
            }])
        );
    }

    #[test]
    fn an_address_edit_keeps_the_other_addresses_and_their_labels() {
        let card = card(&[("+44 1", Some("mobile"))], None);
        let new = ContactFields {
            email: "ada@new.test".to_owned(),
            ..fields()
        };
        let emails = card_patch(&card, &fields(), &new)
            .emails
            .expect("the address changed");
        assert_eq!(
            emails
                .iter()
                .map(|email| (
                    email.value.as_str(),
                    email.kind.as_deref(),
                    email.is_primary
                ))
                .collect::<Vec<_>>(),
            [
                ("ada@new.test", Some("work"), true),
                ("ada@home.test", Some("home"), false),
            ]
        );
    }

    #[test]
    fn an_unchanged_contact_sends_nothing() {
        let card = card(&[("+44 1", Some("mobile"))], Some(("Engines", "Analyst")));
        assert_eq!(
            card_patch(&card, &fields(), &fields()),
            ContactPatch::default()
        );
    }
}
