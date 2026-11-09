//! Updating from the GitHub releases of the repository. GitHub is only
//! contacted when the user asks to check, and only over HTTPS to GitHub's
//! own hosts.
//!
//! A release needs a version tag (`0.2.0` or `v0.2.0`) and a zip like
//! `GUC-Station-0.2.0-windows-x64.zip` containing `GUC Station/GUC Station.exe`.
//! Pre-releases count, since early versions are published that way.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const REPO: &str = "WISPOFLIFE/guc-station";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_PAGE: &str = "https://github.com/WISPOFLIFE/guc-station/releases";
const EXE_NAME: &str = "GUC Station.exe";
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// Reads `1.2.3`, `v1.2.3` or `1.2`; anything after `-` or `+` is ignored.
    pub fn parse(tag: &str) -> Option<Version> {
        let core = tag.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let major = parts.next()??;
        let minor = parts.next()??;
        let patch = parts.next().unwrap_or(Some(0))?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version(major, minor, patch))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub name: String,
    pub notes: String,
    pub page: String,
    pub prerelease: bool,
    pub asset_url: String,
    pub asset_size: u64,
    /// Lowercase hex SHA-256 that GitHub reports for the zip, when it does.
    pub sha256: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    // Redirects may only lead to GitHub's own hosts, over HTTPS.
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let allowed = attempt.url().scheme() == "https" && attempt.url().host_str().is_some_and(is_github_host);
        if attempt.previous().len() > 5 || !allowed { attempt.stop() } else { attempt.follow() }
    });
    reqwest::Client::builder()
        .user_agent(format!("GUC-Station/{CURRENT}"))
        .redirect(policy)
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())
}

fn is_github_host(host: &str) -> bool {
    matches!(host, "github.com" | "api.github.com") || host.ends_with(".githubusercontent.com")
}

/// The newest release that's newer than this app, if there is one.
pub async fn check() -> Result<Option<Release>, String> {
    let current = Version::parse(CURRENT).unwrap_or(Version(0, 0, 0));
    newest(&releases_json().await?, current)
}

async fn releases_json() -> Result<Vec<u8>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
    let resp = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    let status = resp.status();
    let body = resp.bytes().await.map_err(|e| e.to_string())?;
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Err("GitHub is limiting how often this computer can check. Try again in an hour.".into());
    }
    if !status.is_success() {
        return Err(format!("GitHub answered with an error ({status})."));
    }
    Ok(body.to_vec())
}

