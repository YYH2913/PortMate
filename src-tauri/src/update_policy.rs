//! Release identity and package selection shared by the updater and its tests.
use semver::Version;
use serde::Deserialize;
use url::Url;

pub(super) const RELEASES_URL: &str = "https://github.com/YYH2913/PortMate/releases";
pub(super) const RELEASE_API: &str =
    "https://api.github.com/repos/YYH2913/PortMate/releases/latest";

#[derive(Debug, Deserialize)]
pub(super) struct GithubRelease {
    pub(super) tag_name: String,
    pub(super) draft: bool,
    pub(super) prerelease: bool,
    pub(super) body: Option<String>,
    pub(super) published_at: Option<String>,
    #[serde(default)]
    pub(super) assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GithubAsset {
    pub(super) name: String,
    pub(super) browser_download_url: String,
}

pub(super) fn stable_release_version(release: &GithubRelease) -> Result<Version, &'static str> {
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|_| "invalid-release")?;
    if release.draft || release.prerelease || !version.pre.is_empty() {
        return Err("invalid-release");
    }
    Ok(version)
}

pub(super) fn release_url(tag: &str) -> String {
    let mut url = Url::parse(RELEASES_URL).expect("constant release URL");
    url.path_segments_mut()
        .expect("GitHub base URL")
        .push("tag")
        .push(tag);
    url.to_string()
}

/// Only assets in the selected release of the configured repository are used.
pub(super) fn is_release_asset(url: &Url, tag: &str) -> bool {
    let mut base = Url::parse(RELEASES_URL).expect("constant release URL");
    base.path_segments_mut()
        .expect("GitHub base URL")
        .push("download")
        .push(tag);
    let prefix = format!("{}/", base.as_str());
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.as_str().starts_with(&prefix)
        && url.as_str()[prefix.len()..].split('/').count() == 1
        && url.as_str().len() > prefix.len()
}

pub(super) fn updater_target(os: &str, arch: &str, package: &str) -> Option<String> {
    if !matches!(arch, "x86_64" | "aarch64") {
        return None;
    }
    match (os, package) {
        ("windows", "msi" | "nsis") => Some(format!("windows-{arch}-{package}")),
        ("linux", "appimage") => Some(format!("linux-{arch}-appimage")),
        ("macos", "app") => Some(format!("darwin-{arch}-app")),
        _ => None,
    }
}

pub(super) fn asset_matches_target(url: &Url, target: &str) -> bool {
    let path = url.path().to_ascii_lowercase();
    if target.ends_with("-msi") {
        path.ends_with(".msi")
    } else if target.ends_with("-nsis") {
        path.ends_with("-setup.exe")
    } else if target.ends_with("-appimage") {
        path.ends_with(".appimage")
    } else if target.ends_with("-app") {
        path.ends_with(".app.tar.gz")
    } else {
        false
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn appimage_header(header: &[u8]) -> bool {
    header.len() >= 11
        && header.starts_with(b"\x7fELF")
        && &header[8..10] == b"AI"
        && matches!(header[10], 1 | 2)
}

#[cfg(test)]
#[path = "../../test/rust/portmate/unit/update_policy.rs"]
mod tests;
