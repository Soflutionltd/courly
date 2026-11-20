//! In-app updates for the installed macOS bundle.
//!
//! Same contract as the Tauri updater: a `latest.json` manifest published
//! on GitHub Releases, a `.app.tar.gz` payload, and a minisign signature
//! (Tauri key format, signed by `scripts/release-macos.sh`). The running bundle
//! is swapped in place and relaunched, so the user keeps the same app.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rust_i18n::t;

pub const REPOSITORY: &str = "Soflutionltd/courly";
const MANIFEST_URL: &str =
    "https://github.com/Soflutionltd/courly/releases/latest/download/latest.json";
/// Public half of `~/.tauri/courly_updater.key` (base64 of the minisign
/// public key file, as written by `tauri signer generate`).
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDA4REQxQUQ2MzUxNTk1RjAKUldUd2xSVTExaHJkQ0t1Z1pUQUJmNlpiWVk5WmdsK0NoNTlmN2o4ck4vQkZ3UXIvMDRNbTB0ZGMK";
const PLATFORM: &str = "darwin-aarch64";
/// The archive must come from this repository's own releases. The
/// signature already guards integrity; this keeps a tampered manifest from
/// sending every client to a third-party host.
const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/Soflutionltd/courly/releases/download/";

fn is_release_archive_url(url: &str) -> bool {
    url.strip_prefix(RELEASE_DOWNLOAD_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(".."))
}
const MAX_ARCHIVE_BYTES: u64 = 500 * 1024 * 1024;

pub const BOOT_CHECK_DELAY: Duration = Duration::from_secs(4);
pub const CHECK_INTERVAL: Duration = Duration::from_secs(180);
/// Window-focus checks are throttled to one per minute.
pub const FOCUS_CHECK_THROTTLE: Duration = Duration::from_secs(60);
pub const RETRY_DELAY: Duration = Duration::from_secs(15);
pub const AUTO_RETRIES: u32 = 2;

static HTTP: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("Courly/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| log::warn!("updater: HTTP client unavailable: {error}"))
        .ok()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    url: String,
    signature: String,
}

#[derive(serde::Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    platforms: HashMap<String, PlatformEntry>,
}

#[derive(serde::Deserialize)]
struct PlatformEntry {
    signature: String,
    url: String,
}

#[derive(Debug, Clone)]
pub enum InstallEvent {
    Progress { downloaded: u64, total: Option<u64> },
    Installing,
    Done(Result<(), String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Available(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        downloaded: u64,
        total: Option<u64>,
    },
    Installing(UpdateInfo),
    Failed {
        info: UpdateInfo,
        error: String,
    },
    Restarting(UpdateInfo),
    /// The new bundle is on disk but the automatic relaunch could not be
    /// scheduled; the user quits and reopens.
    Installed(UpdateInfo),
}

#[derive(Debug, Clone)]
pub enum UpdaterMessage {
    /// Boot delay, periodic tick or window focus.
    Check {
        forced: bool,
    },
    /// "Check for updates" in Settings: a forced check whose outcome is
    /// reported even when there is nothing new.
    CheckRequestedByUser,
    Checked(Result<Option<UpdateInfo>, String>),
    Install,
    Later,
    Event(InstallEvent),
    Retry,
    Quit,
}

#[derive(Debug, Default)]
pub struct UpdaterState {
    pub phase: Phase,
    /// Version the user postponed with "Later"; offered again next launch.
    pub dismissed_version: Option<String>,
    pub attempts: u32,
    pub checking: bool,
    /// The running check was asked for by the user; its outcome is shown.
    pub report_outcome: bool,
    pub last_check: Option<std::time::Instant>,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The `.app` bundle the running binary lives in. `None` for `cargo run`
/// builds, which never update themselves.
pub fn installed_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn client() -> Result<&'static reqwest::Client, String> {
    HTTP.as_ref()
        .ok_or_else(|| t!("updater.error.http_client_unavailable").into_owned())
}

/// Fetch the manifest and return the update when it is newer than the
/// running version. The timestamp query defeats the GitHub CDN cache,
/// which otherwise serves the previous `latest.json` for minutes after a
/// release.
pub async fn check() -> Result<Option<UpdateInfo>, String> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) || installed_bundle().is_none() {
        return Ok(None);
    }
    let stamp = jiff::Timestamp::now().as_millisecond();
    let response = client()?
        .get(format!("{MANIFEST_URL}?ts={stamp}"))
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("manifest request: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("manifest request: HTTP {}", response.status()));
    }
    let manifest: Manifest = response
        .json()
        .await
        .map_err(|error| format!("manifest parse: {error}"))?;
    if !is_newer(&manifest.version, current_version()) {
        return Ok(None);
    }
    let Some(entry) = manifest.platforms.get(PLATFORM) else {
        return Ok(None);
    };
    if !is_release_archive_url(&entry.url) {
        return Err(format!(
            "manifest: archive URL outside {RELEASE_DOWNLOAD_PREFIX}"
        ));
    }
    Ok(Some(UpdateInfo {
        version: manifest.version,
        notes: manifest.notes,
        url: entry.url.clone(),
        signature: entry.signature.clone(),
    }))
}

