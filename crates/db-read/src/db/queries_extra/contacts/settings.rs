use crate::db::ReadConn;
use crate::db::queries_extra::contact_search::{CONTACT_ORIGIN_COLUMNS, ContactOrigins};

/// A contact entry with extended fields for the settings UI.
#[derive(Debug, Clone)]
pub struct ContactSettingsEntry {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub account_color: Option<String>,
    pub groups: Vec<String>,
    pub source: Option<String>,
    pub server_id: Option<String>,
    pub origins: ContactOrigins,
}

/// The address book the settings list is narrowed to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContactOriginFilter {
    #[default]
    All,
    Apple,
    Microsoft,
    Google,
    Courly,
}

impl ContactOriginFilter {
    fn clause(self) -> &'static str {
        match self {
            Self::All => "1",
            Self::Apple => {
                "(c.source = 'macos'
                  OR EXISTS (SELECT 1 FROM device_contact_claims d WHERE d.email = c.email))"
            }
            Self::Microsoft => {
                "(c.source = 'graph'
                  OR EXISTS (SELECT 1 FROM contact_claims cc
                             WHERE cc.email = c.email AND cc.source = 'graph'))"
            }
            Self::Google => {
                "(c.source = 'google'
                  OR EXISTS (SELECT 1 FROM contact_claims cc
                             WHERE cc.email = c.email AND cc.source = 'google'))"
            }
            Self::Courly => "c.source = 'user'",
        }
    }
}

/// Load contacts for the settings management list (synchronous).
pub fn load_contacts_for_settings_sync(
    conn: &ReadConn<'_>,
    filter: &str,
    origin: ContactOriginFilter,
) -> Result<Vec<ContactSettingsEntry>, String> {
    let trimmed = filter.trim();
    let escaped = trimmed.replace('%', r"\%").replace('_', r"\_");
    let pattern = format!("%{escaped}%");

    let text_clause = if trimmed.is_empty() {
        "1"
    } else {
        "(c.email LIKE ?1 ESCAPE '\\'
          OR c.display_name LIKE ?1 ESCAPE '\\'
          OR c.company LIKE ?1 ESCAPE '\\')"
    };
    let sql = format!(
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id, c.source, c.server_id,
                a.account_color,
                GROUP_CONCAT(g.name, '||') AS group_names,
                {CONTACT_ORIGIN_COLUMNS}
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         LEFT JOIN contact_group_members m
           ON m.member_type = 'email' AND m.member_value = c.email
         LEFT JOIN contact_groups g ON g.id = m.group_id
         WHERE c.source != 'seen'
           AND {text_clause}
           AND {origin_clause}
         GROUP BY c.id
         ORDER BY c.last_contacted_at DESC NULLS LAST,
                  c.display_name ASC
         LIMIT 200",
        origin_clause = origin.clause(),
    );

    let db_params: &[&dyn rusqlite::types::ToSql] =
        if trimmed.is_empty() { &[] } else { &[&pattern] };

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(db_params, |row| {
            let group_names: Option<String> = row.get("group_names")?;
            let groups = group_names
                .map(|s| s.split("||").map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(ContactSettingsEntry {
                id: row.get("id")?,
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                email2: row.get("email2")?,
                phone: row.get("phone")?,
                company: row.get("company")?,
                notes: row.get("notes")?,
                account_id: row.get("account_id")?,
                account_color: row.get("account_color")?,
                groups,
                source: row.get("source")?,
                server_id: row.get("server_id")?,
                origins: ContactOrigins::from_row(row)?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
