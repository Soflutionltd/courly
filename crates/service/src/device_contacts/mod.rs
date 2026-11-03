//! The Mac's address book (the Contacts app: iCloud and every account the
//! user added there), imported read-only into `contacts` so search and
//! compose suggest those people. macOS asks the user once for access.

mod address_book;

use std::collections::HashSet;

use db::db::queries_extra::{
    DeviceContactRow, prune_device_contacts_sync, upsert_device_contact_sync,
};
use service_state::WriteDbState;

/// A person in the Mac's address book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceContact {
    pub identifier: String,
    pub given_name: String,
    pub family_name: String,
    pub organization: String,
    pub emails: Vec<String>,
}

/// One row per address: a person with a work and a home address can be
/// found under either.
fn rows_for(contacts: &[DeviceContact]) -> Vec<DeviceContactRow> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for contact in contacts {
        let full_name = format!(
            "{} {}",
            contact.given_name.trim(),
            contact.family_name.trim()
        );
        let full_name = full_name.trim();
        let organization = contact.organization.trim();
        let display_name = if full_name.is_empty() {
            (!organization.is_empty()).then(|| organization.to_owned())
        } else {
            Some(full_name.to_owned())
        };
        let company = (!organization.is_empty()).then(|| organization.to_owned());
        for email in &contact.emails {
            let email = email.trim().to_lowercase();
            if email.is_empty() || !seen.insert(email.clone()) {
                continue;
            }
            rows.push(DeviceContactRow {
                email,
                display_name: display_name.clone(),
                company: company.clone(),
                server_id: contact.identifier.clone(),
            });
        }
    }
    rows
}

/// Writes the address book as it is now: new and changed people are
/// stored, people deleted from it are removed.
async fn import(write_db: &WriteDbState, contacts: Vec<DeviceContact>) -> Result<usize, String> {
    let rows = rows_for(&contacts);
    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|error| format!("begin device contact import: {error}"))?;
            for row in &rows {
                upsert_device_contact_sync(&tx, row)?;
            }
            let kept: HashSet<String> = rows.iter().map(|row| row.email.clone()).collect();
            prune_device_contacts_sync(&tx, &kept)?;
            tx.commit()
                .map_err(|error| format!("commit device contact import: {error}"))?;
            Ok(rows.len())
        })
        .await
}

pub(crate) use run::spawn_post_ready_device_contacts;

mod run {
    use std::sync::Arc;
    use std::time::Duration;

    use super::address_book::{self, ReadError};
    use crate::boot;

    /// The Contacts framework offers change notifications only to a running
    /// app's main loop, so the service rereads the address book on a timer.
    const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

    pub(crate) fn spawn_post_ready_device_contacts(
        boot_state: Arc<boot::BootSharedState>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if boot_state.wait_for_ready().await.is_err() {
                log::debug!("device contacts: boot failed, skipping");
                return;
            }
            let write_db = match boot_state.write_db_state() {
                Ok(write_db) => write_db,
                Err(error) => {
                    log::error!("device contacts: db missing after boot.ready: {error}");
                    return;
                }
            };
            let cancellation = boot_state.shutdown_token().child_token();
            loop {
                match tokio::task::spawn_blocking(address_book::read).await {
                    Ok(Ok(contacts)) => match super::import(&write_db, contacts).await {
                        Ok(count) => log::info!("device contacts: {count} address(es) imported"),
                        Err(error) => log::warn!("device contacts: import failed: {error}"),
                    },
                    Ok(Err(ReadError::Denied)) => {
                        log::warn!(
                            "device contacts: access to Contacts is not granted; \
                             allow Courly in System Settings > Privacy & Security > Contacts"
                        );
                        return;
                    }
                    Ok(Err(ReadError::Failed(error))) => {
                        log::warn!("device contacts: reading the address book failed: {error}");
                    }
                    Err(error) => log::warn!("device contacts: reader task failed: {error}"),
                }
                tokio::select! {
                    () = cancellation.cancelled() => return,
                    () = tokio::time::sleep(REFRESH_INTERVAL) => {}
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::db::WriteConn;
    use rusqlite::{OptionalExtension, params};
    use tempfile::TempDir;

    fn fresh_db() -> (WriteDbState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let pool = db::db::open_writer_pool(tmp.path()).expect("writer pool");
        (WriteDbState::from_pool(pool), tmp)
    }

    fn person(identifier: &str, given: &str, family: &str, emails: &[&str]) -> DeviceContact {
        DeviceContact {
            identifier: identifier.to_owned(),
            given_name: given.to_owned(),
            family_name: family.to_owned(),
            organization: String::new(),
            emails: emails.iter().map(|email| (*email).to_owned()).collect(),
        }
    }

    fn contact(conn: &WriteConn<'_>, email: &str) -> Option<(Option<String>, String)> {
        conn.query_row(
            "SELECT display_name, source FROM contacts WHERE email = ?1",
            params![email],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .expect("read contact")
    }

    #[test]
    fn each_address_is_one_row_named_after_the_person_or_the_company() {
        let mut company = person("c2", "", "", &["hello@gallery.example"]);
        company.organization = "NOA Contemporary".to_owned();
        let rows = rows_for(&[
            person(
                "c1",
                "Florian",
                "Koenig",
                &[" Florian@Gallery.example ", "f@home.example"],
            ),
            company,
        ]);
        let names: Vec<(&str, Option<&str>)> = rows
            .iter()
            .map(|row| (row.email.as_str(), row.display_name.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("florian@gallery.example", Some("Florian Koenig")),
                ("f@home.example", Some("Florian Koenig")),
                ("hello@gallery.example", Some("NOA Contemporary")),
            ]
        );
    }

    #[tokio::test]
    async fn import_adds_updates_and_removes_without_touching_synced_contacts() {
        let (write_db, _tmp) = fresh_db();
        write_db
            .with_write(|conn| {
                conn.execute(
                    "INSERT INTO contacts (id, email, display_name, source) \
                     VALUES ('g', 'shared@example.test', 'From Outlook', 'graph')",
                    [],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })
            .await
            .expect("seed");

        import(
            &write_db,
            vec![
                person("c1", "Ada", "Lovelace", &["ada@example.test"]),
                person("c2", "Grace", "Hopper", &["grace@example.test"]),
                person("c3", "Mac", "Name", &["shared@example.test"]),
            ],
        )
        .await
        .expect("first import");
        import(
            &write_db,
            vec![person("c1", "Ada", "King", &["ada@example.test"])],
        )
        .await
        .expect("second import");

        write_db
            .with_write(|conn| {
                assert_eq!(
                    contact(conn, "ada@example.test"),
                    Some((Some("Ada King".to_owned()), "macos".to_owned()))
                );
                assert_eq!(contact(conn, "grace@example.test"), None);
                assert_eq!(
                    contact(conn, "shared@example.test"),
                    Some((Some("From Outlook".to_owned()), "graph".to_owned()))
                );
                Ok(())
            })
            .await
            .expect("check");
    }
}