/// Download, verify and swap the bundle, reporting progress on `events`.
/// The caller relaunches with [`relaunch_after_exit`] once `Done(Ok)`
/// arrives.
pub async fn install(
    info: UpdateInfo,
    mut events: iced::futures::channel::mpsc::Sender<InstallEvent>,
) {
    let result = install_inner(&info, &mut events).await;
    let _ = events.try_send(InstallEvent::Done(result));
}

async fn install_inner(
    info: &UpdateInfo,
    events: &mut iced::futures::channel::mpsc::Sender<InstallEvent>,
) -> Result<(), String> {
    let bundle =
        installed_bundle().ok_or_else(|| t!("updater.error.not_installed").into_owned())?;
    let mut response = client()?
        .get(&info.url)
        .send()
        .await
        .map_err(|error| t!("updater.error.download", error = error).into_owned())?;
    if !response.status().is_success() {
        return Err(t!("updater.error.download_http", status = response.status()).into_owned());
    }
    let total = response.content_length();
    if total.is_some_and(|size| size > MAX_ARCHIVE_BYTES) {
        return Err(t!("updater.error.archive_too_large").into_owned());
    }
    let mut archive = Vec::with_capacity(usize::try_from(total.unwrap_or(0)).unwrap_or(0));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| t!("updater.error.download", error = error).into_owned())?
    {
        archive.extend_from_slice(&chunk);
        if archive.len() as u64 > MAX_ARCHIVE_BYTES {
            return Err(t!("updater.error.archive_too_large").into_owned());
        }
        let _ = events.try_send(InstallEvent::Progress {
            downloaded: archive.len() as u64,
            total,
        });
    }

    verify_signature(&archive, &info.signature, &info.version)?;
    let _ = events.try_send(InstallEvent::Installing);
    tokio::task::spawn_blocking(move || replace_bundle(&bundle, &archive))
        .await
        .map_err(|error| t!("updater.error.install", error = error).into_owned())?
}

