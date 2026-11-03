//! Unified contact search across contacts, GAL cache, seen addresses,
//! and contact groups. Used by compose autocomplete and calendar attendee
//! fields.

use rusqlite::params;

use crate::db::ReadConn;
use crate::db::sql_fragments::{build_fts_query, make_like_pattern};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactSearchKind {
    /// A regular contact (synced or local).
    Contact,
    /// An address observed in message headers (lower priority).
    SeenAddress,
    /// A contact group (can be expanded into individual addresses).
    Group { group_id: String, member_count: i64 },
}

/// A single result from a contact/autocomplete search.
///
/// Used by compose recipient fields and calendar attendee fields.
/// Deduplicates by email across contacts, seen addresses, and groups.
#[derive(Debug, Clone)]
pub struct ContactSearchResult {
    /// Email address (empty for group results).
    pub email: String,
    /// Display name.
    pub display_name: Option<String>,
    /// What kind of result this is.
    pub kind: ContactSearchKind,
    /// Source identifier (e.g. "google", "graph", "carddav", "user", "seen").
    pub source: Option<String>,
    pub origins: ContactOrigins,
}

/// The address books holding a contact's address. One address can sit in
/// several at once: the Mac's address book and an Outlook account, say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContactOrigins {
    /// The Mac's address book (iCloud and the accounts added to Contacts).
    pub apple: bool,
    /// A Microsoft account, or its organization directory.
    pub microsoft: bool,
    /// A Google account, or its organization directory.
    pub google: bool,
    /// A CardDAV or JMAP address book.
    pub other_book: bool,
    /// Written in Courly and kept only here.
    pub courly: bool,
}

/// Columns `ContactOrigins::from_row` reads, for the contact aliased `c`.
pub const CONTACT_ORIGIN_COLUMNS: &str = "c.source AS origin_source,
    (SELECT group_concat(DISTINCT cc.source) FROM contact_claims cc
      WHERE cc.email = c.email AND cc.corpus = 'main') AS origin_claims,
    EXISTS (SELECT 1 FROM device_contact_claims d WHERE d.email = c.email) AS origin_device";

impl ContactOrigins {
    /// Reads the columns of `CONTACT_ORIGIN_COLUMNS`.
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let mut origins = Self::default();
        if let Some(source) = row.get::<_, Option<String>>("origin_source")? {
            origins.add_source(&source);
        }
        if let Some(claims) = row.get::<_, Option<String>>("origin_claims")? {
            for source in claims.split(',') {
                origins.add_source(source);
            }
        }
        origins.apple |= row.get::<_, bool>("origin_device")?;
        Ok(origins)
    }

    /// Records the address book a `contacts.source` or claim source names.
    pub fn add_source(&mut self, source: &str) {
        match source {
            "macos" => self.apple = true,
            "graph" => self.microsoft = true,
            "google" => self.google = true,
            "jmap" | "carddav" => self.other_book = true,
            "user" => self.courly = true,
            _ => {}
        }
    }

    /// The origin of a directory entry cached for an account of `provider`.
    #[must_use]
    pub fn of_directory(provider: &str) -> Self {
        let mut origins = Self::default();
        match provider {
            "graph" => origins.microsoft = true,
            "gmail_api" => origins.google = true,
            _ => origins.other_book = true,
        }
        origins
    }
}

// ---------------------------------------------------------------------------
// Search-field suggestions
// ---------------------------------------------------------------------------

/// A person suggested while typing a search or an address.
#[derive(Debug, Clone)]
pub struct PersonTypeaheadRow {
    pub email: String,
    pub display_name: Option<String>,
    pub origins: ContactOrigins,
}

const PEOPLE_TYPEAHEAD_LIMIT: i64 = 10;
/// Words past this many are ignored, keeping the statement small.
const PEOPLE_TYPEAHEAD_MAX_WORDS: usize = 6;

