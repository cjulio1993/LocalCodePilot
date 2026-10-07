use eframe::egui;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};

const RELEASES_URL: &str =
    "https://api.github.com/repos/cjulio1993/LocalCodePilot/releases?per_page=20";
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub title: String,
    pub release_url: String,
    pub download_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("the workspace package version must follow semantic versioning")
}

pub fn cached_update_is_newer(update: &UpdateInfo) -> bool {
    parse_version(&update.version).is_some_and(|version| version > current_version())
}

pub fn spawn_update_check(repaint: egui::Context) -> Receiver<Result<Option<UpdateInfo>, String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = fetch_update(&current_version());
        let _ = sender.send(result);
        repaint.request_repaint();
    });
    receiver
}

fn fetch_update(current: &Version) -> Result<Option<UpdateInfo>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "LocalCodePilot-Update-Checker")
        .call()
        .map_err(|error| format!("não foi possível consultar as releases: {error}"))?;
    let releases: Vec<GithubRelease> = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("resposta de atualização inválida: {error}"))?;
    Ok(select_update(releases, current))
}

fn select_update(releases: Vec<GithubRelease>, current: &Version) -> Option<UpdateInfo> {
    releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = parse_version(&release.tag_name)?;
            if version <= *current
                || (current.pre.is_empty() && (release.prerelease || !version.pre.is_empty()))
            {
                return None;
            }
            let download_url = release
                .assets
                .iter()
                .find(|asset| windows_portable_asset(&asset.name))
                .map(|asset| asset.browser_download_url.clone())
                .unwrap_or_else(|| release.html_url.clone());
            let title = release
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("LocalCodePilot {version}"));
            Some((
                version.clone(),
                UpdateInfo {
                    version: version.to_string(),
                    title,
                    release_url: release.html_url,
                    download_url,
                },
            ))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, update)| update)
}

fn parse_version(value: &str) -> Option<Version> {
    Version::parse(value.trim().trim_start_matches(['v', 'V'])).ok()
}

fn windows_portable_asset(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".zip")
        && name.contains("windows")
        && (name.contains("x64") || name.contains("amd64"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn releases(json: &str) -> Vec<GithubRelease> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn selects_the_newest_alpha_and_its_windows_asset() {
        let update = select_update(
            releases(
                r#"[
                    {
                        "tag_name":"v0.4.0-alpha.1",
                        "name":"LocalCodePilot 0.4.0-alpha.1",
                        "html_url":"https://example.com/release",
                        "prerelease":true,
                        "assets":[{
                            "name":"LocalCodePilot-0.4.0-alpha.1-windows-x64.zip",
                            "browser_download_url":"https://example.com/windows.zip"
                        }]
                    },
                    {
                        "tag_name":"v0.3.0-alpha.2",
                        "html_url":"https://example.com/older",
                        "prerelease":true
                    }
                ]"#,
            ),
            &Version::parse("0.3.0-alpha.1").unwrap(),
        )
        .unwrap();

        assert_eq!(update.version, "0.4.0-alpha.1");
        assert_eq!(update.download_url, "https://example.com/windows.zip");
    }

    #[test]
    fn stable_channel_ignores_prereleases() {
        let update = select_update(
            releases(
                r#"[
                    {
                        "tag_name":"v0.5.0-alpha.1",
                        "html_url":"https://example.com/alpha",
                        "prerelease":true
                    },
                    {
                        "tag_name":"v0.4.1",
                        "html_url":"https://example.com/stable",
                        "prerelease":false
                    }
                ]"#,
            ),
            &Version::parse("0.4.0").unwrap(),
        )
        .unwrap();

        assert_eq!(update.version, "0.4.1");
    }

    #[test]
    fn ignores_drafts_and_versions_that_are_not_newer() {
        let update = select_update(
            releases(
                r#"[
                    {
                        "tag_name":"v0.4.0-alpha.2",
                        "html_url":"https://example.com/draft",
                        "draft":true,
                        "prerelease":true
                    },
                    {
                        "tag_name":"v0.4.0-alpha.1",
                        "html_url":"https://example.com/current",
                        "prerelease":true
                    }
                ]"#,
            ),
            &Version::parse("0.4.0-alpha.1").unwrap(),
        );

        assert!(update.is_none());
    }
}
