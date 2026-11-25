//! Contact writes: the local row, then every address book holding the
//! contact (the Mac's address book and the accounts' cards).

use conduit_types::{
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
use crate::conduit::ResidentEngine;
use crate::conduit::resident::ResidentActionAccount;

const LOCAL_SOURCE: &str = "user";
const DEVICE_SOURCE: &str = "macos";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

/// The pending operation that retries an edit an address book did not
/// confirm, and the one that retries a creation.
pub(crate) const RETRY_SAVE_OPERATION: &str = "contactSave";
pub(crate) const RETRY_CREATE_OPERATION: &str = "contactCreate";
/// A contact write is retried until confirmed or this many attempts failed.
const RETRY_MAX: i64 = 10;

/// Reaches the accounts holding a contact.
pub(crate) struct ContactAccounts(pub(crate) Option<ResidentEngine>);

impl ContactAccounts {
    async fn get(&self, account_id: &str) -> Result<ResidentActionAccount, String> {
        match &self.0 {
            Some(engine) => engine.action_account(account_id).await,
            None => Err("the account is not connected yet".to_owned()),
        }
    }
}

/// An edit waiting in the pending queue: what the address books held when
/// it was made, and what they should hold. `old` stays the oldest value a
/// book was last confirmed with, through later edits, because a card's
/// values are edited from what the book holds, not from what Courly shows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PendingEdit {
    old: ContactFields,
    new: ContactFields,
}

/// A creation waiting in the pending queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PendingCreate {
    destination: String,
    input: ContactSaveInput,
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
                let pending = pending_params(conn, RETRY_SAVE_OPERATION, &id)?;
                Ok(Some((stored, holders, pending)))
            },
            std::convert::identity,
        )
        .await
        .map_err(ActionError::db)?;

    match current {
        Some((stored, holders, pending)) => {
            write_local(ctx, &input, stored.account_id.as_deref(), &stored.source).await?;
            // An earlier edit the books have not confirmed is still owed:
            // the books hold its `old`, so that is what this edit starts
            // from. Comparing with the local row instead, already rewritten,
            // took a second save of the same values for a success while
            // nothing had reached the books.
            let old = pending
                .as_deref()
                .and_then(|params| serde_json::from_str::<PendingEdit>(params).ok())
                .map_or_else(|| stored_fields(&stored), |edit| edit.old);
            let new = input.fields();
            if old == new {
                forget_retry(ctx, RETRY_SAVE_OPERATION, &input.id).await;
                return Ok(ActionOutcome::Success);
            }
            let retry_account = first_account(&holders);
            let failures = push_edit(ctx, accounts, &stored, holders, &old, &new).await?;
            let queued = settle(
                ctx,
                RETRY_SAVE_OPERATION,
                &input.id,
                retry_account.as_deref(),
                &failures,
                &PendingEdit { old, new },
            )
            .await;
            Ok(promised(outcome_of(&failures), queued))
        }
        None => {
            write_local(ctx, &input, None, LOCAL_SOURCE).await?;
            let destination = input.source.as_deref().unwrap_or(LOCAL_SOURCE);
            if destination == LOCAL_SOURCE {
                return Ok(ActionOutcome::Success);
            }
            let failures: Vec<BookFailure> = create_in(ctx, accounts, destination, &input)
                .await
                .err()
                .into_iter()
                .collect();
            let queued = settle(
                ctx,
                RETRY_CREATE_OPERATION,
                &input.id,
                input.account_id.as_deref(),
                &failures,
                &PendingCreate {
                    destination: destination.to_owned(),
                    input: input.clone(),
                },
            )
            .await;
            Ok(promised(outcome_of(&failures), queued))
        }
    }
}

/// A refused write is reported as retryable only when a retry is actually
/// queued: the user is told the change is on its way, so it must be.
fn promised(outcome: ActionOutcome, queued: bool) -> ActionOutcome {
    match outcome {
        ActionOutcome::LocalOnly { reason, retryable } => ActionOutcome::LocalOnly {
            reason,
            retryable: retryable && queued,
        },
        other => other,
    }
}

