use db_read::ReadConn;
use db_read::db::queries_extra::{get_outbox_message_count, get_threads_scoped};
use db_read::db::types::AccountScope;
use types::outbox::OUTBOX_VIEW_ID;

/// Two accounts with an outbox each, named in two languages and filed under
/// ids of the provider, plus a third account without one.
fn mailbox_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "
        CREATE TABLE folders (account_id TEXT, id TEXT, name TEXT);
        CREATE TABLE threads (
            id TEXT, account_id TEXT, subject TEXT, snippet TEXT, last_message_at INTEGER,
            message_count INTEGER, is_read INTEGER, is_starred INTEGER, is_important INTEGER,
            has_attachments INTEGER, is_snoozed INTEGER, snooze_until INTEGER,
            is_pinned INTEGER, is_muted INTEGER, is_flag_complete INTEGER,
            namespace_kind TEXT, is_chat_thread INTEGER);
        CREATE TABLE thread_folders (account_id TEXT, thread_id TEXT, folder_id TEXT);
        CREATE TABLE messages (
            id TEXT, account_id TEXT, thread_id TEXT, from_name TEXT, from_address TEXT,
            to_addresses TEXT, snippet TEXT, date INTEGER, is_read INTEGER);
        CREATE TABLE message_folders (account_id TEXT, message_id TEXT, folder_id TEXT);
        INSERT INTO folders VALUES
            ('a', 'INBOX', 'Inbox'), ('a', 'g-out-a', 'Boîte d''envoi'),
            ('b', 'INBOX', 'Inbox'), ('b', 'g-out-b', 'Outbox'), ('b', 'g-work', 'Work'),
            ('c', 'INBOX', 'Inbox');
        INSERT INTO threads VALUES
            ('wait-a', 'a', 'Quote', 's', 300, 2, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('wait-b', 'b', 'Invoice', 's', 400, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('work', 'b', 'Work', 's', 500, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0),
            ('read', 'c', 'Read', 's', 600, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, 0);
        INSERT INTO thread_folders VALUES
            ('a', 'wait-a', 'INBOX'), ('a', 'wait-a', 'g-out-a'),
            ('b', 'wait-b', 'g-out-b'), ('b', 'work', 'g-work'), ('c', 'read', 'INBOX');
        INSERT INTO messages VALUES
            ('in', 'a', 'wait-a', 'Shop', 'shop@example.com', NULL, 'question', 350, 1),
            ('out-a', 'a', 'wait-a', 'Me', 'me@example.com', 'Shop <shop@example.com>',
             'answer', 300, 1),
            ('out-b', 'b', 'wait-b', 'Me', 'me@example.org', 'Client <client@example.com>',
             'invoice', 400, 1),
            ('w', 'b', 'work', 'Boss', 'boss@example.com', NULL, 'work', 500, 0),
            ('r', 'c', 'read', 'News', 'news@example.com', NULL, 'news', 600, 0);
        INSERT INTO message_folders VALUES
            ('a', 'in', 'INBOX'), ('a', 'out-a', 'g-out-a'), ('b', 'out-b', 'g-out-b'),
            ('b', 'w', 'g-work'), ('c', 'r', 'INBOX');
        ",
    )
    .expect("fixtures");
    conn
}

#[test]
fn the_outbox_view_gathers_the_outbox_of_every_account() {
    let conn = mailbox_db();
    let read = ReadConn::from_raw(&conn);
    let threads = get_threads_scoped(&read, &AccountScope::All, Some(OUTBOX_VIEW_ID), None, None)
        .expect("outbox threads");

    let order: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
    assert_eq!(order, ["wait-b", "wait-a"]);
    // The row shows the message that waits, not the newer one received since.
    let waiting = &threads[1];
    assert_eq!(waiting.snippet.as_deref(), Some("answer"));
    assert_eq!(waiting.last_message_at, Some(300));
    assert_eq!(
        waiting.to_addresses.as_deref(),
        Some("Shop <shop@example.com>")
    );
}

#[test]
fn the_outbox_counts_every_waiting_message_read_or_not() {
    let conn = mailbox_db();
    let read = ReadConn::from_raw(&conn);

    assert_eq!(
        get_outbox_message_count(&read, &AccountScope::All).expect("count"),
        Some(2)
    );
    assert_eq!(
        get_outbox_message_count(&read, &AccountScope::Single("a".to_owned())).expect("count"),
        Some(1)
    );
}

#[test]
fn accounts_without_an_outbox_have_no_outbox_view() {
    let conn = mailbox_db();
    let read = ReadConn::from_raw(&conn);
    let scope = AccountScope::Single("c".to_owned());

    assert_eq!(
        get_outbox_message_count(&read, &scope).expect("count"),
        None
    );
    assert!(
        get_threads_scoped(&read, &scope, Some(OUTBOX_VIEW_ID), None, None)
            .expect("outbox threads")
            .is_empty()
    );
}
