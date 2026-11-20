use db_read::ReadConn;
use db_read::db::queries_extra::{
    get_threads_for_shared_mailbox, get_threads_for_shared_mailbox_starred,
};

/// One account with a thread of its own and two threads of a shared
/// mailbox, one of them starred.
fn shared_mailbox_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "
        CREATE TABLE threads (
            id TEXT, account_id TEXT, subject TEXT, snippet TEXT, last_message_at INTEGER,
            message_count INTEGER, is_read INTEGER, is_starred INTEGER, is_important INTEGER,
            has_attachments INTEGER, is_snoozed INTEGER, snooze_until INTEGER,
            is_pinned INTEGER, is_muted INTEGER, is_flag_complete INTEGER,
            namespace_kind TEXT, namespace_id TEXT, is_chat_thread INTEGER);
        CREATE TABLE thread_folders (account_id TEXT, thread_id TEXT, folder_id TEXT);
        CREATE TABLE messages (
            id TEXT, account_id TEXT, thread_id TEXT, from_name TEXT, from_address TEXT,
            to_addresses TEXT, snippet TEXT, date INTEGER);
        INSERT INTO threads VALUES
            ('own', 'a', 'Own', 's', 300, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0, NULL, NULL, 0),
            ('team-1', 'a', 'Team', 's', 200, 1, 1, 1, 0, 0, 0, NULL, 0, 0, 0,
             'shared', 'box', 0),
            ('team-2', 'a', 'Team', 's', 100, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0,
             'shared', 'box', 0),
            ('other-box', 'a', 'Other', 's', 50, 1, 1, 0, 0, 0, 0, NULL, 0, 0, 0,
             'shared', 'elsewhere', 0);
        INSERT INTO thread_folders VALUES
            ('a', 'own', 'INBOX'), ('a', 'team-1', 'INBOX'), ('a', 'team-2', 'archive'),
            ('a', 'other-box', 'INBOX');
        INSERT INTO messages VALUES
            ('m1', 'a', 'team-1', 'Ann', 'ann@example.com', NULL, 's', 200),
            ('m2', 'a', 'team-2', 'Bob', 'bob@example.com', NULL, 's', 100);
        ",
    )
    .expect("fixtures");
    conn
}

fn ids(threads: &[db_read::db::types::DbThread]) -> Vec<&str> {
    threads.iter().map(|thread| thread.id.as_str()).collect()
}

#[test]
fn a_shared_mailbox_lists_its_own_threads() {
    let conn = shared_mailbox_db();
    let read = ReadConn::from_raw(&conn);

    let all = get_threads_for_shared_mailbox(&read, "a", "box", None, None).expect("all threads");
    assert_eq!(ids(&all), ["team-1", "team-2"]);
    assert_eq!(all[0].from_address.as_deref(), Some("ann@example.com"));

    let inbox = get_threads_for_shared_mailbox(&read, "a", "box", Some("INBOX"), None)
        .expect("inbox threads");
    assert_eq!(ids(&inbox), ["team-1"]);

    let starred =
        get_threads_for_shared_mailbox_starred(&read, "a", "box", None).expect("starred threads");
    assert_eq!(ids(&starred), ["team-1"]);
}
