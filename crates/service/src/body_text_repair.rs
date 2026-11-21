//! One-time rewrite of the stored body texts that hold markup.
//!
//! Some senders write HTML in the plain text part of their mails, and the
//! text derived for a mail without a plain text part kept the rules of its
//! style elements. Until `common::rfc822::text_free_of_markup` those were
//! stored as the text of the message, so search found the mail through its
//! tags and style rules and the list previewed CSS. New mail is clean on
//! arrival; this pass brings
//! the mail stored before to the same state: the text in the body store, the
//! preview of the message and of its conversation, and the search document.
//!
//! It runs in the background once per data directory, a chunk at a time, and
//! records that it finished in the body store's `user_version`. Stopped
//! half-way, it starts over on the next boot: a text already rewritten holds
//! no markup and is skipped.

use std::collections::HashMap;

use common::rfc822::{snippet_from_body, text_free_of_markup};
use service_state::{BodyStoreWriteState, SearchWriteHandle, WriteDbState};
use store::body_store::{BodyStoreReadState, MessageBody, compress_body};
use tokio_util::sync::CancellationToken;

/// Messages read from the body store at a time: their HTML is decompressed
/// too, so this bounds the memory of the pass.
const CHUNK_SIZE: usize = 200;
/// `user_version` of a body store whose texts were rewritten. Version 1 left
/// the rules of style elements that carry an attribute in the text.
const REPAIRED_VERSION: i64 = 2;

/// A text to rewrite: the message, and the text read from its HTML part.
type Fix = (String, String);

/// The bodies whose text holds markup, with the text that replaces it.
fn fixes(bodies: Vec<MessageBody>) -> Vec<Fix> {
    bodies
        .into_iter()
        .filter_map(|body| {
            let clean =
                text_free_of_markup(body.body_text.as_deref()?, body.body_html.as_deref()?)?;
            Some((body.message_id, clean))
        })
        .collect()
}

async fn store_texts(body_write: &BodyStoreWriteState, fixes: Vec<Fix>) -> Result<(), String> {
    body_write
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("body text repair tx: {e}"))?;
            for (message_id, text) in &fixes {
                tx.execute(
                    "UPDATE bodies SET body_text = ?1 WHERE message_id = ?2",
                    rusqlite::params![compress_body(text)?, message_id],
                )
                .map_err(|e| format!("body text repair row: {e}"))?;
            }
            tx.commit()
                .map_err(|e| format!("body text repair commit: {e}"))
        })
        .await
}

/// Give each message the preview of its new text, and its conversation too
/// when the conversation was showing the preview of that message.
async fn store_snippets(
    db: &WriteDbState,
    rows: Vec<(String, String, String)>,
) -> Result<(), String> {
    db.with_write(move |conn| {
        for (account_id, message_id, snippet) in &rows {
            let previous: Option<(Option<String>, String)> = conn
                .query_row(
                    "SELECT snippet, thread_id FROM messages WHERE account_id = ?1 AND id = ?2",
                    rusqlite::params![account_id, message_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(format!("body text repair read snippet: {other}")),
                })?;
            let Some((old_snippet, thread_id)) = previous else {
                continue;
            };
            conn.execute(
                "UPDATE messages SET snippet = ?3 WHERE account_id = ?1 AND id = ?2",
                rusqlite::params![account_id, message_id, snippet],
            )
            .map_err(|e| format!("body text repair message snippet: {e}"))?;
            conn.execute(
                "UPDATE threads SET snippet = ?3 \
                 WHERE account_id = ?1 AND id = ?2 AND snippet IS ?4",
                rusqlite::params![account_id, thread_id, snippet, old_snippet],
            )
            .map_err(|e| format!("body text repair thread snippet: {e}"))?;
        }
        Ok(())
    })
    .await
}

async fn version(body_write: &BodyStoreWriteState) -> Result<i64, String> {
    body_write
        .with_conn(|conn| {
            conn.query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(|e| format!("read body store version: {e}"))
        })
        .await
}

