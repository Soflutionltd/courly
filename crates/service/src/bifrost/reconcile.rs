//! Periodic Graph mailbox reconcile.
//!
//! The change stream is the only way a server-side deletion reaches Courly.
//! A deletion it never delivers (the account stopped mid-drive, a delta page
//! lost to a crash) leaves a local row the server no longer holds, and nothing
//! ever revisits it. This pass lists every personal mail folder in full,
//! compares the server's ids with the local ones, and removes the local rows
//! the server confirms are gone. It never writes to the server.

use std::collections::HashSet;

use bifrost_sync::SyncEngine;
use bifrost_types::{
    AccountErrorKind, AccountId, FolderId, HydrationProjection, ObjectId, ResourceKind,
};

use super::consumer::write;
use super::containers::ContainerIndex;
use super::{BifrostConsumerStores, BifrostProviderKind};

/// Above this many vanished messages, the pass only proceeds when they are
/// also a small share of the local folders: a listing that suddenly misses
/// most of a mailbox points at a fault, not at the user's deletions.
const UNCHECKED_REMOVAL_FLOOR: usize = 50;
const MAX_REMOVAL_SHARE_PERCENT: usize = 10;
/// One server read per candidate: the rest waits for the next pass rather
/// than burst into the provider's throttling budget.
const MAX_CONFIRMATIONS_PER_PASS: usize = 500;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReconcileReport {
    /// Local messages the server confirmed gone, now deleted locally.
    pub removed: usize,
    /// Server messages Courly does not hold. Reported, not fetched.
    pub missing: usize,
}

pub(crate) async fn reconcile_graph_mail(
    engine: &SyncEngine,
    stores: &BifrostConsumerStores,
    account_id: &str,
    containers: &ContainerIndex,
) -> Result<ReconcileReport, String> {
    let server = GraphMailbox {
        engine,
        account: AccountId(account_id.to_string()),
    };
    reconcile_mail(
        &server,
        stores,
        account_id,
        containers.personal_mail_folders(),
    )
    .await
}

/// What the reconcile needs from the server.
trait MailboxServer {
    /// Every message id the server holds in one folder. Fails unless the
    /// listing is known to be whole: a partial one would make present
    /// messages look deleted.
    async fn list_folder(&self, native: &str) -> Result<HashSet<String>, String>;

    /// True only when the server answers that the message does not exist.
    async fn is_gone(&self, id: &str) -> Result<bool, String>;
}

struct GraphMailbox<'a> {
    engine: &'a SyncEngine,
    account: AccountId,
}

