//! Location of the data directory under the platform data dir.

#[cfg(any(test, not(feature = "dev-seed")))]
use std::path::{Path, PathBuf};

/// Named after the bundle identifier, so renaming the app never has to move
/// user data again.
#[cfg(any(test, not(feature = "dev-seed")))]
pub(crate) const DATA_DIR_NAME: &str = "com.soflution.mail";
#[cfg(feature = "dev-seed")]
pub(crate) const DEV_DATA_DIR_NAME: &str = "com.soflution.mail.dev";
#[cfg(any(test, not(feature = "dev-seed")))]
const LEGACY_DATA_DIR_NAME: &str = "org.folknor.ratatoskr";

/// The data directory under `base`, adopting the directory used before the
/// rename to Courly. The move is a single rename on the same volume, so an
/// interrupted launch leaves either the old or the new directory, never a
/// mix. When both exist the new one wins and the old one is left untouched.
/// When the move fails the old directory keeps being used, so the user's
/// accounts never look gone.
#[cfg(any(test, not(feature = "dev-seed")))]
pub(crate) fn resolve(base: &Path) -> PathBuf {
    let current = base.join(DATA_DIR_NAME);
    let legacy = base.join(LEGACY_DATA_DIR_NAME);
    if current.exists() {
        if legacy.exists() {
            log::warn!(
                "both {} and {} exist; using the first, the second is left as is",
                current.display(),
                legacy.display()
            );
        }
        return current;
    }
    if !legacy.exists() {
        return current;
    }
    match std::fs::rename(&legacy, &current) {
        Ok(()) => {
            log::info!(
                "moved the data directory from {} to {}",
                legacy.display(),
                current.display()
            );
            current
        }
        Err(_) if current.exists() => current,
        Err(error) => {
            log::error!(
                "could not move the data directory from {} to {}: {error}; using it in place",
                legacy.display(),
                current.display()
            );
            legacy
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DATA_DIR_NAME, LEGACY_DATA_DIR_NAME, resolve};

    #[test]
    fn the_legacy_directory_is_moved_with_its_contents() {
        let base = tempfile::tempdir().expect("tempdir");
        let legacy = base.path().join(LEGACY_DATA_DIR_NAME);
        std::fs::create_dir(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("courly.db"), b"accounts").expect("db");

        let resolved = resolve(base.path());

        assert_eq!(resolved, base.path().join(DATA_DIR_NAME));
        assert_eq!(
            std::fs::read(resolved.join("courly.db")).expect("moved db"),
            b"accounts"
        );
        assert!(!legacy.exists());
    }

    #[test]
    fn a_fresh_install_gets_the_new_directory_without_creating_it() {
        let base = tempfile::tempdir().expect("tempdir");

        let resolved = resolve(base.path());

        assert_eq!(resolved, base.path().join(DATA_DIR_NAME));
        assert!(!resolved.exists());
    }

    #[test]
    fn an_existing_new_directory_wins_and_the_legacy_one_is_untouched() {
        let base = tempfile::tempdir().expect("tempdir");
        let current = base.path().join(DATA_DIR_NAME);
        let legacy = base.path().join(LEGACY_DATA_DIR_NAME);
        std::fs::create_dir(&current).expect("current dir");
        std::fs::create_dir(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("courly.db"), b"old").expect("db");

        assert_eq!(resolve(base.path()), current);
        assert_eq!(
            std::fs::read(legacy.join("courly.db")).expect("legacy db"),
            b"old"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_move_keeps_using_the_legacy_directory() {
        use std::os::unix::fs::PermissionsExt;

        let base = tempfile::tempdir().expect("tempdir");
        let legacy = base.path().join(LEGACY_DATA_DIR_NAME);
        std::fs::create_dir(&legacy).expect("legacy dir");
        std::fs::set_permissions(base.path(), std::fs::Permissions::from_mode(0o555))
            .expect("read-only base");

        let resolved = resolve(base.path());

        std::fs::set_permissions(base.path(), std::fs::Permissions::from_mode(0o755))
            .expect("restore base");
        assert_eq!(resolved, legacy);
        assert!(legacy.exists());
    }
}