fn decode_base64_text(value: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(value.trim())
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

/// Check the minisign signature and that its trusted comment (covered by
/// the signature) names the announced version, so a tampered manifest
/// cannot pass an older signed archive off as a newer release.
fn verify_signature(archive: &[u8], signature: &str, version: &str) -> Result<(), String> {
    let public_key = decode_base64_text(PUBLIC_KEY)
        .and_then(|text| minisign_verify::PublicKey::decode(&text).map_err(|e| e.to_string()))
        .map_err(|error| t!("updater.error.public_key", error = error).into_owned())?;
    let signature = decode_base64_text(signature)
        .and_then(|text| minisign_verify::Signature::decode(&text).map_err(|e| e.to_string()))
        .map_err(|error| t!("updater.error.signature_decode", error = error).into_owned())?;
    public_key
        .verify(archive, &signature, false)
        .map_err(|_| t!("updater.error.signature_invalid").into_owned())?;
    let expected = format!("version:{version}");
    if signature
        .trusted_comment()
        .split('\t')
        .any(|field| field == expected)
    {
        Ok(())
    } else {
        Err(t!("updater.error.signature_wrong_version", version = version).into_owned())
    }
}

/// Extract next to the current bundle (same volume, so the swap is two
/// renames) and put the new bundle in place, restoring the old one if the
/// second rename fails.
fn replace_bundle(bundle: &Path, archive: &[u8]) -> Result<(), String> {
    let parent = bundle
        .parent()
        .ok_or_else(|| t!("updater.error.bundle_no_parent").into_owned())?;
    let name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| t!("updater.error.bundle_name_not_utf8").into_owned())?;
    let staging = parent.join(format!(".{name}.update"));
    let previous = parent.join(format!(".{name}.previous"));
    for leftover in [&staging, &previous] {
        if leftover.exists() {
            std::fs::remove_dir_all(leftover).map_err(|error| {
                t!(
                    "updater.error.clean",
                    path = leftover.display(),
                    error = error
                )
                .into_owned()
            })?;
        }
    }
    std::fs::create_dir_all(&staging).map_err(|error| {
        t!(
            "updater.error.cannot_write",
            path = bundle.display(),
            error = error
        )
        .into_owned()
    })?;

    let archive_path = staging.join("update.tar.gz");
    std::fs::write(&archive_path, archive)
        .map_err(|error| t!("updater.error.install", error = error).into_owned())?;
    let status = std::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&staging)
        .status()
        .map_err(|error| t!("updater.error.tar", error = error).into_owned())?;
    if !status.success() {
        return Err(t!("updater.error.tar_status", status = status).into_owned());
    }
    let new_bundle = staging.join(name);
    if !new_bundle.join("Contents/Info.plist").is_file() {
        return Err(t!("updater.error.archive_missing_bundle", name = name).into_owned());
    }

    std::fs::rename(bundle, &previous)
        .map_err(|error| t!("updater.error.install", error = error).into_owned())?;
    if let Err(error) = std::fs::rename(&new_bundle, bundle) {
        let _ = std::fs::rename(&previous, bundle);
        return Err(t!("updater.error.install", error = error).into_owned());
    }
    for leftover in [&staging, &previous] {
        if let Err(error) = std::fs::remove_dir_all(leftover) {
            log::warn!("updater: could not remove {}: {error}", leftover.display());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn relaunch_after_exit() -> Result<(), String> {
    Err("relaunch is only supported on macOS".to_string())
}

/// Start a detached shell that waits for this process to exit, then opens
/// the bundle again.
#[cfg(unix)]
pub fn relaunch_after_exit() -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;

    let bundle = installed_bundle().ok_or("Courly is not running from an installed app")?;
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("while /bin/kill -0 \"$0\" 2>/dev/null; do /bin/sleep 0.2; done; /usr/bin/open \"$1\"")
        .arg(std::process::id().to_string())
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("relaunch: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_compare_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn malformed_versions_are_never_newer() {
        assert!(!is_newer("0.2", "0.1.0"));
        assert!(!is_newer("0.2.0-beta", "0.1.0"));
        assert!(!is_newer("0.2.0.1", "0.1.0"));
        assert!(!is_newer("latest", "0.1.0"));
    }

    #[test]
    fn embedded_public_key_decodes() {
        let text = decode_base64_text(PUBLIC_KEY).expect("base64");
        minisign_verify::PublicKey::decode(&text).expect("minisign public key");
    }

    /// `tauri signer sign --app-version 9.9.9` of the bytes `hello\n` with
    /// the release key.
    const HELLO_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUd2xSVTExaHJkQ0ZBLzlnbk5ETlpCbi9sb0QwYkNYNFRucHhxWDIzNXhmZTljbEFmakpkTXBaM3lkN3I0eWsrVk1vV0lCK29YKzdteW1Mdjh2NjZsVUJDcm5HU1NDckFZPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMzEwMDY5CWZpbGU6YS5iaW4JdmVyc2lvbjo5LjkuOQpzbzlWaHY4d2xHS3NPRllOdmRzQXFrc0poc0dYMmwwQ1A0a3FyMWVOc3VmazQvZ3RPUXBRbjl2QitBVnAyVXZVN0xZMVNwckRkMm5XYU1NTkhOV3ZEdz09Cg==";

    #[test]
    fn release_signature_is_accepted_for_its_version() {
        verify_signature(b"hello\n", HELLO_SIGNATURE, "9.9.9").expect("valid signature");
    }

    #[test]
    fn release_signature_is_bound_to_its_version() {
        assert!(verify_signature(b"hello\n", HELLO_SIGNATURE, "10.0.0").is_err());
    }

    #[test]
    fn tampered_payload_is_rejected() {
        assert!(verify_signature(b"hello!\n", HELLO_SIGNATURE, "9.9.9").is_err());
    }

    #[test]
    fn garbage_signature_is_rejected() {
        let bogus = STANDARD.encode("untrusted comment: x\nnot-a-signature\n");
        assert!(verify_signature(b"payload", &bogus, "0.1.0").is_err());
        assert!(verify_signature(b"payload", "%%%", "0.1.0").is_err());
    }

    #[test]
    fn archive_url_must_be_one_of_our_releases() {
        assert!(is_release_archive_url(
            "https://github.com/Soflutionltd/courly/releases/download/v0.2.7/Courly.app.tar.gz"
        ));
        for foreign in [
            "https://evil.tld/Courly.app.tar.gz",
            "http://github.com/Soflutionltd/courly/releases/download/v1/x.tar.gz",
            "https://github.com/Soflutionltd/courly-fork/releases/download/v1/x.tar.gz",
            "https://github.com/Soflutionltd/courly/releases/download/../../other/x",
            "https://github.com/Soflutionltd/courly/releases/download/",
        ] {
            assert!(!is_release_archive_url(foreign), "{foreign}");
        }
    }

    #[test]
    fn replace_bundle_swaps_in_the_new_app() {
        let root = tempfile::tempdir().expect("tempdir");
        let bundle = root.path().join("Courly.app");
        std::fs::create_dir_all(bundle.join("Contents")).expect("old bundle");
        std::fs::write(bundle.join("Contents/Info.plist"), "old").expect("old plist");

        let source = tempfile::tempdir().expect("source");
        let new_bundle = source.path().join("Courly.app/Contents");
        std::fs::create_dir_all(&new_bundle).expect("new bundle");
        std::fs::write(new_bundle.join("Info.plist"), "new").expect("new plist");
        let archive = source.path().join("update.tar.gz");
        let status = std::process::Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(source.path())
            .arg("Courly.app")
            .status()
            .expect("tar");
        assert!(status.success());

        replace_bundle(&bundle, &std::fs::read(&archive).expect("archive")).expect("replace");
        let plist = std::fs::read_to_string(bundle.join("Contents/Info.plist")).expect("plist");
        assert_eq!(plist, "new");
        assert!(!root.path().join(".Courly.app.update").exists());
        assert!(!root.path().join(".Courly.app.previous").exists());
    }

    #[test]
    fn replace_bundle_keeps_the_old_app_when_the_archive_is_wrong() {
        let root = tempfile::tempdir().expect("tempdir");
        let bundle = root.path().join("Courly.app");
        std::fs::create_dir_all(bundle.join("Contents")).expect("old bundle");
        std::fs::write(bundle.join("Contents/Info.plist"), "old").expect("old plist");

        let source = tempfile::tempdir().expect("source");
        std::fs::create_dir_all(source.path().join("Other.app")).expect("other");
        let archive = source.path().join("update.tar.gz");
        let status = std::process::Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(source.path())
            .arg("Other.app")
            .status()
            .expect("tar");
        assert!(status.success());

        assert!(replace_bundle(&bundle, &std::fs::read(&archive).expect("archive")).is_err());
        let plist = std::fs::read_to_string(bundle.join("Contents/Info.plist")).expect("plist");
        assert_eq!(plist, "old");
    }
}
