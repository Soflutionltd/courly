use db_read::ReadConn;
use db_read::db::queries_extra::{
    get_thread_count_scoped, get_threads_scoped, get_unread_count_scoped,
    get_unread_counts_by_folder,
};
use db_read::db::types::AccountScope;

fn mailbox_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "
        CREATE TABLE threads (
            id TEXT, account_id TEXT, subject TEXT, snippet TEXT, last_message_at INTEGER,
            message_count INTEGER, is_read INTEGER, is_starred INTEGER, is_important INTEGER,
            has_attachments INTEGER, is_snoozed INTEGER, snooze_until INTEGER,
            is_pinned INTEGER, is_muted INTEGER, is_flag_complete INTEGER,
            namespace_kind TEXT, is_chat_thread INTEGER);
        CREATE TABLE thread_folders (account_id TEXT, thread_id TEXT, folder_id TEXT);
        CREATE TABLE messages (
            id TEXT, account_id TEXT, thread_id TEXT, from_name TEXT, from_address TEXT,
            snippet TEXT, date INTEGER, is_read INTEGER);
        CREATE TABLE message_folders (account_id TEXT, message_id TEXT, folder_id TEXT);
        INSERT INTO threads VALUES
            ('talk', 'a', 'Talk', 's', 300, 3, 0, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('draft', 'a', 'Draft', 's', 200, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('junk', 'a', 'Junk', 's', 100, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('pinned', 'a', 'Pinned', 's', 90, 1, 1, 0, 0, 0, 0, NULL, 1, 0, 0, NULL, 0),
            ('filed', 'a', 'Filed', 's', 80, 1, 0, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('other', 'b', 'Other', 's', 70, 1, 0, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0);
        INSERT INTO thread_folders VALUES
            ('a', 'talk', 'INBOX'), ('a', 'talk', 'SENT'), ('a', 'draft', 'DRAFT'),
            ('a', 'junk', 'SPAM'), ('a', 'pinned', 'projects'), ('a', 'filed', 'projects'),
            ('b', 'other', 'INBOX');
        INSERT INTO messages VALUES
            ('t1', 'a', 'talk', 'X', 'x@example.com', 's', 100, 0),
            ('t2', 'a', 'talk', 'Me', 'me@example.com', 's', 200, 1),
            ('t3', 'a', 'talk', 'X', 'x@example.com', 's', 300, 0),
            ('d1', 'a', 'draft', 'Me', 'me@example.com', 's', 200, 1),
            ('j1', 'a', 'junk', 'Y', 'y@example.com', 's', 100, 1),
            ('p1', 'a', 'pinned', 'Z', 'z@example.com', 's', 90, 1),
            ('f1', 'a', 'filed', 'Z', 'z@example.com', 's', 80, 0),
            ('o1', 'b', 'other', 'W', 'w@example.com', 's', 70, 0);
        INSERT INTO message_folders VALUES
            ('a', 't1', 'INBOX'), ('a', 't2', 'SENT'), ('a', 't3', 'INBOX'),
            ('a', 'd1', 'DRAFT'), ('a', 'j1', 'SPAM'), ('a', 'p1', 'projects'),
            ('a', 'f1', 'projects'), ('b', 'o1', 'INBOX');
        ALTER TABLE messages ADD COLUMN to_addresses TEXT;
        ",
    )
    .expect("fixtures");
    conn
}

fn count_of(conn: &ReadConn<'_>, scope: &AccountScope, folder_id: &str) -> i64 {
    get_unread_counts_by_folder(conn, scope)
        .expect("folder counts")
        .into_iter()
        .find(|row| row.folder_id == folder_id)
        .map_or(0, |row| row.unread_count.as_i64())
}

#[test]
fn system_folders_count_messages_like_outlook() {
    let conn = mailbox_db();
    let read = ReadConn::from_raw(&conn);
    let scope = AccountScope::Single("a".to_owned());

    assert_eq!(count_of(&read, &scope, "INBOX"), 2);
    assert_eq!(count_of(&read, &scope, "SENT"), 0);
    assert_eq!(count_of(&read, &scope, "DRAFT"), 1);
    assert_eq!(count_of(&read, &scope, "SPAM"), 1);
}

#[test]
fn the_all_accounts_inbox_count_is_the_sum_of_the_inboxes() {
    let conn = mailbox_db();
    let read = ReadConn::from_raw(&conn);

    assert_eq!(count_of(&read, &AccountScope::All, "INBOX"), 3);
    assert_eq!(get_unread_count_scoped(&read, &AccountScope::All), Ok(3));
}

#[test]
fn the_all_accounts_inbox_lists_the_inboxes_and_the_pinned_threads() {
    let conn = mailbox_db();
    let read = ReadConn::from_raw(&conn);

    let threads = get_threads_scoped(&read, &AccountScope::All, Some("INBOX"), None, None)
        .expect("inbox threads");
    let mut ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["other", "pinned", "talk"]);
    assert_eq!(
        get_thread_count_scoped(&read, &AccountScope::All, Some("INBOX")),
        Ok(3)
    );
}