/// Rewrite every stored text that holds markup. Returns how many were.
pub(crate) async fn run(
    db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    body_read: &BodyStoreReadState,
    search_write: &SearchWriteHandle,
    cancel: &CancellationToken,
) -> Result<usize, String> {
    if version(body_write).await? >= REPAIRED_VERSION {
        return Ok(0);
    }
    let pairs = db
        .with_read(db::db::queries_extra::select_all_message_ids_for_rebuild)
        .await?;
    let mut repaired = 0;
    for chunk in pairs.chunks(CHUNK_SIZE) {
        if cancel.is_cancelled() {
            return Err("stopped before the end; the next boot starts over".into());
        }
        let message_ids = chunk.iter().map(|(_, id)| id.clone()).collect();
        let found = fixes(body_read.get_batch(message_ids).await?);
        if found.is_empty() {
            continue;
        }
        let account_of: HashMap<&str, &str> = chunk
            .iter()
            .map(|(account_id, message_id)| (message_id.as_str(), account_id.as_str()))
            .collect();
        let snippets: Vec<(String, String, String)> = found
            .iter()
            .filter_map(|(message_id, text)| {
                let account_id = account_of.get(message_id.as_str())?;
                Some((
                    (*account_id).to_owned(),
                    message_id.clone(),
                    snippet_from_body(Some(text)),
                ))
            })
            .collect();
        let touched: Vec<(String, String)> = snippets
            .iter()
            .map(|(account_id, message_id, _)| (account_id.clone(), message_id.clone()))
            .collect();
        repaired += found.len();
        store_texts(body_write, found).await?;
        store_snippets(db, snippets).await?;
        crate::rebuild::rebuild_chunk(db, search_write, body_read, &touched).await?;
    }
    body_write
        .with_conn(|conn| {
            conn.execute_batch(&format!("PRAGMA user_version = {REPAIRED_VERSION}"))
                .map_err(|e| format!("write body store version: {e}"))
        })
        .await?;
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<html><head><meta name=\"viewport\" content=\"x\">\
<style>a { color:#007eff; }</style></head><body><p>Setup done</p></body></html>";

    fn body(id: &str, text: &str) -> MessageBody {
        MessageBody {
            message_id: id.to_owned(),
            body_html: Some(PAGE.to_owned()),
            body_text: Some(text.to_owned()),
        }
    }

    #[tokio::test]
    async fn a_stored_text_holding_markup_is_rewritten_and_a_clean_one_left() {
        let dir = tempfile::tempdir().expect("temp dir");
        let body_write = BodyStoreWriteState::init(dir.path()).expect("body store");
        let body_read = BodyStoreReadState::init(dir.path()).expect("body store reader");
        body_write
            .put_batch(vec![
                body("dirty", PAGE),
                body("clean", "Setup done, thanks"),
            ])
            .await
            .expect("stored");

        let ids = vec!["dirty".to_owned(), "clean".to_owned()];
        let found = fixes(body_read.get_batch(ids.clone()).await.expect("read"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "dirty");
        store_texts(&body_write, found).await.expect("rewritten");

        let after: HashMap<String, MessageBody> = body_read
            .get_batch(ids.clone())
            .await
            .expect("read")
            .into_iter()
            .map(|stored| (stored.message_id.clone(), stored))
            .collect();
        let dirty = after["dirty"].body_text.as_deref().expect("text");
        assert!(dirty.contains("Setup done"), "{dirty:?}");
        assert!(
            !dirty.contains("meta") && !dirty.contains("color"),
            "{dirty:?}"
        );
        assert_eq!(after["dirty"].body_html.as_deref(), Some(PAGE));
        assert_eq!(
            after["clean"].body_text.as_deref(),
            Some("Setup done, thanks")
        );
        // Nothing is left to rewrite: a second pass is a no-op.
        assert!(fixes(body_read.get_batch(ids).await.expect("read")).is_empty());
    }
}
