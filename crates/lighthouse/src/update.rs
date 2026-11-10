//! Self-update from GitHub releases.
//!
//! A release provides one plain binary per platform (see [`asset_name`]) and a `SHA256SUMS` file in
//! `sha256sum` format. An update is only installed when the binary's checksum matches.
//!
//! After installing, Lighthouse restarts itself: under a service manager (`LIGHTHOUSE_SUPERVISED`
//! set, as in the systemd unit) it exits with [`EXIT_RESTART`] and lets the manager start it again;
//! otherwise it launches the new binary with the same arguments and exits.
//! Setting `LIGHTHOUSE_NO_SELF_UPDATE` (as the Docker image does) limits it to reporting new versions.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use lighthouse_core::UpdateConfig;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Exit code asking a service manager to restart Lighthouse after an update.
pub const EXIT_RESTART: i32 = 75;

const CHECKSUM_FILE: &str = "SHA256SUMS";

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("package version is valid semver")
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
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub page_url: String,
    binary_url: Option<String>,
    sums_url: Option<String>,
}

fn parse_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

/// Pick the highest usable release from a GitHub release list.
fn pick(releases: Vec<GhRelease>, prerelease: bool) -> Option<Release> {
    releases
        .into_iter()
        .filter(|r| !r.draft && (prerelease || !r.prerelease))
        .filter_map(|r| {
            let version = parse_tag(&r.tag_name)?;
            let find = |name: &str| {
                r.assets
                    .iter()
                    .find(|a| a.name == name)
                    .map(|a| a.browser_download_url.clone())
            };
            Some(Release {
                version,
                page_url: r.html_url.clone(),
                binary_url: asset_name().and_then(find),
                sums_url: find(CHECKSUM_FILE),
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// The newest release that is newer than this build, if any.
pub async fn check(config: &UpdateConfig) -> anyhow::Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{}/releases?per_page=20", config.repo);
    let resp = crate::http::client()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("could not reach GitHub ({url})"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("GitHub repository {} not found", config.repo);
    }
    let releases: Vec<GhRelease> = resp
        .error_for_status()?
        .json()
        .await
        .context("unexpected reply from GitHub")?;
    Ok(pick(releases, config.prerelease).filter(|r| r.version > current_version()))
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

    info!(version = %release.version, "downloading update");
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
    info!(version = %release.version, "update installed");
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
            Ok(None) => info!("Lighthouse is up to date"),
            Ok(Some(release)) if config.auto_install && self_update_allowed() => match install(&release).await {
                Ok(()) => restart(),
                Err(e) => warn!("update to {} failed: {e:#}", release.version),
            },
            Ok(Some(release)) => {
                warn!(
                    "Lighthouse {} is available ({}). Run `lighthouse update` to install it.",
                    release.version, release.page_url
                )
            }
            Err(e) => warn!("update check failed: {e:#}"),
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, prerelease: bool, with_assets: bool) -> GhRelease {
        let assets = if with_assets {
            vec![
                GhAsset {
                    name: CHECKSUM_FILE.into(),
                    browser_download_url: format!("https://x/{tag}/sums"),
                },
                GhAsset {
                    name: asset_name().unwrap_or("none").into(),
                    browser_download_url: format!("https://x/{tag}/bin"),
                },
            ]
        } else {
            Vec::new()
        };
        GhRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            html_url: format!("https://x/{tag}"),
            assets,
        }
    }

    #[test]
    fn picks_highest_stable_release() {
        let list = vec![
            rel("v1.2.0", false, true),
            rel("v1.10.0", false, true),
            rel("v2.0.0-rc.1", true, true),
        ];
        assert_eq!(pick(list, false).unwrap().version, Version::new(1, 10, 0));
    }

    #[test]
    fn prerelease_opt_in() {
        let list = vec![rel("v1.2.0", false, true), rel("v2.0.0-rc.1", true, true)];
        assert_eq!(pick(list, true).unwrap().version, Version::parse("2.0.0-rc.1").unwrap());
    }

    #[test]
    fn ignores_bad_tags_and_finds_assets() {
        let r = pick(vec![rel("nightly", false, true), rel("0.3.0", false, true)], false).unwrap();
        assert_eq!(r.version, Version::new(0, 3, 0));
        assert_eq!(r.sums_url.as_deref(), Some("https://x/0.3.0/sums"));
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