/// People matching every typed word in their name, address or company, as
/// Outlook suggests them: "florian contemporary" finds Florian Koenig at
/// noacontemporary.com. Address-book contacts come first, then addresses
/// seen in mail; each address appears once, the most recent first.
pub fn search_people_for_typeahead_sync(
    conn: &ReadConn<'_>,
    query: &str,
) -> Result<Vec<PersonTypeaheadRow>, String> {
    let patterns: Vec<String> = query
        .split_whitespace()
        .take(PEOPLE_TYPEAHEAD_MAX_WORDS)
        .map(make_like_pattern)
        .collect();
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let every_word = |columns: &[&str]| {
        (1..=patterns.len())
            .map(|idx| {
                let any_column = columns
                    .iter()
                    .map(|column| format!("{column} LIKE ?{idx} ESCAPE '\\' COLLATE NOCASE"))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                format!("({any_column})")
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let sql = format!(
        "SELECT email, display_name, origin_source, origin_claims, origin_device FROM (
           SELECT email, display_name, rank, recency,
                  origin_source, origin_claims, origin_device,
                  ROW_NUMBER() OVER (
                    PARTITION BY lower(email) ORDER BY rank, recency DESC
                  ) AS rn
           FROM (
             SELECT c.email, c.display_name, 0 AS rank,
                    COALESCE(c.last_contacted_at, 0) AS recency,
                    {CONTACT_ORIGIN_COLUMNS}
             FROM contacts c
             WHERE {contact_match}
             UNION ALL
             SELECT s.email, s.display_name, 1 AS rank, s.last_seen_at AS recency,
                    NULL, NULL, 0
             FROM seen_addresses s
             WHERE {seen_match}
           )
         )
         WHERE rn = 1
         ORDER BY rank, recency DESC
         LIMIT {PEOPLE_TYPEAHEAD_LIMIT}",
        contact_match = every_word(&["c.display_name", "c.email", "c.email2", "c.company"]),
        seen_match = every_word(&["s.display_name", "s.email"]),
    );
    let params: Vec<&dyn rusqlite::types::ToSql> = patterns
        .iter()
        .map(|p| p as &dyn rusqlite::types::ToSql)
        .collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(params.as_slice(), |row| {
        Ok(PersonTypeaheadRow {
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            origins: ContactOrigins::from_row(row)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Unified search
// ---------------------------------------------------------------------------

/// Search contacts, GAL cache, seen addresses, and groups for autocomplete.
///
/// Uses FTS5 prefix matching for the contacts and seen_addresses tables
/// (with LIKE fallback if the FTS5 tables are unavailable). GAL cache uses
/// LIKE-only matching.
/// Deduplicates by email with priority order:
/// contacts -> GAL cache -> seen addresses -> groups.
/// Returns up to `limit` results.
pub fn search_contacts_unified(
    conn: &ReadConn<'_>,
    query: &str,
    limit: i64,
) -> Result<Vec<ContactSearchResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let like_pattern = make_like_pattern(trimmed);
    let mut results = Vec::new();
    let mut seen_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Search contacts table (highest priority) - FTS5 with LIKE fallback.
    search_contacts_fts_or_like(
        conn,
        trimmed,
        &like_pattern,
        limit,
        &mut seen_emails,
        &mut results,
    )?;

    // 2. Search GAL cache (second priority, after synced contacts).
    let gal_remaining = limit - i64::try_from(results.len()).unwrap_or(i64::MAX);
    if gal_remaining > 0 {
        search_gal_cache(
            conn,
            &like_pattern,
            gal_remaining,
            &mut seen_emails,
            &mut results,
        )?;
    }

    // 3. Search seen_addresses table (lower priority, fills remaining) - FTS5 with LIKE fallback.
    let remaining = limit - i64::try_from(results.len()).unwrap_or(i64::MAX);
    if remaining > 0 {
        search_seen_addresses_fts_or_like(
            conn,
            trimmed,
            &like_pattern,
            remaining,
            &mut seen_emails,
            &mut results,
        )?;
    }

    // 4. Search contact groups by name.
    let group_remaining = limit - i64::try_from(results.len()).unwrap_or(i64::MAX);
    if group_remaining > 0 {
        search_groups_table(conn, &like_pattern, group_remaining, &mut results)?;
    }

    Ok(results)
}

/// Search contacts via FTS5 prefix matching, falling back to LIKE if
/// the FTS5 table is unavailable.
fn search_contacts_fts_or_like(
    conn: &ReadConn<'_>,
    raw_query: &str,
    like_pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let fts_query = build_fts_query(raw_query);
    if !fts_query.is_empty() {
        let fts_sql = format!(
            "SELECT c.email, c.display_name, c.source, {CONTACT_ORIGIN_COLUMNS}
             FROM contacts c
             INNER JOIN contacts_fts ON contacts_fts.rowid = c.rowid
             WHERE contacts_fts MATCH ?1
             ORDER BY c.last_contacted_at DESC NULLS LAST,
                      c.display_name ASC
             LIMIT ?2"
        );
        match conn.prepare(&fts_sql) {
            Ok(mut stmt) => {
                let rows = stmt
                    .query_map(params![&fts_query, limit], |row| {
                        Ok(ContactSearchResult {
                            email: row.get("email")?,
                            display_name: row.get("display_name")?,
                            kind: ContactSearchKind::Contact,
                            source: row.get("source")?,
                            origins: ContactOrigins::from_row(row)?,
                        })
                    })
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    let result = row.map_err(|e| e.to_string())?;
                    let key = result.email.to_lowercase();
                    if seen_emails.insert(key) {
                        results.push(result);
                    }
                }
                return Ok(());
            }
            Err(_) => { /* FTS5 table missing - fall through to LIKE */ }
        }
    }

    // LIKE fallback
    search_contacts_table(conn, like_pattern, limit, seen_emails, results)
}

fn search_gal_cache(
    conn: &ReadConn<'_>,
    pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let sql = "SELECT g.email, g.display_name, a.provider
               FROM gal_cache g
               JOIN accounts a ON a.id = g.account_id
               WHERE g.email LIKE ?1 ESCAPE '\\' OR g.display_name LIKE ?1 ESCAPE '\\'
               ORDER BY g.display_name ASC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            Ok(ContactSearchResult {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                kind: ContactSearchKind::Contact,
                source: Some("gal_cache".to_string()),
                origins: ContactOrigins::of_directory(&row.get::<_, String>("provider")?),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let result = row.map_err(|e| e.to_string())?;
        let key = result.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(result);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal search helpers
// ---------------------------------------------------------------------------

fn search_contacts_table(
    conn: &ReadConn<'_>,
    pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let sql = format!(
        "SELECT c.email, c.display_name, c.source, {CONTACT_ORIGIN_COLUMNS}
         FROM contacts c
         WHERE c.email LIKE ?1 ESCAPE '\\' OR c.display_name LIKE ?1 ESCAPE '\\'
         ORDER BY c.last_contacted_at DESC NULLS LAST, c.display_name ASC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            Ok(ContactSearchResult {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                kind: ContactSearchKind::Contact,
                source: row.get("source")?,
                origins: ContactOrigins::from_row(row)?,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let result = row.map_err(|e| e.to_string())?;
        let key = result.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(result);
        }
    }
    Ok(())
}

/// Search seen_addresses via FTS5 prefix matching, falling back to LIKE if
/// the FTS5 table is unavailable (e.g. old DB without migration 79).
fn search_seen_addresses_fts_or_like(
    conn: &ReadConn<'_>,
    raw_query: &str,
    like_pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let fts_query = build_fts_query(raw_query);
    if !fts_query.is_empty() {
        let fts_sql = "SELECT s.email, s.display_name
                       FROM seen_addresses s
                       INNER JOIN seen_addresses_fts ON seen_addresses_fts.rowid = s.rowid
                       WHERE seen_addresses_fts MATCH ?1
                       ORDER BY s.last_seen_at DESC
                       LIMIT ?2";
        match conn.prepare(fts_sql) {
            Ok(mut stmt) => {
                let rows = stmt
                    .query_map(params![&fts_query, limit], |row| {
                        Ok(ContactSearchResult {
                            email: row.get("email")?,
                            display_name: row.get("display_name")?,
                            kind: ContactSearchKind::SeenAddress,
                            source: Some("seen".to_string()),
                            origins: ContactOrigins::default(),
                        })
                    })
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    let result = row.map_err(|e| e.to_string())?;
                    let key = result.email.to_lowercase();
                    if seen_emails.insert(key) {
                        results.push(result);
                    }
                }
                return Ok(());
            }
            Err(_) => { /* FTS5 table missing - fall through to LIKE */ }
        }
    }

    // LIKE fallback
    let sql = "SELECT email, display_name FROM seen_addresses
               WHERE email LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\'
               ORDER BY last_seen_at DESC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like_pattern, limit], |row| {
            Ok(ContactSearchResult {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                kind: ContactSearchKind::SeenAddress,
                source: Some("seen".to_string()),
                origins: ContactOrigins::default(),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let result = row.map_err(|e| e.to_string())?;
        let key = result.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(result);
        }
    }
    Ok(())
}

fn search_groups_table(
    conn: &ReadConn<'_>,
    pattern: &str,
    limit: i64,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let sql = "SELECT g.id, g.name,
                      (SELECT COUNT(*) FROM contact_group_members m
                       WHERE m.group_id = g.id) AS member_count
               FROM contact_groups g
               WHERE g.name LIKE ?1 ESCAPE '\\'
               ORDER BY g.name ASC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            let id: String = row.get("id")?;
            let name: String = row.get("name")?;
            let count: i64 = row.get("member_count")?;
            Ok(ContactSearchResult {
                email: String::new(),
                display_name: Some(name),
                kind: ContactSearchKind::Group {
                    group_id: id,
                    member_count: count,
                },
                source: None,
                origins: ContactOrigins::default(),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        results.push(row.map_err(|e| e.to_string())?);
    }
    Ok(())
}