fn newest(json: &[u8], current: Version) -> Result<Option<Release>, String> {
    let list: Vec<serde_json::Value> = serde_json::from_slice(json).map_err(|e| format!("GitHub sent something unexpected: {e}"))?;
    let releases = list.iter().filter(|r| !r["draft"].as_bool().unwrap_or(false)).filter_map(|r| {
        let version = Version::parse(r["tag_name"].as_str()?)?;
        let assets = r["assets"].as_array()?;
        let zip = |a: &&serde_json::Value| a["name"].as_str().is_some_and(|n| n.to_lowercase().ends_with(".zip"));
        // The Windows zip by name, or the only zip there is.
        let asset = assets
            .iter()
            .filter(zip)
            .find(|a| a["name"].as_str().is_some_and(|n| n.to_lowercase().contains("windows")))
            .or_else(|| assets.iter().filter(zip).next())?;
        Some(Release {
            version,
            name: r["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(r["tag_name"].as_str()?).to_string(),
            notes: r["body"].as_str().unwrap_or_default().trim().to_string(),
            page: r["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
            prerelease: r["prerelease"].as_bool().unwrap_or(false),
            asset_url: asset["browser_download_url"].as_str()?.to_string(),
            asset_size: asset["size"].as_u64().unwrap_or(0),
            sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase),
        })
    });
    Ok(releases.filter(|r| r.version > current).max_by_key(|r| r.version))
}

/// Downloads the release zip, checks it, and returns the new exe inside it.
pub async fn download(release: &Release) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(&release.asset_url).map_err(|e| e.to_string())?;
    if url.scheme() != "https" || !url.host_str().is_some_and(is_github_host) {
        return Err("The update isn't hosted on GitHub, so it wasn't downloaded.".into());
    }
    if release.asset_size > MAX_DOWNLOAD {
        return Err("The update is far bigger than expected, so it wasn't downloaded.".into());
    }
    let resp = client()?.get(url).send().await.map_err(|e| format!("Download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download failed ({}).", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("Download failed: {e}"))?;
    verify(&bytes, release)?;
    exe_from_zip(&bytes)
}

fn verify(bytes: &[u8], release: &Release) -> Result<(), String> {
    if release.asset_size > 0 && bytes.len() as u64 != release.asset_size {
        return Err("The download was incomplete. Try again.".into());
    }
    if let Some(expected) = &release.sha256 {
        let actual: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        if &actual != expected {
            return Err("The download doesn't match the release's checksum, so it wasn't installed.".into());
        }
    }
    Ok(())
}

/// Only the app's exe is taken from the zip; other entries are ignored.
fn exe_from_zip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("The update isn't a readable zip: {e}"))?;
    let index = (0..archive.len())
        .find(|&i| archive.by_index(i).is_ok_and(|f| f.name().replace('\\', "/").rsplit('/').next() == Some(EXE_NAME)))
        .ok_or_else(|| format!("The update zip has no {EXE_NAME} in it."))?;
    let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
    if file.size() > MAX_DOWNLOAD {
        return Err("The app inside the update is far bigger than expected.".into());
    }
    let mut exe = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut exe).map_err(|e| format!("Couldn't unpack the update: {e}"))?;
    if !exe.starts_with(b"MZ") {
        return Err("The update doesn't contain a Windows program.".into());
    }
    Ok(exe)
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Swaps in the new exe. Windows won't overwrite a running program but will
/// rename it, so the current exe becomes `.old` (removed on the next start)
/// and the new one takes its name. On failure the old exe is put back.
pub fn install(exe: &Path, new_exe: &[u8]) -> std::io::Result<()> {
    let staged = sibling(exe, ".new");
    let old = sibling(exe, ".old");
    std::fs::write(&staged, new_exe)?;
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    Ok(())
}

/// Removes what the last update left behind.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(sibling(&exe, ".old"));
        let _ = std::fs::remove_file(sibling(&exe, ".new"));
    }
}

