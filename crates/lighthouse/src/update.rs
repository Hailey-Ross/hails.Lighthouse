//! Self-update from this project's GitHub releases, by channel.
//!
//! Each channel is a rolling release: `stable` (rebuilt from `main`) and `bleeding-edge` (rebuilt
//! from every commit to `dev`). A release carries one plain binary per platform (see
//! [`asset_name`]), `SHA256SUMS`, and `release.json` with its channel, build number, commit and
//! version. Official builds have the same details baked in (see [`this_build`]), so the updater
//! knows whether the channel has something newer. An update is only installed when the binary's
//! checksum matches.
//!
//! After installing, Lighthouse restarts itself: under a service manager (`LIGHTHOUSE_SUPERVISED`
//! set, as in the systemd unit) it exits with [`EXIT_RESTART`] and lets the manager start it again;
//! otherwise it launches the new binary with the same arguments and exits.
//! Setting `LIGHTHOUSE_NO_SELF_UPDATE` (as the Docker image does) limits it to reporting new builds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use lighthouse_core::UpdateConfig;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Exit code asking a service manager to restart Lighthouse after an update.
pub const EXIT_RESTART: i32 = 75;

const CHECKSUM_FILE: &str = "SHA256SUMS";
const RELEASE_INFO: &str = "release.json";

/// What this program is. Release builds get these from the build workflow; local builds have none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub channel: Option<&'static str>,
    pub build: Option<u64>,
    pub revision: Option<&'static str>,
}

pub fn this_build() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        channel: option_env!("LIGHTHOUSE_CHANNEL").filter(|c| !c.is_empty()),
        build: option_env!("LIGHTHOUSE_BUILD").and_then(|b| b.parse().ok()),
        revision: option_env!("LIGHTHOUSE_REVISION").filter(|r| !r.is_empty()),
    }
}

impl BuildInfo {
    /// `0.1.0, build 12 (stable, commit 947b283ad9ca)` or `0.1.0 (local build)`.
    pub fn describe(&self) -> String {
        match (self.build, self.channel) {
            (Some(b), Some(c)) => {
                let commit = self
                    .revision
                    .map(|r| format!(", commit {}", &r[..r.len().min(12)]))
                    .unwrap_or_default();
                format!("{}, build {b} ({c}{commit})", self.version)
            }
            _ => format!("{} (local build)", self.version),
        }
    }
}

/// `--version` text, computed once.
pub fn version_text() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| this_build().describe())
}

/// Release asset name for this platform, or `None` where no prebuilt binary is published.
pub fn asset_name() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("lighthouse-x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("lighthouse-aarch64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("lighthouse-x86_64-pc-windows-msvc.exe")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("lighthouse-aarch64-apple-darwin")
    } else {
        None
    }
}

pub fn self_update_allowed() -> bool {
    std::env::var_os("LIGHTHOUSE_NO_SELF_UPDATE").is_none()
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// `release.json` of a channel.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub channel: String,
    pub build: u64,
    pub revision: String,
    #[serde(default)]
    pub version: String,
}

/// A build available on the configured channel.
#[derive(Debug, Clone)]
pub struct Release {
    pub info: ReleaseInfo,
    pub page_url: String,
    binary_url: Option<String>,
    sums_url: Option<String>,
}

impl Release {
    /// `0.1.0, build 14 (stable)`
    pub fn describe(&self) -> String {
        let version = if self.info.version.is_empty() {
            "build"
        } else {
            self.info.version.as_str()
        };
        format!("{version}, build {} ({})", self.info.build, self.info.channel)
    }
}

/// Is `remote` (from `channel`) something this build should move to?
///
/// - Same channel: a higher build number.
/// - Switched channel (e.g. bleeding-edge to stable): any different commit.
/// - A local build (no build number): never automatically; `lighthouse update` still offers it.
pub fn is_newer(local: &BuildInfo, remote: &ReleaseInfo) -> bool {
    match (local.channel, local.build) {
        (Some(c), Some(b)) if c == remote.channel => remote.build > b,
        (Some(_), Some(_)) => local.revision != Some(remote.revision.as_str()),
        _ => false,
    }
}

/// The current release on the configured channel (whether or not it is newer).
pub async fn latest(config: &UpdateConfig) -> anyhow::Result<Release> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        config.repo, config.channel
    );
    let http = crate::http::client();
    let resp = http
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("could not reach GitHub ({url})"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("no {} release found in {}", config.channel, config.repo);
    }
    let release: GhRelease = resp
        .error_for_status()?
        .json()
        .await
        .context("unexpected reply from GitHub")?;
    let find = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
    };
    let info_url =
        find(RELEASE_INFO).with_context(|| format!("the {} release has no {RELEASE_INFO}", config.channel))?;
    let info: ReleaseInfo = http
        .get(&info_url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("unreadable release.json")?;
    Ok(Release {
        info,
        page_url: release.html_url.clone(),
        binary_url: asset_name().and_then(find),
        sums_url: find(CHECKSUM_FILE),
    })
}

/// The channel's release, if it is newer than this build.
pub async fn check(config: &UpdateConfig) -> anyhow::Result<Option<Release>> {
    let release = latest(config).await?;
    Ok(is_newer(&this_build(), &release.info).then_some(release))
}