impl MailboxServer for GraphMailbox<'_> {
    async fn list_folder(&self, native: &str) -> Result<HashSet<String>, String> {
        let ids = self
            .engine
            .folder_message_ids(&self.account, FolderId(native.to_string()))
            .await
            .map_err(|error| format!("list {native}: {error}"))?;
        Ok(ids.into_iter().map(|id| id.0).collect())
    }

    async fn is_gone(&self, id: &str) -> Result<bool, String> {
        match self
            .engine
            .message_hydrate(
                &self.account,
                ObjectId(id.to_string()),
                HydrationProjection::Headers,
            )
            .await
        {
            Ok(_) => Ok(false),
            Err(bifrost_sync::Error::Account(error))
                if matches!(
                    error.kind(),
                    AccountErrorKind::NotFound(ResourceKind::Message)
                ) =>
            {
                Ok(true)
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

/// `folders` are `(native id, storage id)` pairs.
async fn reconcile_mail(
    server: &impl MailboxServer,
    stores: &BifrostConsumerStores,
    account_id: &str,
    folders: Vec<(String, String)>,
) -> Result<ReconcileReport, String> {
    if folders.is_empty() {
        return Ok(ReconcileReport::default());
    }
    let mut server_ids = HashSet::new();
    for (native, _) in &folders {
        server_ids.extend(server.list_folder(native).await?);
    }
    let storage_ids: Vec<String> = folders.into_iter().map(|(_, storage)| storage).collect();
    let local_ids = local_message_ids(stores, account_id, storage_ids).await?;

    let mut candidates: Vec<&String> = local_ids
        .iter()
        .filter(|id| !server_ids.contains(*id))
        .collect();
    candidates.sort();
    let missing = server_ids
        .iter()
        .filter(|id| !local_ids.contains(*id))
        .count();
    if !removal_is_plausible(candidates.len(), local_ids.len()) {
        return Err(format!(
            "{} of {} local messages are absent from the server listing; \
             refusing to delete that many",
            candidates.len(),
            local_ids.len()
        ));
    }

    let mut gone = Vec::new();
    for id in candidates.into_iter().take(MAX_CONFIRMATIONS_PER_PASS) {
        match server.is_gone(id).await {
            Ok(true) => gone.push(id.clone()),
            Ok(false) => {}
            // Most likely throttling: the remaining candidates wait for the
            // next pass, the confirmed ones are still removed.
            Err(error) => {
                log::warn!("reconcile {account_id}: could not confirm {id} is gone: {error}");
                break;
            }
        }
    }
    if !gone.is_empty() {
        write::persist(stores, account_id, BifrostProviderKind::Graph, &[], &gone)
            .await
            .map_err(|error| format!("reconcile delete: {error}"))?;
    }
    Ok(ReconcileReport {
        removed: gone.len(),
        missing,
    })
}

async fn local_message_ids(
    stores: &BifrostConsumerStores,
    account_id: &str,
    storage_ids: Vec<String>,
) -> Result<HashSet<String>, String> {
    let account_id = account_id.to_string();
    stores
        .db
        .with_read(move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT DISTINCT message_id FROM message_folders
                     WHERE account_id = ?1 AND folder_id = ?2",
                )
                .map_err(|error| format!("prepare local ids: {error}"))?;
            let mut ids = HashSet::new();
            for folder_id in &storage_ids {
                let rows = statement
                    .query_map(rusqlite::params![account_id, folder_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|error| format!("query local ids: {error}"))?;
                for id in rows {
                    ids.insert(id.map_err(|error| format!("read local id: {error}"))?);
                }
            }
            Ok(ids)
        })
        .await
}

fn removal_is_plausible(removals: usize, local: usize) -> bool {
    removals <= UNCHECKED_REMOVAL_FLOOR
        || removals.saturating_mul(100) <= local.saturating_mul(MAX_REMOVAL_SHARE_PERCENT)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::super::consumer::move_purge_test::{state, stores, synthetic_row};
    use super::*;

    #[derive(Default)]
    struct FakeServer {
        folders: HashMap<String, Result<HashSet<String>, String>>,
        gone: HashSet<String>,
        unreachable: HashSet<String>,
        confirmations: std::sync::atomic::AtomicUsize,
    }

    impl FakeServer {
        fn folder(mut self, native: &str, ids: &[&str]) -> Self {
            let ids = ids.iter().map(|id| (*id).to_string()).collect();
            self.folders.insert(native.to_string(), Ok(ids));
            self
        }

        fn failing_folder(mut self, native: &str) -> Self {
            self.folders
                .insert(native.to_string(), Err("throttled".to_string()));
            self
        }

        fn gone(mut self, ids: &[&str]) -> Self {
            self.gone = ids.iter().map(|id| (*id).to_string()).collect();
            self
        }

        fn unreachable(mut self, ids: &[&str]) -> Self {
            self.unreachable = ids.iter().map(|id| (*id).to_string()).collect();
            self
        }
    }

    impl MailboxServer for FakeServer {
        async fn list_folder(&self, native: &str) -> Result<HashSet<String>, String> {
            self.folders
                .get(native)
                .cloned()
                .ok_or_else(|| format!("unknown folder {native}"))?
        }

        async fn is_gone(&self, id: &str) -> Result<bool, String> {
            self.confirmations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.unreachable.contains(id) {
                return Err("throttled".to_string());
            }
            Ok(self.gone.contains(id))
        }
    }

    fn folders() -> Vec<(String, String)> {
        vec![
            ("mbx-inbox".to_string(), "INBOX".to_string()),
            ("mbx-trash".to_string(), "TRASH".to_string()),
        ]
    }

    async fn message_ids(stores: &BifrostConsumerStores) -> Result<Vec<String>, String> {
        stores
            .db
            .with_read(|conn| {
                let mut statement = conn
                    .prepare("SELECT id FROM messages ORDER BY id")
                    .map_err(|e| e.to_string())?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())
            })
            .await
    }

    async fn seeded(
        ids: &[(&str, &str)],
    ) -> Result<(BifrostConsumerStores, tempfile::TempDir), String> {
        let (db_state, tmp) = state();
        let stores = stores(db_state, &tmp);
        let rows: Vec<_> = ids
            .iter()
            .map(|(id, folder)| synthetic_row(id, &format!("thread-{id}"), &[folder]))
            .collect();
        write::persist(&stores, "acc", BifrostProviderKind::Graph, &rows, &[]).await?;
        Ok((stores, tmp))
    }

    #[tokio::test]
    async fn removes_only_what_the_server_confirms_is_gone() -> Result<(), String> {
        let (stores, _tmp) = seeded(&[
            ("kept", "INBOX"),
            ("ghost", "INBOX"),
            ("unconfirmed", "INBOX"),
            ("trashed", "TRASH"),
        ])
        .await?;
        let server = FakeServer::default()
            .folder("mbx-inbox", &["kept", "new"])
            .folder("mbx-trash", &["trashed"])
            .gone(&["ghost"]);

        let report = reconcile_mail(&server, &stores, "acc", folders()).await?;

        assert_eq!(
            report,
            ReconcileReport {
                removed: 1,
                missing: 1
            }
        );
        assert_eq!(
            message_ids(&stores).await?,
            vec!["kept", "trashed", "unconfirmed"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_incomplete_listing_deletes_nothing() -> Result<(), String> {
        let (stores, _tmp) = seeded(&[("ghost", "INBOX"), ("trashed", "TRASH")]).await?;
        let server = FakeServer::default()
            .folder("mbx-inbox", &[])
            .failing_folder("mbx-trash")
            .gone(&["ghost", "trashed"]);

        assert!(
            reconcile_mail(&server, &stores, "acc", folders())
                .await
                .is_err()
        );
        assert_eq!(message_ids(&stores).await?, vec!["ghost", "trashed"]);
        Ok(())
    }

    #[tokio::test]
    async fn a_listing_missing_most_of_the_mailbox_deletes_nothing() -> Result<(), String> {
        let names: Vec<String> = (0..60).map(|n| format!("m{n:02}")).collect();
        let seeds: Vec<(&str, &str)> = names.iter().map(|id| (id.as_str(), "INBOX")).collect();
        let (stores, _tmp) = seeded(&seeds).await?;
        let all: Vec<&str> = names.iter().map(String::as_str).collect();
        let server = FakeServer::default()
            .folder("mbx-inbox", &[])
            .folder("mbx-trash", &[])
            .gone(&all);

        assert!(
            reconcile_mail(&server, &stores, "acc", folders())
                .await
                .is_err()
        );
        assert_eq!(message_ids(&stores).await?.len(), 60);
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_confirmation_ends_the_pass_but_keeps_what_was_confirmed() -> Result<(), String>
    {
        let (stores, _tmp) = seeded(&[
            ("a-gone", "INBOX"),
            ("b-busy", "INBOX"),
            ("c-gone", "INBOX"),
        ])
        .await?;
        let server = FakeServer::default()
            .folder("mbx-inbox", &[])
            .folder("mbx-trash", &[])
            .gone(&["a-gone", "c-gone"])
            .unreachable(&["b-busy"]);

        let report = reconcile_mail(&server, &stores, "acc", folders()).await?;

        assert_eq!(report.removed, 1);
        assert_eq!(message_ids(&stores).await?, vec!["b-busy", "c-gone"]);
        Ok(())
    }

    #[tokio::test]
    async fn confirmations_stop_at_the_per_pass_budget() -> Result<(), String> {
        let vanished_count = MAX_CONFIRMATIONS_PER_PASS + 10;
        let total = vanished_count * 10;
        let names: Vec<String> = (0..total).map(|n| format!("m{n:05}")).collect();
        let seeds: Vec<(&str, &str)> = names.iter().map(|id| (id.as_str(), "INBOX")).collect();
        let (stores, _tmp) = seeded(&seeds).await?;
        let (vanished, kept) = names.split_at(vanished_count);
        let kept: Vec<&str> = kept.iter().map(String::as_str).collect();
        let vanished: Vec<&str> = vanished.iter().map(String::as_str).collect();
        let server = FakeServer::default()
            .folder("mbx-inbox", &kept)
            .folder("mbx-trash", &[])
            .gone(&vanished);

        let report = reconcile_mail(&server, &stores, "acc", folders()).await?;

        assert_eq!(report.removed, MAX_CONFIRMATIONS_PER_PASS);
        assert_eq!(
            server
                .confirmations
                .load(std::sync::atomic::Ordering::SeqCst),
            MAX_CONFIRMATIONS_PER_PASS
        );
        assert_eq!(
            message_ids(&stores).await?.len(),
            total - MAX_CONFIRMATIONS_PER_PASS
        );
        Ok(())
    }

    #[test]
    fn a_few_removals_always_proceed() {
        assert!(removal_is_plausible(0, 0));
        assert!(removal_is_plausible(50, 60));
    }

    #[test]
    fn mass_removals_proceed_only_as_a_small_share() {
        assert!(removal_is_plausible(100, 1_000));
        assert!(!removal_is_plausible(101, 1_000));
        assert!(!removal_is_plausible(60, 70));
    }
}