/// Send an edit to every address book holding the contact. Returns the
/// books that refused it; an address change follows the contact's claims
/// in the books that took it.
async fn push_edit(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    stored: &StoredContact,
    holders: ContactHolders,
    old: &ContactFields,
    new: &ContactFields,
) -> Result<Vec<BookFailure>, ActionError> {
    let mut failures = Vec::new();
    let mut updated_cards = Vec::new();
    for holder in holders_of(stored, holders) {
        match update_holder(ctx, accounts, &holder, old, new).await {
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
    Ok(failures)
}

/// The first account among the books holding the contact: the pending
/// queue files a retry under an account.
fn first_account(holders: &ContactHolders) -> Option<String> {
    holders
        .accounts
        .first()
        .map(|holder| holder.account_id.clone())
}

/// After a write: forget any retry when every book took it, or queue one
/// when a book refused it in a way a later attempt can fix. Returns whether
/// a retry is queued.
///
/// The queue files work under an account, so a write that only the Mac's
/// address book refused, with no account involved, is not queued: the
/// outcome still tells the user it is not in that book.
async fn settle(
    ctx: &ActionContext,
    operation: &'static str,
    contact_id: &str,
    account_id: Option<&str>,
    failures: &[BookFailure],
    params: &impl serde::Serialize,
) -> bool {
    if failures.is_empty() {
        forget_retry(ctx, operation, contact_id).await;
        return false;
    }
    if !failures.iter().any(|failure| failure.error.is_retryable()) {
        return false;
    }
    let Some(account_id) = account_id else {
        log::info!("contact {contact_id}: refused write not queued, no account holds it");
        return false;
    };
    let params = match serde_json::to_string(params) {
        Ok(params) => params,
        Err(error) => {
            log::warn!("contact {contact_id}: retry not queued: {error}");
            return false;
        }
    };
    let op_id = uuid::Uuid::new_v4().to_string();
    let account_id = account_id.to_owned();
    let resource = contact_id.to_owned();
    let queued = ctx
        .write_db
        .clone()
        .with_write(move |conn| {
            db::db::pending_ops::db_pending_ops_enqueue_sync(
                conn,
                &op_id,
                &account_id,
                operation,
                &resource,
                &params,
                RETRY_MAX,
            )
        })
        .await;
    match queued {
        Ok(()) => true,
        Err(error) => {
            log::warn!("contact {contact_id}: retry not queued: {error}");
            false
        }
    }
}

/// Drop the queued retries of `operation` for the contact: the write they
/// carried has been confirmed, or replaced by one that has.
async fn forget_retry(ctx: &ActionContext, operation: &'static str, contact_id: &str) {
    let resource = contact_id.to_owned();
    let forgotten = ctx
        .write_db
        .clone()
        .with_write(move |conn| {
            conn.execute(
                "DELETE FROM pending_operations WHERE operation_type = ?1 AND resource_id = ?2",
                rusqlite::params![operation, resource],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
        .await;
    if let Err(error) = forgotten {
        log::warn!("contact {contact_id}: confirmed write still queued: {error}");
    }
}

/// The parameters of the queued `operation` for the contact, if one waits.
fn pending_params(
    conn: &impl db::db::WriteTarget,
    operation: &str,
    contact_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT params FROM pending_operations \
         WHERE operation_type = ?1 AND resource_id = ?2 \
           AND status IN ('pending', 'executing') \
         ORDER BY created_at DESC LIMIT 1",
        rusqlite::params![operation, contact_id],
        |row| row.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.to_string()),
    })
}

/// Retry, from the pending queue, a contact write the address books did not
/// confirm. A contact deleted since has nothing left to send.
pub(crate) async fn retry_contact_write(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    operation: &str,
    contact_id: &str,
    params: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("retry_contact_write", "", contact_id);
    let outcome = match retry(ctx, accounts, operation, contact_id, params).await {
        Ok(outcome) => outcome,
        Err(error) => ActionOutcome::Failed { error },
    };
    mlog.emit(&outcome);
    outcome
}

async fn retry(
    ctx: &ActionContext,
    accounts: &ContactAccounts,
    operation: &str,
    contact_id: &str,
    params: &str,
) -> Result<ActionOutcome, ActionError> {
    let id = contact_id.to_owned();
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
    let Some((stored, holders)) = current else {
        return Ok(ActionOutcome::NoOp);
    };
    if operation == RETRY_SAVE_OPERATION {
        let edit: PendingEdit = serde_json::from_str(params)
            .map_err(|error| ActionError::invalid_state(format!("queued edit: {error}")))?;
        let failures = push_edit(ctx, accounts, &stored, holders, &edit.old, &edit.new).await?;
        return Ok(outcome_of(&failures));
    }
    let create: PendingCreate = serde_json::from_str(params)
        .map_err(|error| ActionError::invalid_state(format!("queued creation: {error}")))?;
    // Already in the book: an earlier attempt recorded it, or a sync
    // brought it in after an attempt whose answer was lost.
    let account_id = create.input.account_id.as_deref();
    let already_held = holders
        .accounts
        .iter()
        .any(|holder| Some(holder.account_id.as_str()) == account_id)
        || (stored.source == create.destination && stored.server_id.is_some());
    if already_held {
        return Ok(ActionOutcome::NoOp);
    }
    if let Some(found) = find_in_book(accounts, &create).await {
        record_origin(ctx, &create, &found).await?;
        return Ok(ActionOutcome::Success);
    }
    let failures: Vec<BookFailure> = create_in(ctx, accounts, &create.destination, &create.input)
        .await
        .err()
        .into_iter()
        .collect();
    Ok(outcome_of(&failures))
}

/// The card an earlier attempt of this creation may have made in the
/// account's address book, found by the contact's address. `None` when
/// the book holds none, or cannot be searched.
async fn find_in_book(accounts: &ContactAccounts, create: &PendingCreate) -> Option<String> {
    let account_id = create.input.account_id.clone()?;
    let account = accounts.get(&account_id).await.ok()?;
    let address = create.input.email.trim().to_lowercase();
    let page = account
        .engine
        .contact_search(
            &AccountId(account_id),
            conduit_types::ContactSearchRequest::new(address.clone()),
        )
        .await
        .ok()?;
    page.items
        .into_iter()
        .find(|card| {
            card.emails
                .iter()
                .any(|email| same_address(&email.value, &address))
        })
        .map(|card| card.id.0)
}

/// Record that the contact lives in its destination book as `server_id`.
async fn record_origin(
    ctx: &ActionContext,
    create: &PendingCreate,
    server_id: &str,
) -> Result<(), ActionError> {
    let id = create.input.id.clone();
    let source = create.destination.clone();
    let account_id = create.input.account_id.clone();
    let server_id = server_id.to_owned();
    let emails = create.input.fields().emails();
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
        .map_err(ActionError::db)
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

/// Convert a conduit engine error into an `ActionError` without leaking the
/// raw `AccountError` across the IPC wire: `AccountError`s route through the
/// shared mapper (stable `message_key` + retryable classification), and only
/// engine-internal errors (never attached, shutting down, ...) fall back to a
/// `Display` string.
fn map_engine_error(error: conduit_sync::Error) -> ActionError {
    match error {
        conduit_sync::Error::Account(ref account_error) => {
            crate::conduit::account_error_to_action_error(account_error)
        }
        other => ActionError::remote(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use conduit_types::{ContactCorpus, ContactProvenance, ProtocolKind};

    use super::*;

    /// A contact held by an Outlook account, with a phone number. No engine
    /// is attached, so every write to the book is refused the way a lost
    /// connection refuses it: transiently.
    fn held_contact() -> (ActionContext, tempfile::TempDir) {
        let (ctx, tmp) = crate::actions::tests::make_test_ctx();
        ctx.write_db
            .with_write_sync(|conn| {
                conn.execute(
                    "INSERT INTO accounts (id, email, provider, is_active) \
                     VALUES ('acc1', 'me@test.example', 'graph', 1)",
                    [],
                )
                .map_err(|error| error.to_string())?;
                db_upsert_contact_full(
                    conn,
                    UpsertContactParams {
                        id: "c1",
                        email: "ada@test.example",
                        display_name: Some("Ada"),
                        email2: None,
                        phone: Some("+44 1"),
                        company: None,
                        notes: None,
                        account_id: Some("acc1"),
                        source: "graph",
                    },
                )?;
                upsert_contact_claim_sync(
                    conn,
                    "acc1",
                    "graph",
                    "card-1",
                    "ada@test.example",
                    None,
                    "main",
                )
            })
            .expect("seed");
        (ctx, tmp)
    }

    fn edit(phone: &str) -> ContactSaveInput {
        ContactSaveInput {
            id: "c1".to_owned(),
            email: "ada@test.example".to_owned(),
            display_name: Some("Ada".to_owned()),
            email2: None,
            phone: Some(phone.to_owned()),
            company: None,
            notes: None,
            account_id: Some("acc1".to_owned()),
            source: Some("graph".to_owned()),
            server_id: None,
        }
    }

    fn queued_edit(ctx: &ActionContext) -> Option<PendingEdit> {
        ctx.write_db
            .with_write_sync(|conn| pending_params(conn, RETRY_SAVE_OPERATION, "c1"))
            .expect("read queue")
            .map(|params| serde_json::from_str(&params).expect("queued edit"))
    }

    fn offline() -> ContactAccounts {
        ContactAccounts(None)
    }

    /// The scenario of the review: the book refuses an edit, the user saves
    /// the same values again once the network is back. The second save must
    /// send the edit again, not report a success because Courly's own row
    /// already holds the new values.
    #[tokio::test]
    async fn saving_the_same_edit_again_sends_it_again() {
        let (ctx, _tmp) = held_contact();

        let first = save_contact(&ctx, &offline(), edit("+44 2")).await;
        assert!(
            matches!(
                first,
                ActionOutcome::LocalOnly {
                    retryable: true,
                    ..
                }
            ),
            "{first:?}"
        );
        let queued = queued_edit(&ctx).expect("the refused edit waits in the queue");
        assert_eq!(queued.old.phone.as_deref(), Some("+44 1"));
        assert_eq!(queued.new.phone.as_deref(), Some("+44 2"));

        let again = save_contact(&ctx, &offline(), edit("+44 2")).await;
        assert!(
            matches!(again, ActionOutcome::LocalOnly { .. }),
            "a save the book has still not taken is not a success: {again:?}"
        );
    }

    /// A second edit made while the first waits keeps what the book was
    /// last confirmed with: the card is edited from that, not from the
    /// intermediate value only Courly ever held.
    #[tokio::test]
    async fn a_later_edit_keeps_what_the_book_last_confirmed() {
        let (ctx, _tmp) = held_contact();
        save_contact(&ctx, &offline(), edit("+44 2")).await;
        save_contact(&ctx, &offline(), edit("+44 3")).await;

        let queued = queued_edit(&ctx).expect("queued");
        assert_eq!(queued.old.phone.as_deref(), Some("+44 1"));
        assert_eq!(queued.new.phone.as_deref(), Some("+44 3"));
    }

    /// Going back to what the book holds leaves nothing to send.
    #[tokio::test]
    async fn undoing_a_refused_edit_clears_its_retry() {
        let (ctx, _tmp) = held_contact();
        save_contact(&ctx, &offline(), edit("+44 2")).await;

        let back = save_contact(&ctx, &offline(), edit("+44 1")).await;
        assert!(matches!(back, ActionOutcome::Success), "{back:?}");
        assert!(queued_edit(&ctx).is_none());
    }

    /// The pending worker retries the queued edit through this pipeline; a
    /// refused retry stays queued for the next round.
    #[tokio::test]
    async fn the_pending_worker_retries_a_queued_edit() {
        let (ctx, _tmp) = held_contact();
        save_contact(&ctx, &offline(), edit("+44 2")).await;

        crate::actions::pending::process_pending_ops(&ctx, None).await;

        let retries: i64 = ctx
            .write_db
            .with_write_sync(|conn| {
                conn.query_row(
                    "SELECT retry_count FROM pending_operations WHERE resource_id = 'c1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())
            })
            .expect("queued");
        assert_eq!(retries, 1, "the worker tried it once and kept it");
    }

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