/// Find the checksum for `name` in `sha256sum` output (`<hex>  <name>` or `<hex> *<name>`).
fn checksum_for<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then_some(hash)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download, verify and swap in the new binary. Does not restart.
pub async fn install(release: &Release) -> anyhow::Result<()> {
    if !self_update_allowed() {
        bail!("self-update is disabled here (LIGHTHOUSE_NO_SELF_UPDATE); update the container image instead");
    }
    let name = asset_name().context("no prebuilt binary is published for this platform")?;
    let binary_url = release
        .binary_url
        .as_deref()
        .with_context(|| format!("release has no {name}"))?;
    let sums_url = release
        .sums_url
        .as_deref()
        .context("release has no SHA256SUMS file; refusing to install")?;

    let http = crate::http::client();
    let sums = http.get(sums_url).send().await?.error_for_status()?.text().await?;
    let expected = checksum_for(&sums, name).with_context(|| format!("SHA256SUMS has no entry for {name}"))?;

    info!(release = %release.describe(), "downloading update");
    let bytes = http.get(binary_url).send().await?.error_for_status()?.bytes().await?;
    let actual = hex(&Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("checksum mismatch for {name}: expected {expected}, got {actual}; update not installed");
    }

    let exe = std::env::current_exe().context("cannot locate the running executable")?;
    let staged = staged_path(&exe);
    std::fs::write(&staged, &bytes).with_context(|| format!("cannot write {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    let result = self_replace::self_replace(&staged).context("could not replace the executable");
    let _ = std::fs::remove_file(&staged);
    result?;
    info!(release = %release.describe(), "update installed");
    Ok(())
}

fn staged_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".update");
    exe.with_file_name(name)
}

/// Restart into the freshly installed binary. Never returns.
pub fn restart() -> ! {
    if std::env::var_os("LIGHTHOUSE_SUPERVISED").is_some() {
        info!("exiting so the service manager restarts the new version");
        std::process::exit(EXIT_RESTART);
    }
    match std::env::current_exe().map_err(anyhow::Error::from).and_then(|exe| {
        Ok(std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .spawn()?)
    }) {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            warn!("could not start the new version ({e}); exiting, start Lighthouse again manually");
            std::process::exit(EXIT_RESTART);
        }
    }
}

/// Background task for `run`: check periodically, install when `auto_install` is on.
pub async fn service(config: UpdateConfig) {
    if !config.check {
        return;
    }
    // Let the bots come up before the first check.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let every = Duration::from_secs(config.check_interval_hours * 3600);
    loop {
        match check(&config).await {
            Ok(None) => info!("Lighthouse {} is up to date", this_build().describe()),
            Ok(Some(release)) if config.auto_install && self_update_allowed() => match install(&release).await {
                Ok(()) => restart(),
                Err(e) => warn!("update to {} failed: {e:#}", release.describe()),
            },
            Ok(Some(release)) => warn!(
                "Lighthouse {} is available ({}). Run `lighthouse update` to install it.",
                release.describe(),
                release.page_url
            ),
            Err(e) => warn!("update check failed: {e:#}"),
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(channel: Option<&'static str>, build: Option<u64>, revision: Option<&'static str>) -> BuildInfo {
        BuildInfo {
            version: "0.1.0",
            channel,
            build,
            revision,
        }
    }

    fn remote(channel: &str, build: u64, revision: &str) -> ReleaseInfo {
        ReleaseInfo {
            channel: channel.into(),
            build,
            revision: revision.into(),
            version: "0.1.0".into(),
        }
    }

    #[test]
    fn newer_builds_on_the_same_channel() {
        let me = local(Some("stable"), Some(12), Some("aaa"));
        assert!(is_newer(&me, &remote("stable", 13, "bbb")));
        assert!(!is_newer(&me, &remote("stable", 12, "aaa")));
        assert!(!is_newer(&me, &remote("stable", 11, "ccc")));
    }

    #[test]
    fn switching_channel_moves_to_any_other_commit() {
        let me = local(Some("bleeding-edge"), Some(80), Some("aaa"));
        assert!(is_newer(&me, &remote("stable", 12, "bbb")));
        assert!(!is_newer(&me, &remote("stable", 12, "aaa")));
    }

    #[test]
    fn local_builds_are_never_replaced_automatically() {
        assert!(!is_newer(&local(None, None, None), &remote("stable", 99, "bbb")));
    }

    #[test]
    fn describes_builds() {
        assert_eq!(
            local(Some("stable"), Some(12), Some("947b283ad9ca75c4")).describe(),
            "0.1.0, build 12 (stable, commit 947b283ad9ca)"
        );
        assert_eq!(local(None, None, None).describe(), "0.1.0 (local build)");
    }

    #[test]
    fn reads_release_json() {
        let info: ReleaseInfo = serde_json::from_str(
            r#"{"channel":"stable","build":44,"revision":"947b","version":"0.1.0","assets":{"x":"y"}}"#,
        )
        .unwrap();
        assert_eq!(info, remote("stable", 44, "947b"));
    }

    #[test]
    fn checksum_lookup() {
        let sums = "abc123  lighthouse-x86_64-unknown-linux-gnu\ndef456 *lighthouse-x86_64-pc-windows-msvc.exe\n";
        assert_eq!(
            checksum_for(sums, "lighthouse-x86_64-unknown-linux-gnu"),
            Some("abc123")
        );
        assert_eq!(
            checksum_for(sums, "lighthouse-x86_64-pc-windows-msvc.exe"),
            Some("def456")
        );
        assert_eq!(checksum_for(sums, "other"), None);
    }

    #[test]
    fn hex_encoding() {
        assert_eq!(
            hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
