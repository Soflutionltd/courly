//! UI-side compose-draft write-ahead log.
//!
//! Drafts use a UI-side WAL because the window-close path needs
//! sub-millisecond durability and an async IPC cannot meet that
//! bound. The auto-save tick and the close path both append to
//! `<data_dir>/drafts.wal` synchronously. The Service drains the
//! WAL on next boot via `BootPhase::DrainingDraftWal` before the UI
//! re-reads `local_drafts`. This is the only UI write path that
//! survives Phase 6a's lockdown - the WAL is local file IO, not a
//! SQLite write.
//!
//! Format: append-only NDJSON (one `WalEntry` per line). Each
//! entry carries the `SaveLocalDraftParams` payload plus an epoch
//! millisecond stamp for ordering. The Service's drain replays
//! entries in file order; SQLite's `ON CONFLICT(id) DO UPDATE`
//! makes a duplicate replay a no-op and a partial replay safe to
//! re-run.
//!
//! Crash safety: the writer flushes after every line and calls
//! `sync_all` so a process kill mid-write leaves at most one
//! partial trailing line on disk. The drainer skips any line that
//! fails to parse (logs a warning) and continues with the rest.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use rtsk::db::queries_extra::SaveLocalDraftParams;
use serde::{Deserialize, Serialize};

/// One row in the WAL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// Wall-clock millisecond stamp at append time. Not used for
    /// ordering during the drain (file order is authoritative); kept
    /// for diagnostic purposes in the rotated `*.replayed` archive.
    pub epoch_ms: u64,
    pub params: SaveLocalDraftParams,
}

pub use service_api::WAL_FILENAME;

/// Returns the absolute path of the active WAL.
pub fn wal_path(data_dir: &Path) -> PathBuf {
    data_dir.join(WAL_FILENAME)
}

