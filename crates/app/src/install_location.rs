//! First launch from the download: Courly installs itself in Applications.
//!
//! A disk image cannot install anything by itself, so the installer window
//! invites a double click on the app. When Courly then finds it is running
//! from a mounted disk image, from Downloads, or from the read-only copy
//! Gatekeeper translocates a quarantined app to, it copies itself into
//! /Applications (or ~/Applications when that is not writable) without
//! asking, relaunches from there and ejects the disk image. A newer copy
//! already installed is opened instead of being replaced. Any other
//! location, a development build included, is left alone.

#[cfg(target_os = "macos")]
pub use macos::install_from_download;

/// Other platforms install through their own packages.
#[cfg(not(target_os = "macos"))]
pub fn install_from_download() {}

/// Where a bundle was launched from, as far as moving it is concerned.
#[derive(Debug, PartialEq, Eq)]
enum Origin {
    /// Already where applications live, or somewhere chosen on purpose.
    Settled,
    /// Inside a mounted volume, typically the downloaded disk image.
    Volume(std::path::PathBuf),
    /// Inside the user's Downloads folder.
    Downloads,
}

/// Classify the original (untranslocated) path of the bundle.
fn origin_of(bundle: &std::path::Path, home: Option<&std::path::Path>) -> Origin {
    use std::path::{Component, Path};

    let in_dir = |dir: &Path| bundle.parent().is_some_and(|parent| parent == dir);
    if in_dir(Path::new("/Applications"))
        || home.is_some_and(|home| in_dir(&home.join("Applications")))
    {
        return Origin::Settled;
    }
    if let Ok(rest) = bundle.strip_prefix("/Volumes")
        && let Some(Component::Normal(name)) = rest.components().next()
    {
        return Origin::Volume(Path::new("/Volumes").join(name));
    }
    if home.is_some_and(|home| bundle.starts_with(home.join("Downloads"))) {
        return Origin::Downloads;
    }
    Origin::Settled
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use objc2::rc::Retained;
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    use super::{Origin, origin_of};

    // Exported by Security.framework since macOS 10.12 but without a
    // public header; Sparkle and LetsMove call it for the same purpose:
    // a quarantined app launched from a disk image or Downloads runs from
    // a randomized read-only path, and only this maps it back.
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecTranslocateIsTranslocatedURL(
            path: *const NSURL,
            is_translocated: *mut u8,
            error: *mut *mut c_void,
        ) -> u8;
        fn SecTranslocateCreateOriginalPathForURL(
            translocated: *const NSURL,
            error: *mut *mut c_void,
        ) -> *mut NSURL;
    }

    fn file_url(path: &Path) -> Retained<NSURL> {
        NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
    }

    /// The path the user actually launched, undoing App Translocation.
    fn original_path(running: &Path) -> PathBuf {
        let url = file_url(running);
        let mut translocated = 0u8;
        // SAFETY: `url` is a live NSURL, toll-free bridged to CFURLRef; the
        // out-pointers are valid for the call and errors are ignored (a
        // null error pointer is accepted by both functions).
        let is_translocated = unsafe {
            SecTranslocateIsTranslocatedURL(
                Retained::as_ptr(&url),
                &mut translocated,
                std::ptr::null_mut(),
            ) != 0
                && translocated != 0
        };
        if !is_translocated {
            return running.to_path_buf();
        }
        // SAFETY: as above. The function follows the Create rule, so the
        // returned URL is owned (+1) and `from_raw` takes that ownership.
        let original = unsafe {
            Retained::from_raw(SecTranslocateCreateOriginalPathForURL(
                Retained::as_ptr(&url),
                std::ptr::null_mut(),
            ))
        };
        original
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()))
            .unwrap_or_else(|| running.to_path_buf())
    }

    /// Runs at launch, before any window or the Service exists. Returns
    /// normally when Courly should start where it is, including when the
    /// copy fails (Courly then simply runs from the download); exits the
    /// process once the installed copy is set to relaunch.
    pub fn install_from_download() {
        let Some(running) = crate::updater::installed_bundle() else {
            return;
        };
        let source = original_path(&running);
        let home = dirs::home_dir();
        let origin = origin_of(&source, home.as_deref());
        if origin == Origin::Settled {
            return;
        }

        let installed = match install(&running, home.as_deref()) {
            Ok(installed) => installed,
            Err(error) => {
                log::warn!("install: could not install in Applications: {error}");
                return;
            }
        };
        if origin == Origin::Downloads {
            trash(&source);
        }
        let volume = match origin {
            Origin::Volume(volume) => Some(volume),
            Origin::Settled | Origin::Downloads => None,
        };
        match relaunch(&installed, volume.as_deref()) {
            Ok(()) => std::process::exit(0),
            Err(error) => log::warn!("install: relaunch failed: {error}"),
        }
    }

    /// `CFBundleShortVersionString` of a bundle on disk.
    fn bundle_version(bundle: &Path) -> Option<String> {
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
            .arg(bundle.join("Contents/Info.plist"))
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Copy the running bundle into the first writable Applications folder
    /// and return the installed path.
    fn install(running: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
        let name = running
            .file_name()
            .ok_or("the app bundle has no name")?
            .to_owned();
        let mut targets = vec![PathBuf::from("/Applications")];
        if let Some(home) = home {
            targets.push(home.join("Applications"));
        }
        let mut last_error = String::from("no Applications folder");
        for dir in targets {
            match install_into(running, &dir, &name) {
                Ok(installed) => return Ok(installed),
                Err(error) => last_error = format!("{}: {error}", dir.display()),
            }
        }
        Err(last_error)
    }

    /// Copy into a hidden staging bundle beside the destination, so a failed
    /// copy never leaves a half-written Courly.app behind, then swap it in.
    /// A previous copy goes to the Trash rather than being deleted.
    fn install_into(running: &Path, dir: &Path, name: &std::ffi::OsStr) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let destination = dir.join(name);
        // An older download opened by mistake must not downgrade the app:
        // the newer installed copy is opened as it is.
        if bundle_version(&destination).is_some_and(|installed| {
            crate::updater::is_newer(&installed, crate::updater::current_version())
        }) {
            return Ok(destination);
        }
        let staging = dir.join(format!(".{}.install", name.to_string_lossy()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
        }
        let copied = Command::new("/usr/bin/ditto")
            .arg(running)
            .arg(&staging)
            .stderr(Stdio::null())
            .status()
            .map_err(|error| error.to_string())?;
        if !copied.success() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("ditto exited with {copied}"));
        }
        // The user already approved this download when Gatekeeper let it
        // open; without this the installed copy would ask a second time.
        let _ = Command::new("/usr/bin/xattr")
            .args(["-d", "-r", "com.apple.quarantine"])
            .arg(&staging)
            .stderr(Stdio::null())
            .status();
        if destination.exists() && !trash(&destination) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("{} could not be replaced", destination.display()));
        }
        std::fs::rename(&staging, &destination).map_err(|error| {
            let _ = std::fs::remove_dir_all(&staging);
            error.to_string()
        })?;
        Ok(destination)
    }

    /// Move a path to the Trash, so nothing is lost for good.
    fn trash(path: &Path) -> bool {
        let url = file_url(path);
        let manager = NSFileManager::defaultManager();
        match manager.trashItemAtURL_resultingItemURL_error(&url, None) {
            Ok(()) => true,
            Err(error) => {
                log::warn!(
                    "install: could not move {} to the Trash: {error:?}",
                    path.display()
                );
                false
            }
        }
    }

    /// Once this process has exited, open the installed copy and eject the
    /// disk image it came from.
    fn relaunch(installed: &Path, volume: Option<&Path>) -> Result<(), String> {
        use std::os::unix::process::CommandExt as _;

        Command::new("/bin/sh")
            .arg("-c")
            .arg(concat!(
                "while /bin/kill -0 \"$0\" 2>/dev/null; do /bin/sleep 0.2; done; ",
                "/usr/bin/open \"$1\"; ",
                // macOS keeps the image busy for a moment after the app on
                // it exits, so the eject is retried for a few seconds.
                "if [ -n \"$2\" ]; then for try in 1 2 3 4 5 6 7 8 9 10; do ",
                "/usr/sbin/diskutil eject \"$2\" >/dev/null 2>&1 && break; /bin/sleep 1; ",
                "done; fi",
            ))
            .arg(std::process::id().to_string())
            .arg(installed)
            .arg(volume.map(Path::as_os_str).unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Origin, origin_of};

    const HOME: &str = "/Users/someone";

    fn origin(path: &str) -> Origin {
        origin_of(Path::new(path), Some(Path::new(HOME)))
    }

    #[test]
    fn an_app_already_in_applications_stays() {
        assert_eq!(origin("/Applications/Courly.app"), Origin::Settled);
        assert_eq!(
            origin("/Users/someone/Applications/Courly.app"),
            Origin::Settled
        );
    }

    #[test]
    fn an_app_on_the_disk_image_names_its_volume() {
        assert_eq!(
            origin("/Volumes/Courly/Courly.app"),
            Origin::Volume(PathBuf::from("/Volumes/Courly"))
        );
        assert_eq!(
            origin("/Volumes/Courly 1/Courly.app"),
            Origin::Volume(PathBuf::from("/Volumes/Courly 1"))
        );
    }

    #[test]
    fn an_app_in_downloads_is_offered_the_move() {
        assert_eq!(
            origin("/Users/someone/Downloads/Courly.app"),
            Origin::Downloads
        );
        assert_eq!(
            origin("/Users/someone/Downloads/Courly 2/Courly.app"),
            Origin::Downloads
        );
    }

    #[test]
    fn a_build_anywhere_else_is_left_alone() {
        assert_eq!(
            origin("/Users/someone/Code/courly/target/build-local/Courly.app"),
            Origin::Settled
        );
        assert_eq!(
            origin("/Applications/Utilities/Courly.app"),
            Origin::Settled
        );
        assert_eq!(origin("/Users/someone/Desktop/Courly.app"), Origin::Settled);
    }
}
