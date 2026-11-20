//! Write helpers for `label_groups`. Reads live in
//! `rtsk::db::queries_extra::navigation`; writes belong here so the
//! Service is the only crate that can mutate them - the `app-no-db`
//! dependency rule enforces that the app cannot import this module.

use crate::db::{WriteConn, params};

/// A provider label that belongs to a label group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelGroupMember {
    pub account_id: String,
    pub label_id: String,
    /// The label's name on its provider.
    pub name: String,
}

/// A label group as stored: its name and colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLabelGroup {
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
}

/// The stored row of one label group, `None` when it does not exist.
pub fn get_label_group_sync(
    conn: &WriteConn<'_>,
    group_id: i64,
) -> Result<Option<StoredLabelGroup>, String> {
    let mut stmt = conn
        .prepare("SELECT name, color_bg, color_fg FROM label_groups WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![group_id], |row| {
            Ok(StoredLabelGroup {
                name: row.get(0)?,
                color_bg: row.get(1)?,
                color_fg: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.next().transpose().map_err(|e| e.to_string())
}

/// The id of the label group with this name, whatever the case.
pub fn find_label_group_by_name_sync(
    conn: &WriteConn<'_>,
    name: &str,
) -> Result<Option<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM label_groups WHERE name = ?1 COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![name], |row| row.get::<_, i64>(0))
        .map_err(|e| e.to_string())?;
    rows.next().transpose().map_err(|e| e.to_string())
}

/// The provider labels of one label group.
pub fn label_group_members_sync(
    conn: &WriteConn<'_>,
    group_id: i64,
) -> Result<Vec<LabelGroupMember>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.account_id, m.label_id, l.name \
             FROM label_group_members m \
             JOIN labels l ON l.account_id = m.account_id AND l.id = m.label_id \
             WHERE m.group_id = ?1 \
             ORDER BY m.account_id, m.label_id",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map(params![group_id], |row| {
        Ok(LabelGroupMember {
            account_id: row.get(0)?,
            label_id: row.get(1)?,
            name: row.get(2)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Create a label group, or give the one that already has this name its
/// colours. Returns its id. A new group goes last.
pub fn create_label_group_sync(
    conn: &WriteConn<'_>,
    name: &str,
    color_bg: &str,
    color_fg: &str,
) -> Result<i64, String> {
    if let Some(existing) = find_label_group_by_name_sync(conn, name)? {
        conn.execute(
            "UPDATE label_groups SET color_bg = ?1, color_fg = ?2 WHERE id = ?3",
            params![color_bg, color_fg, existing],
        )
        .map_err(|e| format!("label_group.save recolour: {e}"))?;
        return Ok(existing);
    }
    conn.execute(
        "INSERT INTO label_groups (name, color_bg, color_fg, sort_order) \
         VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM label_groups))",
        params![name, color_bg, color_fg],
    )
    .map_err(|e| format!("label_group.save insert: {e}"))?;
    Ok(conn.last_insert_rowid())
}

/// Why a label group could not be renamed or recoloured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelGroupUpdateError {
    /// No group has this id.
    Missing,
    /// Another group already has the new name.
    NameTaken,
    Database(String),
}

/// Rename and recolour one label group.
pub fn update_label_group_sync(
    conn: &WriteConn<'_>,
    group_id: i64,
    name: &str,
    color_bg: &str,
    color_fg: &str,
) -> Result<(), LabelGroupUpdateError> {
    let taken = find_label_group_by_name_sync(conn, name)
        .map_err(LabelGroupUpdateError::Database)?
        .is_some_and(|other| other != group_id);
    if taken {
        return Err(LabelGroupUpdateError::NameTaken);
    }
    let changed = conn
        .execute(
            "UPDATE label_groups SET name = ?1, color_bg = ?2, color_fg = ?3 WHERE id = ?4",
            params![name, color_bg, color_fg, group_id],
        )
        .map_err(|e| LabelGroupUpdateError::Database(format!("label_group.save update: {e}")))?;
    if changed == 0 {
        return Err(LabelGroupUpdateError::Missing);
    }
    Ok(())
}

/// Make `members` the provider labels of one label group. A label listed
/// here that was in another group leaves it: a label is in one group only.
pub fn set_label_group_members_sync(
    conn: &WriteConn<'_>,
    group_id: i64,
    members: &[(String, String)],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("label_group.save members begin tx: {e}"))?;
    tx.execute(
        "DELETE FROM label_group_members WHERE group_id = ?1",
        params![group_id],
    )
    .map_err(|e| format!("label_group.save clear members: {e}"))?;
    for (account_id, label_id) in members {
        tx.execute(
            "INSERT INTO label_group_members (group_id, account_id, label_id) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(account_id, label_id) DO UPDATE SET group_id = excluded.group_id",
            params![group_id, account_id, label_id],
        )
        .map_err(|e| format!("label_group.save add member: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("label_group.save members commit: {e}"))
}

/// Delete one label group and its membership rows. The provider labels
/// themselves are not touched.
pub fn delete_label_group_sync(conn: &WriteConn<'_>, group_id: i64) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("label_group.delete begin tx: {e}"))?;
    tx.execute(
        "DELETE FROM label_group_members WHERE group_id = ?1",
        params![group_id],
    )
    .map_err(|e| format!("label_group.delete members: {e}"))?;
    tx.execute("DELETE FROM label_groups WHERE id = ?1", params![group_id])
        .map_err(|e| format!("label_group.delete group: {e}"))?;
    tx.commit()
        .map_err(|e| format!("label_group.delete commit: {e}"))
}

/// Persist a new ordering for label groups. Each `(group_id, sort_order)`
/// pair is written in a single transaction. Drives drag-to-reorder in
/// Settings > Labels.
pub fn update_label_group_sort_order_sync(
    conn: &WriteConn<'_>,
    updates: &[(i64, i64)],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("label_group.reorder begin tx: {e}"))?;
    {
        let mut stmt = tx
            .prepare("UPDATE label_groups SET sort_order = ?1 WHERE id = ?2")
            .map_err(|e| e.to_string())?;
        for (id, order) in updates {
            stmt.execute(params![order, id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit()
        .map_err(|e| format!("label_group.reorder commit: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn labels_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE labels (account_id TEXT, id TEXT, name TEXT,
                 PRIMARY KEY (account_id, id));
             CREATE TABLE label_groups (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
                 color_bg TEXT NOT NULL, color_fg TEXT NOT NULL,
                 sort_order INTEGER NOT NULL DEFAULT 0,
                 UNIQUE (name COLLATE NOCASE));
             CREATE TABLE label_group_members (
                 group_id INTEGER NOT NULL, account_id TEXT NOT NULL, label_id TEXT NOT NULL,
                 PRIMARY KEY (group_id, account_id, label_id),
                 UNIQUE (account_id, label_id));
             INSERT INTO labels VALUES
                 ('a', 'cat:Urgent', 'Urgent'), ('b', 'cat:Urgent', 'Urgent'),
                 ('a', 'cat:Later', 'Later');",
        )
        .expect("schema");
        conn
    }

    fn write(conn: &Connection) -> WriteConn<'_> {
        WriteConn::from_raw(conn)
    }

    #[test]
    fn a_created_group_goes_last_and_an_existing_name_is_recoloured() {
        let conn = labels_db();
        let first =
            create_label_group_sync(&write(&conn), "Urgent", "#111111", "#ffffff").expect("create");
        let second =
            create_label_group_sync(&write(&conn), "Later", "#222222", "#ffffff").expect("create");
        assert_ne!(first, second);
        let orders: Vec<i64> = [first, second]
            .iter()
            .map(|id| {
                conn.query_row(
                    "SELECT sort_order FROM label_groups WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .expect("sort order")
            })
            .collect();
        assert!(orders[0] < orders[1]);

        // The same name in another case is the same group, with new colours.
        let again = create_label_group_sync(&write(&conn), "URGENT", "#333333", "#000000")
            .expect("create again");
        assert_eq!(again, first);
        assert_eq!(
            get_label_group_sync(&write(&conn), first).expect("read"),
            Some(StoredLabelGroup {
                name: "Urgent".to_owned(),
                color_bg: "#333333".to_owned(),
                color_fg: "#000000".to_owned(),
            })
        );
    }

    #[test]
    fn a_group_is_renamed_unless_the_name_is_taken_or_the_group_is_gone() {
        let conn = labels_db();
        let urgent =
            create_label_group_sync(&write(&conn), "Urgent", "#111111", "#ffffff").expect("create");
        create_label_group_sync(&write(&conn), "Later", "#222222", "#ffffff").expect("create");

        assert_eq!(
            update_label_group_sync(&write(&conn), urgent, "later", "#111111", "#ffffff"),
            Err(LabelGroupUpdateError::NameTaken)
        );
        assert_eq!(
            update_label_group_sync(&write(&conn), 999, "Other", "#111111", "#ffffff"),
            Err(LabelGroupUpdateError::Missing)
        );
        // Changing the case of its own name is a rename like any other.
        update_label_group_sync(&write(&conn), urgent, "URGENT", "#444444", "#ffffff")
            .expect("rename");
        let stored = get_label_group_sync(&write(&conn), urgent)
            .expect("read")
            .expect("row");
        assert_eq!(stored.name, "URGENT");
        assert_eq!(stored.color_bg, "#444444");
    }

    #[test]
    fn members_are_replaced_and_a_label_leaves_its_former_group() {
        let conn = labels_db();
        let urgent =
            create_label_group_sync(&write(&conn), "Urgent", "#111111", "#ffffff").expect("create");
        let later =
            create_label_group_sync(&write(&conn), "Later", "#222222", "#ffffff").expect("create");
        let pair = |account: &str, label: &str| (account.to_owned(), label.to_owned());

        set_label_group_members_sync(
            &write(&conn),
            urgent,
            &[pair("a", "cat:Urgent"), pair("b", "cat:Urgent")],
        )
        .expect("members");
        set_label_group_members_sync(&write(&conn), later, &[pair("a", "cat:Later")])
            .expect("members");

        // One label moves to the other group, one is dropped.
        set_label_group_members_sync(
            &write(&conn),
            later,
            &[pair("a", "cat:Later"), pair("a", "cat:Urgent")],
        )
        .expect("members");
        let names = |group| -> Vec<(String, String)> {
            label_group_members_sync(&write(&conn), group)
                .expect("read members")
                .into_iter()
                .map(|member| (member.account_id, member.label_id))
                .collect()
        };
        assert_eq!(names(urgent), [pair("b", "cat:Urgent")]);
        assert_eq!(
            names(later),
            [pair("a", "cat:Later"), pair("a", "cat:Urgent")]
        );
    }

    #[test]
    fn deleting_a_group_takes_its_membership_and_leaves_the_labels() {
        let conn = labels_db();
        let urgent =
            create_label_group_sync(&write(&conn), "Urgent", "#111111", "#ffffff").expect("create");
        set_label_group_members_sync(
            &write(&conn),
            urgent,
            &[("a".to_owned(), "cat:Urgent".to_owned())],
        )
        .expect("members");

        delete_label_group_sync(&write(&conn), urgent).expect("delete");
        assert_eq!(get_label_group_sync(&write(&conn), urgent), Ok(None));
        let members: i64 = conn
            .query_row("SELECT COUNT(*) FROM label_group_members", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(members, 0);
        let labels: i64 = conn
            .query_row("SELECT COUNT(*) FROM labels", [], |row| row.get(0))
            .expect("count");
        assert_eq!(labels, 3);
    }
}
