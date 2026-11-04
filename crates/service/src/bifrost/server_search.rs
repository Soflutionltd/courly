//! Server-side search for the mail exchanged with one person.
//!
//! The local store only holds the sync window, so a search for a person
//! misses their older mail. This pass asks Graph for every message the
//! address took part in (sender, to, cc or bcc), then fetches and stores the
//! ones Courly does not hold, so the local search finds them. It never
//! writes to the server.

use std::collections::{HashMap, HashSet};

use bifrost_sync::SyncEngine;
use bifrost_types::{AccountId, Change, ObjectChange, ObjectChangeKind, ObjectId, SearchRequest};
use common::types::FolderKind;

use super::consumer::hydrate::{ConsumerMessageRow, HydrateBatch};
use super::consumer::write;
use super::{BifrostConsumerStores, BifrostProviderKind};

/// Graph's largest `$top` for a message search.
const PAGE_SIZE: u32 = 250;
const MAX_PAGES: usize = 8;
/// Each fetch is a few requests: a person with a very long history fills in
/// over several searches instead of in one burst against the throttling
/// budget.
const MAX_FETCHES_PER_SEARCH: usize = 300;
/// Bifrost qualifies a shared mailbox's message ids with this separator.
/// Courly does not store shared-mailbox mail locally, so those hits are
/// left on the server.
const SHARED_MAILBOX_ID_SEPARATOR: char = '\u{1f}';

pub(crate) async fn fetch_graph_participant_mail(
    engine: &SyncEngine,
    stores: &BifrostConsumerStores,
    account_id: &str,
    folder_map: &HashMap<String, FolderKind>,
    address: &str,
) -> Result<usize, String> {
    let server = GraphSearch {
        engine,
        account: AccountId(account_id.to_string()),
        folder_map,
    };
    fetch_participant_mail(&server, stores, account_id, address).await
}

/// What the pass needs from the server.
trait SearchServer {
    /// One page of message ids the address took part in, with the cursor of
    /// the next page.
    async fn search_page(
        &self,
        address: &str,
        cursor: Option<Vec<u8>>,
    ) -> Result<(Vec<String>, Option<Vec<u8>>), String>;

    /// The rows for messages the server holds. A message that could not be
    /// read is left out.
    async fn fetch(&self, ids: &[String]) -> Result<Vec<ConsumerMessageRow>, String>;
}

struct GraphSearch<'a> {
    engine: &'a SyncEngine,
    account: AccountId,
    folder_map: &'a HashMap<String, FolderKind>,
}

impl SearchServer for GraphSearch<'_> {
    async fn search_page(
        &self,
        address: &str,
        cursor: Option<Vec<u8>>,
    ) -> Result<(Vec<String>, Option<Vec<u8>>), String> {
        let mut request = SearchRequest::provider(format!("participants:{address}"));
        request.limit = Some(PAGE_SIZE);
        request.page_cursor = cursor;
        let page = self
            .engine
            .search_messages(&self.account, request)
            .await
            .map_err(|error| format!("search {address}: {error}"))?;
        Ok((
            page.items.into_iter().map(|id| id.0).collect(),
            page.next_cursor,
        ))
    }

    async fn fetch(&self, ids: &[String]) -> Result<Vec<ConsumerMessageRow>, String> {
        let changes: Vec<Change> = ids
            .iter()
            .map(|id| {
                Change::ObjectChange(ObjectChange {
                    id: ObjectId(id.clone()),
                    kind: ObjectChangeKind::Created,
                })
            })
            .collect();
        let batch = HydrateBatch::from_changes(
            self.engine,
            &self.account,
            BifrostProviderKind::Graph,
            self.folder_map,
            &changes,
        )
        .await
        .map_err(|error| format!("fetch search hits: {error}"))?;
        Ok(batch.rows)
    }
}