fn epoch_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Append one entry to the WAL. Returns `Ok(())` once the bytes
/// have been flushed and `sync_all`d to disk; the caller may treat
/// successful return as "the draft is durable."
///
/// Failure means the local filesystem is in trouble (full disk,
/// permission flip on `<data_dir>`); the caller should log and
/// surface a warning. Today's UI logs at error level and keeps the
/// draft `dirty` so the next tick or close attempt retries.
///
/// On the very first append after install, fsync the parent
/// directory after `sync_all` so the WAL's dirent is durable; a
/// subsequent power-loss would otherwise leave the freshly-created
/// file with no directory entry on filesystems with looser ordering
/// (ext4 `data=writeback`, xfs). After the first write the dirent
/// already exists, so we only pay the dir fsync cost once per data
/// dir lifetime.
pub fn append(data_dir: &Path, params: &SaveLocalDraftParams) -> Result<(), String> {
    let entry = WalEntry {
        epoch_ms: epoch_millis_now(),
        params: params.clone(),
    };
    let mut line =
        serde_json::to_string(&entry).map_err(|e| format!("serialize wal entry: {e}"))?;
    line.push('\n');
    let path = wal_path(data_dir);
    let first_creation = !path.exists();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    file.write_all(line.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    file.flush()
        .map_err(|e| format!("flush {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("sync {}: {e}", path.display()))?;
    drop(file);
    #[cfg(unix)]
    if first_creation {
        let dir_handle = std::fs::File::open(data_dir)
            .map_err(|e| format!("open {} for fsync: {e}", data_dir.display()))?;
        dir_handle
            .sync_all()
            .map_err(|e| format!("fsync {}: {e}", data_dir.display()))?;
    }
    #[cfg(not(unix))]
    let _ = first_creation;
    Ok(())
}

/// Drop every entry of the draft `draft_id` from the WAL, so a draft the
/// user deleted is not brought back by the next boot's drain. The file is
/// rewritten beside the WAL and renamed over it, so a crash leaves either
/// the old or the new log, never a half-written one. Lines that do not
/// parse are kept for the drain to report.
pub fn forget(data_dir: &Path, draft_id: &str) -> Result<(), String> {
    let path = wal_path(data_dir);
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let is_forgotten = |line: &str| {
        serde_json::from_str::<WalEntry>(line).is_ok_and(|entry| entry.params.id == draft_id)
    };
    if !body.lines().any(is_forgotten) {
        return Ok(());
    }
    let kept: String = body
        .lines()
        .filter(|line| !is_forgotten(line))
        .map(|line| format!("{line}\n"))
        .collect();

    let temporary = path.with_extension("wal.rewrite");
    let mut file = std::fs::File::create(&temporary)
        .map_err(|e| format!("create {}: {e}", temporary.display()))?;
    file.write_all(kept.as_bytes())
        .map_err(|e| format!("write {}: {e}", temporary.display()))?;
    file.sync_all()
        .map_err(|e| format!("sync {}: {e}", temporary.display()))?;
    drop(file);
    std::fs::rename(&temporary, &path).map_err(|e| format!("replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fresh_params(id: &str) -> SaveLocalDraftParams {
        SaveLocalDraftParams {
            id: id.to_string(),
            account_id: "acct-1".to_string(),
            to_addresses: Some("a@example.com".to_string()),
            cc_addresses: None,
            bcc_addresses: None,
            subject: Some("hello".to_string()),
            body_html: Some("<p>body</p>".to_string()),
            reply_to_message_id: None,
            thread_id: None,
            from_email: Some("me@example.com".to_string()),
            signature_id: None,
            remote_draft_id: None,
            attachments: None,
            signature_separator_index: None,
        }
    }

    #[test]
    fn append_creates_wal_with_one_line_per_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p1 = fresh_params("draft-1");
        let p2 = fresh_params("draft-2");
        append(dir.path(), &p1).expect("first append");
        append(dir.path(), &p2).expect("second append");
        let body = fs::read_to_string(wal_path(dir.path())).expect("read wal");
        assert_eq!(body.lines().count(), 2, "expected two lines, got: {body}");
    }

    #[test]
    fn append_round_trips_through_serde() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = fresh_params("draft-rt");
        append(dir.path(), &p).expect("append");
        let body = fs::read_to_string(wal_path(dir.path())).expect("read wal");
        let entry: WalEntry = serde_json::from_str(body.trim()).expect("parse line");
        assert_eq!(entry.params.id, "draft-rt");
        assert_eq!(entry.params.subject.as_deref(), Some("hello"));
    }

    /// Pin the wire shape against the canonical golden in
    /// `service-api`. The Service-side `draft_wal` test asserts the
    /// same constant; if either side's `WalEntry` /
    /// `SaveLocalDraftParams` shape drifts, that side's test breaks.
    /// To intentionally evolve the wire shape, update both
    /// `DRAFT_WAL_GOLDEN_FIXTURE_JSON` in `service-api` and the
    /// matching params in both crates.
    #[test]
    fn wire_shape_pins_to_service_api_golden() {
        let entry = WalEntry {
            epoch_ms: service_api::DRAFT_WAL_GOLDEN_FIXTURE_EPOCH_MS,
            params: golden_fixture_params(),
        };
        let serialized = serde_json::to_string(&entry).expect("serialize");
        assert_eq!(serialized, service_api::DRAFT_WAL_GOLDEN_FIXTURE_JSON);
    }

    fn golden_fixture_params() -> SaveLocalDraftParams {
        SaveLocalDraftParams {
            id: "draft-fixture".to_string(),
            account_id: "acct-fixture".to_string(),
            to_addresses: Some("to@example.com".to_string()),
            cc_addresses: Some("cc@example.com".to_string()),
            bcc_addresses: Some("bcc@example.com".to_string()),
            subject: Some("fixture subject".to_string()),
            body_html: Some("<p>fixture body</p>".to_string()),
            reply_to_message_id: Some("msg-reply".to_string()),
            thread_id: Some("thread-fixture".to_string()),
            from_email: Some("me@example.com".to_string()),
            signature_id: Some("sig-1".to_string()),
            remote_draft_id: Some("remote-1".to_string()),
            attachments: Some("[]".to_string()),
            signature_separator_index: Some(42),
        }
    }

    #[test]
    fn a_forgotten_draft_leaves_the_wal_and_the_others_stay_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        for id in ["keep-1", "gone", "keep-2", "gone"] {
            append(dir.path(), &fresh_params(id)).expect("append");
        }

        forget(dir.path(), "gone").expect("forget");

        let body = fs::read_to_string(wal_path(dir.path())).expect("read wal");
        let ids: Vec<String> = body
            .lines()
            .map(|line| {
                serde_json::from_str::<WalEntry>(line)
                    .expect("parse line")
                    .params
                    .id
            })
            .collect();
        assert_eq!(ids, ["keep-1", "keep-2"]);
    }

    #[test]
    fn forgetting_without_a_wal_or_a_matching_entry_changes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        forget(dir.path(), "any").expect("no wal yet");
        assert!(!wal_path(dir.path()).exists());

        append(dir.path(), &fresh_params("keep")).expect("append");
        let before = fs::read_to_string(wal_path(dir.path())).expect("read wal");
        forget(dir.path(), "other").expect("forget");
        let after = fs::read_to_string(wal_path(dir.path())).expect("read wal");
        assert_eq!(before, after);
    }

    #[test]
    fn append_appends_existing_file_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..5 {
            let mut p = fresh_params(&format!("draft-{i}"));
            p.subject = Some(format!("msg-{i}"));
            append(dir.path(), &p).expect("append");
        }
        let body = fs::read_to_string(wal_path(dir.path())).expect("read wal");
        let ids: Vec<String> = body
            .lines()
            .map(|l| {
                let entry: WalEntry = serde_json::from_str(l).expect("parse line");
                entry.params.id
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                "draft-0".to_string(),
                "draft-1".to_string(),
                "draft-2".to_string(),
                "draft-3".to_string(),
                "draft-4".to_string(),
            ],
        );
    }
}
