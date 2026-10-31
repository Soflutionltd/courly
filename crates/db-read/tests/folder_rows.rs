use db_read::ReadConn;
use db_read::db::queries_extra::get_threads_scoped;
use db_read::db::types::AccountScope;

fn folder_view_db() -> rusqlite::Connection {
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
            snippet TEXT, date INTEGER);
        CREATE TABLE message_folders (account_id TEXT, message_id TEXT, folder_id TEXT);
        INSERT INTO threads VALUES
            ('replied', 'a', 'Re: order', 'my reply', 300, 2, 1, 0, 0, 0, 0, NULL, 0, 0, 0,
             NULL, 0),
            ('fresh', 'a', 'News', 'news', 200, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('moved', 'a', 'Moved', 'moved', 50, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0);
        INSERT INTO thread_folders VALUES
            ('a', 'replied', 'INBOX'), ('a', 'fresh', 'INBOX'), ('a', 'moved', 'INBOX');
        INSERT INTO messages VALUES
            ('in', 'a', 'replied', 'Shop', 'shop@example.com', 'your order', 100),
            ('out', 'a', 'replied', 'Me', 'me@example.com', 'my reply', 300),
            ('news', 'a', 'fresh', 'News', 'news@example.com', 'news', 200),
            ('moved', 'a', 'moved', 'Old', 'old@example.com', 'moved', 50);
        INSERT INTO message_folders VALUES
            ('a', 'in', 'INBOX'), ('a', 'out', 'SENT'), ('a', 'news', 'INBOX'),
            ('a', 'moved', 'archive');
        ",
    )
    .expect("fixtures");
    conn
}

#[test]
fn folder_rows_show_the_latest_message_filed_in_that_folder() {
    let conn = folder_view_db();
    let read = ReadConn::from_raw(&conn);
    let threads = get_threads_scoped(
        &read,
        &AccountScope::Single("a".to_owned()),
        Some("INBOX"),
        None,
        None,
    )
    .expect("inbox threads");

    let order: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
    assert_eq!(order, ["fresh", "replied", "moved"]);
    let replied = &threads[1];
    assert_eq!(replied.from_address.as_deref(), Some("shop@example.com"));
    assert_eq!(replied.snippet.as_deref(), Some("your order"));
    assert_eq!(replied.last_message_at, Some(100));
    let moved = &threads[2];
    assert_eq!(moved.from_address.as_deref(), Some("old@example.com"));
    assert_eq!(moved.last_message_at, Some(50));
}