/// How many messages were fetched and stored.
async fn fetch_participant_mail(
    server: &impl SearchServer,
    stores: &BifrostConsumerStores,
    account_id: &str,
    address: &str,
) -> Result<usize, String> {
    let mut hits = Vec::new();
    let mut cursor = None;
    for _ in 0..MAX_PAGES {
        let (ids, next) = server.search_page(address, cursor).await?;
        hits.extend(
            ids.into_iter()
                .filter(|id| !id.contains(SHARED_MAILBOX_ID_SEPARATOR)),
        );
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let held = held_message_ids(stores, account_id, hits.clone()).await?;
    let mut seen = HashSet::new();
    let missing: Vec<String> = hits
        .into_iter()
        .filter(|id| !held.contains(id) && seen.insert(id.clone()))
        .take(MAX_FETCHES_PER_SEARCH)
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let rows = server.fetch(&missing).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    write::persist(stores, account_id, BifrostProviderKind::Graph, &rows, &[])
        .await
        .map_err(|error| format!("store search hits: {error}"))?;
    stores.search.flush_now().await?;
    Ok(rows.len())
}

async fn held_message_ids(
    stores: &BifrostConsumerStores,
    account_id: &str,
    ids: Vec<String>,
) -> Result<HashSet<String>, String> {
    let account_id = account_id.to_string();
    stores
        .db
        .with_read(move |conn| {
            let mut statement = conn
                .prepare("SELECT 1 FROM messages WHERE account_id = ?1 AND id = ?2")
                .map_err(|error| format!("prepare held ids: {error}"))?;
            let mut held = HashSet::new();
            for id in ids {
                let exists = statement
                    .query(rusqlite::params![account_id, id])
                    .map_err(|error| format!("query held ids: {error}"))?
                    .next()
                    .map_err(|error| format!("read held ids: {error}"))?
                    .is_some();
                if exists {
                    held.insert(id);
                }
            }
            Ok(held)
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::consumer::move_purge_test::{state, stores, synthetic_row};
    use super::*;

    #[derive(Default)]
    struct FakeServer {
        pages: Vec<Vec<String>>,
        searches: AtomicUsize,
        fetched: std::sync::Mutex<Vec<String>>,
    }

    impl FakeServer {
        fn pages(pages: &[&[&str]]) -> Self {
            Self {
                pages: pages
                    .iter()
                    .map(|page| page.iter().map(|id| (*id).to_string()).collect())
                    .collect(),
                ..Self::default()
            }
        }

        fn fetched(&self) -> Vec<String> {
            self.fetched
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl SearchServer for FakeServer {
        async fn search_page(
            &self,
            _address: &str,
            cursor: Option<Vec<u8>>,
        ) -> Result<(Vec<String>, Option<Vec<u8>>), String> {
            self.searches.fetch_add(1, Ordering::SeqCst);
            let index = cursor.map_or(0, |bytes| usize::from(bytes[0]));
            let page = self.pages.get(index).cloned().unwrap_or_default();
            let next = (index + 1 < self.pages.len())
                .then(|| vec![u8::try_from(index + 1).expect("few pages")]);
            Ok((page, next))
        }

        async fn fetch(&self, ids: &[String]) -> Result<Vec<ConsumerMessageRow>, String> {
            self.fetched
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(ids.iter().cloned());
            Ok(ids
                .iter()
                .map(|id| synthetic_row(id, &format!("thread-{id}"), &["INBOX"]))
                .collect())
        }
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

    #[tokio::test]
    async fn fetches_only_the_hits_not_held_locally() -> Result<(), String> {
        let (db_state, tmp) = state();
        let stores = stores(db_state, &tmp);
        let held = vec![synthetic_row("recent", "thread-recent", &["INBOX"])];
        write::persist(&stores, "acc", BifrostProviderKind::Graph, &held, &[]).await?;
        let server = FakeServer::pages(&[&["recent", "old-1"], &["old-2", "old-1"]]);

        let fetched = fetch_participant_mail(&server, &stores, "acc", "person@example.org").await?;

        assert_eq!(fetched, 2);
        assert_eq!(server.fetched(), ["old-1", "old-2"]);
        assert_eq!(server.searches.load(Ordering::SeqCst), 2);
        assert_eq!(message_ids(&stores).await?, ["old-1", "old-2", "recent"]);
        Ok(())
    }

    #[tokio::test]
    async fn leaves_shared_mailbox_hits_on_the_server() -> Result<(), String> {
        let (db_state, tmp) = state();
        let stores = stores(db_state, &tmp);
        let shared = format!("team@example.org{SHARED_MAILBOX_ID_SEPARATOR}old");
        let server = FakeServer::pages(&[&[shared.as_str(), "mine"]]);

        let fetched = fetch_participant_mail(&server, &stores, "acc", "person@example.org").await?;

        assert_eq!(fetched, 1);
        assert_eq!(server.fetched(), ["mine"]);
        Ok(())
    }

    #[tokio::test]
    async fn nothing_missing_fetches_nothing() -> Result<(), String> {
        let (db_state, tmp) = state();
        let stores = stores(db_state, &tmp);
        let server = FakeServer::pages(&[&[]]);

        let fetched = fetch_participant_mail(&server, &stores, "acc", "person@example.org").await?;

        assert_eq!(fetched, 0);
        assert!(server.fetched().is_empty());
        Ok(())
    }
}
