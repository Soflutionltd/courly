use db_read::ReadConn;
use db_read::db::queries_extra::{ContactOrigins, search_people_for_typeahead_sync};

fn people_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "
        CREATE TABLE contacts (
            email TEXT, display_name TEXT, email2 TEXT, company TEXT, last_contacted_at INTEGER,
            source TEXT);
        CREATE TABLE seen_addresses (
            email TEXT, display_name TEXT, last_seen_at INTEGER);
        CREATE TABLE contact_claims (
            account_id TEXT, source TEXT, server_id TEXT, email TEXT, corpus TEXT);
        CREATE TABLE device_contact_claims (server_id TEXT, email TEXT);
        INSERT INTO contacts VALUES
            ('florian.koenig@noacontemporary.com', 'Florian Paul Koenig', NULL, NULL, NULL,
             'graph'),
            ('florian@other.example', 'Florian Christ', NULL, 'Atelier', 10, 'user');
        INSERT INTO contact_claims VALUES
            ('acct', 'graph', 'g1', 'florian.koenig@noacontemporary.com', 'main');
        INSERT INTO device_contact_claims VALUES
            ('mac1', 'florian.koenig@noacontemporary.com');
        INSERT INTO seen_addresses VALUES
            ('florent.villain@maif.fr', NULL, 500),
            ('Florian.Koenig@noacontemporary.com', 'Florian K', 900);
        ",
    )
    .expect("fixtures");
    conn
}

fn emails(conn: &rusqlite::Connection, query: &str) -> Vec<String> {
    search_people_for_typeahead_sync(&ReadConn::from_raw(conn), query)
        .expect("people")
        .into_iter()
        .map(|row| row.email)
        .collect()
}

#[test]
fn every_word_must_match_the_name_the_address_or_the_company() {
    let conn = people_db();
    assert_eq!(
        emails(&conn, "florian contemporary"),
        ["florian.koenig@noacontemporary.com"]
    );
    assert_eq!(emails(&conn, "florian atelier"), ["florian@other.example"]);
}

#[test]
fn contacts_come_first_and_each_address_appears_once() {
    let conn = people_db();
    assert_eq!(
        emails(&conn, "flor"),
        [
            "florian@other.example",
            "florian.koenig@noacontemporary.com",
            "florent.villain@maif.fr",
        ]
    );
}

#[test]
fn a_suggestion_names_every_address_book_holding_the_person() {
    let conn = people_db();
    let origins: Vec<(String, ContactOrigins)> =
        search_people_for_typeahead_sync(&ReadConn::from_raw(&conn), "flor")
            .expect("people")
            .into_iter()
            .map(|row| (row.email, row.origins))
            .collect();
    assert_eq!(
        origins,
        [
            (
                "florian@other.example".to_owned(),
                ContactOrigins {
                    courly: true,
                    ..ContactOrigins::default()
                }
            ),
            (
                "florian.koenig@noacontemporary.com".to_owned(),
                ContactOrigins {
                    apple: true,
                    microsoft: true,
                    ..ContactOrigins::default()
                }
            ),
            (
                "florent.villain@maif.fr".to_owned(),
                ContactOrigins::default()
            ),
        ]
    );
}

#[test]
fn a_blank_query_suggests_nobody() {
    let conn = people_db();
    assert!(emails(&conn, "   ").is_empty());
}
