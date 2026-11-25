//! Entry points Dart calls through flutter_rust_bridge.
//!
//! Everything here runs inside the app process: on iOS there is no child
//! Service process, so the shared crates are linked into the app directly.

use std::path::Path;

/// What opening the local Courly database on the device produced.
pub struct DatabaseReport {
    /// Highest schema version present after migrations ran.
    pub schema_version: u32,
    /// Migrations applied during this call (0 when the database was current).
    pub migrations_applied: u32,
    /// Number of tables in the opened database.
    pub table_count: u32,
}

/// Open (or create) the Courly database inside `data_dir` and bring its
/// schema up to date with the same migrations the desktop Service runs.
pub fn open_courly_database(data_dir: String) -> Result<DatabaseReport, String> {
    let data_dir = Path::new(&data_dir);
    if !data_dir.is_dir() {
        return Err(format!(
            "data directory does not exist: {}",
            data_dir.display()
        ));
    }
    let conn = rusqlite::Connection::open(data_dir.join(db::db::DB_FILE_NAME))
        .map_err(|e| format!("open database: {e}"))?;
    let migrations_applied = db::db::migrations::run_all(&conn)?;
    let schema_version = db::db::migrations::current_schema_version(&conn)?;
    let table_count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("count tables: {e}"))?;
    Ok(DatabaseReport {
        schema_version,
        migrations_applied,
        table_count,
    })
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

#[cfg(test)]
mod tests {
    use super::open_courly_database;

    #[test]
    fn fresh_directory_gets_the_full_schema_then_stays_current() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().into_owned();

        let first = open_courly_database(path.clone()).expect("first open");
        assert!(first.migrations_applied > 0);
        assert!(first.schema_version >= 100);
        assert!(first.table_count > 10);

        let second = open_courly_database(path).expect("second open");
        assert_eq!(second.migrations_applied, 0);
        assert_eq!(second.schema_version, first.schema_version);
    }

    #[test]
    fn missing_directory_is_an_explicit_error() {
        let result = open_courly_database("/nonexistent/courly-mobile-test".to_owned());
        assert!(result.is_err());
    }
}
