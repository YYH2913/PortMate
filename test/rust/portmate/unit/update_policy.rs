use super::*;

fn release(tag: &str) -> GithubRelease {
    GithubRelease {
        tag_name: tag.into(),
        draft: false,
        prerelease: false,
        body: None,
        published_at: None,
        assets: vec![],
    }
}

#[test]
fn stable_versions_use_semantic_order_and_exclude_previews() {
    assert!(
        stable_release_version(&release("v0.1.14")).unwrap()
            > stable_release_version(&release("0.1.9")).unwrap()
    );
    for tag in ["latest", "0.2", "0.2.0-beta.1", "../other"] {
        assert!(stable_release_version(&release(tag)).is_err(), "{tag}");
    }
    let mut preview = release("0.2.0");
    preview.prerelease = true;
    assert!(stable_release_version(&preview).is_err());
    preview.prerelease = false;
    preview.draft = true;
    assert!(stable_release_version(&preview).is_err());
}

#[test]
fn updater_only_accepts_the_selected_repository_and_tag() {
    let good = "https://github.com/YYH2913/PortMate/releases/download/v0.2.0/PortMate.exe";
    assert!(is_release_asset(&Url::parse(good).unwrap(), "v0.2.0"));
    assert_eq!(
        release_url("v0.2.0"),
        "https://github.com/YYH2913/PortMate/releases/tag/v0.2.0"
    );
    for url in [
        good.replace("https:", "http:"),
        good.replace("github.com", "github.com.evil.test"),
        good.replace("PortMate/", "other/"),
        good.replace("v0.2.0/", "v0.1.0/"),
        format!("{good}?redirect=other"),
        format!("{good}#fragment"),
        format!("{good}/another"),
    ] {
        assert!(
            !is_release_asset(&Url::parse(&url).unwrap(), "v0.2.0"),
            "{url}"
        );
    }
}

#[test]
fn target_selection_preserves_install_type() {
    assert_eq!(
        updater_target("windows", "x86_64", "msi").as_deref(),
        Some("windows-x86_64-msi")
    );
    assert_eq!(
        updater_target("windows", "x86_64", "nsis").as_deref(),
        Some("windows-x86_64-nsis")
    );
    assert_eq!(
        updater_target("macos", "aarch64", "app").as_deref(),
        Some("darwin-aarch64-app")
    );
    assert_eq!(
        updater_target("linux", "x86_64", "appimage").as_deref(),
        Some("linux-x86_64-appimage")
    );
    for (os, arch, package) in [
        ("linux", "x86_64", "deb"),
        ("linux", "x86_64", "rpm"),
        ("windows", "x86_64", "zip"),
        ("linux", "riscv64", "appimage"),
    ] {
        assert_eq!(updater_target(os, arch, package), None);
    }
}

#[test]
fn github_metadata_retains_release_information_and_assets() {
    let parsed: GithubRelease = serde_json::from_value(serde_json::json!({
        "tag_name":"0.2.0", "draft":false, "prerelease":false,
        "body":"English / 中文", "published_at":"2026-10-20T00:00:00Z",
        "assets":[{"name":"latest.json", "browser_download_url":"https://github.com/YYH2913/PortMate/releases/download/0.2.0/latest.json"}]
    })).unwrap();
    assert_eq!(parsed.body.as_deref(), Some("English / 中文"));
    assert!(parsed.published_at.is_some());
    assert_eq!(parsed.assets[0].name, "latest.json");
    assert!(is_release_asset(
        &Url::parse(&parsed.assets[0].browser_download_url).unwrap(),
        "0.2.0"
    ));
    assert_eq!(
        RELEASE_API,
        "https://api.github.com/repos/YYH2913/PortMate/releases/latest"
    );
}

#[test]
fn update_packages_must_match_the_selected_installer() {
    let msi =
        Url::parse("https://github.com/YYH2913/PortMate/releases/download/0.2.0/PortMate.msi")
            .unwrap();
    assert!(asset_matches_target(&msi, "windows-x86_64-msi"));
    assert!(!asset_matches_target(&msi, "windows-x86_64-nsis"));
    assert!(!asset_matches_target(&msi, "darwin-aarch64-app"));
}

#[test]
fn extracted_portable_elf_is_not_an_appimage() {
    assert!(appimage_header(b"\x7fELF\x02\x01\x01\0AI\x02"));
    assert!(!appimage_header(b"\x7fELF\x02\x01\x01\0\0\0\0"));
    assert!(!appimage_header(b"short"));
}