/// Downloads, checks and installs a release over the running app, returning
/// the exe to start. Development builds only download, to keep `target/` intact.
pub async fn update(release: Release) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new_exe = download(&release).await?;
    if cfg!(debug_assertions) {
        return Err("This is a development build, so it wasn't replaced. The download and its checks worked.".into());
    }
    tokio::task::spawn_blocking(move || install(&exe, &new_exe).map(|()| exe))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Couldn't replace the app in its folder ({e}). Download the new version from GitHub instead."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_version_tags() {
        assert_eq!(Version::parse("0.1.0"), Some(Version(0, 1, 0)));
        assert_eq!(Version::parse("v1.2.3-beta"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("V2.0"), Some(Version(2, 0, 0)));
        for bad in ["", "latest", "1", "1.2.3.4", "v1.x.0"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version::parse(CURRENT).is_some(), "Cargo.toml's version must be readable");
    }

    fn release_json(tag: &str, draft: bool, asset: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","name":"GUC Station {tag}","draft":{draft},"prerelease":true,"body":"Notes","html_url":"https://github.com/{REPO}/releases/tag/{tag}",
            "assets":[{{"name":"notes.txt","size":1,"browser_download_url":"https://github.com/x/notes.txt"}},
                      {{"name":"{asset}","size":42,"digest":"sha256:ABC","browser_download_url":"https://github.com/{REPO}/releases/download/{tag}/{asset}"}}]}}"#
        )
    }

    #[test]
    fn picks_the_newest_newer_release() {
        let json = format!(
            "[{},{},{},{}]",
            release_json("0.3.0", true, "GUC-Station-0.3.0-windows-x64.zip"),
            release_json("v0.2.0", false, "GUC-Station-0.2.0-windows-x64.zip"),
            release_json("0.1.5", false, "GUC-Station-0.1.5-windows-x64.zip"),
            release_json("nightly", false, "GUC-Station-windows-x64.zip"),
        );
        let r = newest(json.as_bytes(), Version(0, 1, 0)).unwrap().unwrap();
        assert_eq!(r.version, Version(0, 2, 0), "drafts and unversioned tags are skipped");
        assert_eq!(r.asset_size, 42);
        assert_eq!(r.sha256.as_deref(), Some("abc"));
        assert!(r.asset_url.ends_with("GUC-Station-0.2.0-windows-x64.zip"));
        assert_eq!(newest(json.as_bytes(), Version(0, 2, 0)).unwrap(), None, "up to date");
        assert_eq!(newest(b"[]", Version(0, 1, 0)).unwrap(), None);
        assert!(newest(b"{\"message\":\"Not Found\"}", Version(0, 1, 0)).is_err());
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn takes_only_the_exe_from_the_zip() {
        let zip = zip_with(&[("GUC Station/README.txt", b"hi"), ("GUC Station/GUC Station.exe", b"MZ new app")]);
        assert_eq!(exe_from_zip(&zip).unwrap(), b"MZ new app");
        assert!(exe_from_zip(&zip_with(&[("GUC Station/README.txt", b"hi")])).is_err());
        assert!(exe_from_zip(&zip_with(&[("GUC Station.exe", b"not a program")])).is_err());
        assert!(exe_from_zip(b"not a zip").is_err());
    }

    #[test]
    fn checks_size_and_checksum() {
        let data = b"zip bytes";
        let sum: String = Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
        let mut r = Release {
            version: Version(9, 9, 9),
            name: String::new(),
            notes: String::new(),
            page: String::new(),
            prerelease: false,
            asset_url: String::new(),
            asset_size: data.len() as u64,
            sha256: Some(sum),
        };
        assert!(verify(data, &r).is_ok());
        assert!(verify(b"zip byteX", &r).is_err(), "checksum");
        assert!(verify(b"zip", &r).is_err(), "size");
        r.sha256 = None;
        assert!(verify(data, &r).is_ok());
    }

    /// Talks to the real GitHub repository: `cargo test -- --ignored live_github`.
    /// Treats this app as version 0.0.1, downloads the newest release, checks
    /// it, and installs it over a copy in a temporary folder.
    #[tokio::test]
    #[ignore]
    async fn live_github() {
        let json = releases_json().await.unwrap();
        let release = newest(&json, Version(0, 0, 1)).unwrap().expect("the repository has a release with a zip");
        println!("newest: {} ({} bytes, sha256 {:?}, pre-release {})", release.version, release.asset_size, release.sha256, release.prerelease);
        let exe = download(&release).await.unwrap();
        println!("exe inside: {} bytes", exe.len());
        let dir = std::env::temp_dir().join(format!("guc-station-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join(EXE_NAME);
        std::fs::write(&target, b"MZ old").unwrap();
        install(&target, &exe).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), exe);
        std::fs::remove_dir_all(&dir).unwrap();
        println!("check as {CURRENT}: {:?}", check().await.map(|r| r.map(|r| r.version)));
    }

    #[test]
    fn swaps_the_exe_and_keeps_the_old_one_aside() {
        let dir = std::env::temp_dir().join(format!("guc-station-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(EXE_NAME);
        std::fs::write(&exe, b"MZ old").unwrap();
        install(&exe, b"MZ new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ new");
        assert_eq!(std::fs::read(sibling(&exe, ".old")).unwrap(), b"MZ old");
        assert!(!sibling(&exe, ".new").exists());
        // A second update replaces the leftover .old.
        install(&exe, b"MZ newer").unwrap();
        assert_eq!(std::fs::read(sibling(&exe, ".old")).unwrap(), b"MZ new");
        // A missing exe fails without leaving a staged copy behind.
        assert!(install(&dir.join("missing.exe"), b"MZ").is_err());
        assert!(!dir.join("missing.exe.new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
