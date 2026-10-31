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
    AccountErrorKind, AccountId, CursorScope, FolderId, HydrationProjection, InventoryEvent,
    ObjectId, ObjectType,
};
use futures_util::StreamExt;

use super::consumer::write;
use super::containers::ContainerIndex;
use super::{BifrostConsumerStores, BifrostProviderKind};

/// Above this many vanished messages, the pass only proceeds when they are
/// also a small share of the local folders: a listing that suddenly misses
/// most of a mailbox points at a fault, not at the user's deletions.
const UNCHECKED_REMOVAL_FLOOR: usize = 50;
const MAX_REMOVAL_SHARE_PERCENT: usize = 10;

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
        let scope = CursorScope::FolderType {
            folder: FolderId(native.to_string()),
            ty: ObjectType::Email,
        };
        let mut stream = self
            .engine
            .inventory_stream(&self.account, scope)
            .map_err(|error| format!("inventory {native}: {error}"))?;
        let mut ids = HashSet::new();
        while let Some(event) = stream.next().await {
            match event {
                InventoryEvent::Batch(batch) => {
                    ids.extend(batch.items.into_iter().map(|entry| entry.id.0));
                }
                InventoryEvent::Done(completion) if completion.coverage.is_complete() => {
                    return Ok(ids);
                }
                InventoryEvent::Done(_) => {
                    return Err(format!("inventory {native}: listing incomplete"));
                }
                InventoryEvent::Terminated(error) => {
                    return Err(format!("inventory {native}: {error}"));
                }
                _ => {}
            }
        }
        Err(format!(
            "inventory {native}: listing ended without completion"
        ))
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
                if matches!(error.kind(), AccountErrorKind::NotFound(_)) =>
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
    for id in candidates {
        match server.is_gone(id).await {
            Ok(true) => gone.push(id.clone()),
            Ok(false) => {}
            Err(error) => {
                log::warn!("reconcile {account_id}: could not confirm {id} is gone: {error}");
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
        .with_write(move |conn| {
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
    }

    impl MailboxServer for FakeServer {
        async fn list_folder(&self, native: &str) -> Result<HashSet<String>, String> {
            self.folders
                .get(native)
                .cloned()
                .ok_or_else(|| format!("unknown folder {native}"))?
        }

        async fn is_gone(&self, id: &str) -> Result<bool, String> {
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
