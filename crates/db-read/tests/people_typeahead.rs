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
fn people_are_listed_by_name_and_each_address_appears_once() {
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
fn a_name_starting_with_the_typed_letters_comes_before_one_only_holding_them() {
    let conn = people_db();
    conn.execute_batch(
        "
        INSERT INTO contacts VALUES
            ('jo@example.com', 'Jonathan Example', NULL, NULL, 900, 'macos'),
            ('print@example.com', 'Imprimerie Signature', NULL, NULL, 800, 'macos'),
            ('jf@dream-international.example', 'Jean Example', NULL, NULL, 700, 'macos');
        INSERT INTO seen_addresses VALUES
            ('nathalie.example@example.org', 'Nathalie Example', 100),
            ('contact@example.net', 'Service Natation', 50),
            ('nat@example.io', NULL, 10);
        ",
    )
    .expect("fixtures");
    assert_eq!(
        emails(&conn, "NAT"),
        [
            // A word of the name or of the address starts with the letters.
            "nathalie.example@example.org",
            "contact@example.net",
            "nat@example.io",
            // The letters only sit inside a word: by name, Imprimerie,
            // Jean, Jonathan.
            "print@example.com",
            "jf@dream-international.example",
            "jo@example.com",
        ]
    );
}

fn people_named_julie() -> rusqlite::Connection {
    let conn = people_db();
    conn.execute_batch(
        "
        INSERT INTO contacts VALUES
            ('debhora@example.com', 'Debhora Saint Julien', NULL, NULL, 900, 'macos'),
            ('julie.rolet@example.com', 'Julie Rolet', NULL, NULL, 100, 'macos');
        INSERT INTO seen_addresses VALUES
            ('j.reibell@example.org', 'Julie Reibell', 500),
            ('roletjulie@example.net', 'Julie Rolet (via Docs)', 400),
            ('martin@example.net', 'Julie Martin', 300);
        ",
    )
    .expect("fixtures");
    conn
}

#[test]
fn a_name_starting_with_what_was_typed_comes_before_one_holding_it_further_on() {
    let conn = people_named_julie();
    assert_eq!(
        emails(&conn, "julie"),
        [
            // Every Julie, by name: Martin, Reibell, Rolet, Rolet (via Docs).
            "martin@example.net",
            "j.reibell@example.org",
            "julie.rolet@example.com",
            "roletjulie@example.net",
            // "Julien" starts with the letters, but it is not her first name.
            "debhora@example.com",
        ]
    );
}

#[test]
fn a_letter_typed_after_a_first_name_is_the_start_of_another_word() {
    let conn = people_named_julie();
    // Not only the person whose address happens to start with "r".
    assert_eq!(
        emails(&conn, "julie r"),
        [
            "j.reibell@example.org",
            "julie.rolet@example.com",
            "roletjulie@example.net",
        ]
    );
    assert_eq!(emails(&conn, "julie re"), ["j.reibell@example.org"]);
}

#[test]
fn a_blank_query_suggests_nobody() {
    let conn = people_db();
    assert!(emails(&conn, "   ").is_empty());
}
