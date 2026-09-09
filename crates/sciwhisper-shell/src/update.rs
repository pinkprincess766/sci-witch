//! Update discovery, SemVer comparison and checksum-verified download.
//!
//! This module only reads: it finds whether a newer preview build exists and
//! downloads it into a caller-chosen directory. It never unpacks anything,
//! never touches the running installation and never launches anything.
//! Replacing a running build needs a separate helper process on Windows
//! (the OS locks a running `.exe`), which is deliberately out of scope here
//! — see `docs/development/AUTO_UPDATE_RU.md`.
//!
//! What the SHA-256 check does and does not prove: it establishes
//! **integrity** of the archive relative to the manifest that named it —
//! a truncated, corrupted or swapped-in-transit body is caught. It is
//! **not** a signature and proves nothing about authenticity: the manifest
//! and the archive come from the same GitHub Release, so anyone able to
//! rewrite that release can rewrite both consistently. Only a Developer ID
//! / code-signing chain would close that gap.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{Error, Result};

const RELEASES_URL: &str =
    "https://api.github.com/repos/pinkprincess766/sci-witch/releases?per_page=20";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;
/// Only channel that exists today (see AUTO_UPDATE_RU.md); a future "stable"
/// channel needs its own selection logic, not just a different constant.
const EXPECTED_CHANNEL: &str = "preview";
const MANIFEST_ASSET_NAME: &str = "update-manifest.json";
const RELEASE_DOWNLOAD_BASE: &str =
    "https://github.com/pinkprincess766/sci-witch/releases/download/";
const RELEASE_NOTES_BASE: &str = "https://github.com/pinkprincess766/sci-witch/releases/tag/";
const RELEASES_MAX_BYTES: u64 = 2 * 1024 * 1024;
const MANIFEST_MAX_BYTES: u64 = 256 * 1024;
/// Published archives are ~3.5 MiB today, so this leaves two orders of
/// magnitude of headroom while still bounding how much a compromised or
/// misbehaving CDN can write to disk. Bundling a Whisper model later would
/// mean raising this deliberately, not silently outgrowing it: the failure
/// is a clear error, never a truncated archive.
const ARCHIVE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const USER_AGENT: &str = "sci-witch-updater";
const MAX_REDIRECTS: u32 = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ManifestPlatform {
    pub asset_name: String,
    pub sha256: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    version: String,
    channel: String,
    notes_url: String,
    platforms: HashMap<String, ManifestPlatform>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes_url: String,
    pub platform: ManifestPlatform,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone)]
struct UpdateSource {
    releases_url: Url,
    release_download_base: Url,
    release_notes_base: Url,
    platform_key: &'static str,
}

impl UpdateSource {
    fn production(platform_key: &'static str) -> Result<Self> {
        Self::new(
            RELEASES_URL,
            RELEASE_DOWNLOAD_BASE,
            RELEASE_NOTES_BASE,
            platform_key,
        )
    }

    fn new(
        releases_url: &str,
        release_download_base: &str,
        release_notes_base: &str,
        platform_key: &'static str,
    ) -> Result<Self> {
        Ok(Self {
            releases_url: parse_request_url(releases_url)?,
            release_download_base: parse_base_url(release_download_base)?,
            release_notes_base: parse_base_url(release_notes_base)?,
            platform_key,
        })
    }

    fn release_asset_url(&self, tag: &str, asset_name: &str) -> Result<Url> {
        require_safe_component(tag, "release tag")?;
        require_safe_filename(asset_name)?;
        let relative = format!("{tag}/{asset_name}");
        self.release_download_base
            .join(&relative)
            .map_err(|e| Error::Message(format!("invalid expected release asset URL: {e}")))
    }

    fn notes_url(&self, tag: &str) -> Result<Url> {
        require_safe_component(tag, "release tag")?;
        self.release_notes_base
            .join(tag)
            .map_err(|e| Error::Message(format!("invalid expected release notes URL: {e}")))
    }
}

/// Returns the release platform identifier for supported production targets.
/// Linux and unshipped CPU architectures are rejected instead of being
/// silently treated as macOS ARM.
pub fn current_platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        _ => None,
    }
}

/// Checks the public releases list for the highest-versioned preview release
/// whose manifest is readable and self-consistent (tag, SemVer, notes URL,
/// asset name and asset URL all agree), matches this build's schema, channel
/// and platform, and is strictly newer than `current_version`.
///
/// `Ok(None)` covers "no update", "already on the latest" and "the newest
/// release cannot be trusted": those abstain identically, on purpose.
/// `Err` is reserved for the caller's own environment — a platform this
/// project never publishes for, or an unusable current version string.
pub fn check_for_update(current_version: &str) -> Result<Option<UpdateInfo>> {
    let platform_key = current_platform_key().ok_or_else(|| {
        Error::Message(format!(
            "updates are not published for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let source = UpdateSource::production(platform_key)?;
    check_for_update_from(&source, current_version)
}

fn check_for_update_from(
    source: &UpdateSource,
    current_version: &str,
) -> Result<Option<UpdateInfo>> {
    let current = Version::parse(current_version)
        .map_err(|e| Error::Message(format!("invalid current version {current_version:?}: {e}")))?;

    let agent = http_agent();
    let releases: Vec<GhRelease> = get_json_limited(
        &agent,
        &source.releases_url,
        RELEASES_MAX_BYTES,
        "release list",
    )?;

    // The API documents no ordering guarantee for this endpoint, so every
    // entry on the page is considered and the newest one wins — never
    // "whatever came first". A release that is not newer is skipped, not
    // treated as the end of the search.
    let mut candidates: Vec<(Version, GhRelease)> = releases
        .into_iter()
        .filter_map(|release| {
            let version = candidate_version(source, &release, &current)?;
            Some((version, release))
        })
        .collect();
    // Highest first: the manifest is only fetched for releases that could
    // still win, and the tag version provably equals the manifest version
    // for anything that validates, so this order is the final order.
    candidates.sort_by(|a, b| b.0.cmp(&a.0));

    for (version, release) in candidates {
        if let Some(update) = validated_update(&agent, source, &release, &version) {
            return Ok(Some(update));
        }
    }
    Ok(None)
}

/// Cheap pre-filter over the release list alone: a usable candidate is a
/// published prerelease whose tag is exactly `v{SemVer}`, is strictly newer
/// than `current`, and carries a manifest asset at the URL this client
/// would reconstruct. Nothing here performs a request.
fn candidate_version(
    source: &UpdateSource,
    release: &GhRelease,
    current: &Version,
) -> Option<Version> {
    if release.draft || !release.prerelease {
        return None;
    }
    require_safe_component(&release.tag_name, "release tag").ok()?;
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    if release.tag_name != format!("v{version}") || version <= *current {
        return None;
    }
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == MANIFEST_ASSET_NAME)?;
    let expected = source
        .release_asset_url(&release.tag_name, MANIFEST_ASSET_NAME)
        .ok()?;
    url_equals(&asset.browser_download_url, &expected).then_some(version)
}

/// Fetches and fully validates one candidate's manifest. `None` means this
/// release cannot be trusted and the caller should try the next best one.
fn validated_update(
    agent: &ureq::Agent,
    source: &UpdateSource,
    release: &GhRelease,
    version: &Version,
) -> Option<UpdateInfo> {
    let manifest_url = source
        .release_asset_url(&release.tag_name, MANIFEST_ASSET_NAME)
        .ok()?;
    let manifest: Manifest =
        get_json_limited(agent, &manifest_url, MANIFEST_MAX_BYTES, "update manifest").ok()?;
    if manifest.schema_version != SUPPORTED_SCHEMA_VERSION || manifest.channel != EXPECTED_CHANNEL {
        return None;
    }
    let platform = manifest.platforms.get(source.platform_key).cloned()?;
    let remote = Version::parse(&manifest.version).ok()?;
    // The manifest must name the very release it was published in.
    if remote != *version || release.tag_name != format!("v{remote}") {
        return None;
    }
    let expected_asset_name = expected_asset_name(&remote, source.platform_key).ok()?;
    if platform.asset_name != expected_asset_name || !valid_sha256(&platform.sha256) {
        return None;
    }
    let expected_asset_url = source
        .release_asset_url(&release.tag_name, &expected_asset_name)
        .ok()?;
    if !url_equals(&platform.url, &expected_asset_url) {
        return None;
    }
    let expected_notes_url = source.notes_url(&release.tag_name).ok()?;
    if !url_equals(&manifest.notes_url, &expected_notes_url) {
        return None;
    }
    Some(UpdateInfo {
        version: manifest.version,
        notes_url: manifest.notes_url,
        platform,
    })
}

/// Redirects are followed by hand rather than by the agent, so that each
/// hop can be inspected. GitHub answers a release-asset URL with a redirect
/// to its CDN, so refusing redirects outright would break every real
/// download; silently accepting an `https://` → `http://` hop would quietly
/// drop the transport protection instead.
///
/// What keeps this safe overall is that the *initial* URL is never taken
/// from a response — it is rebuilt from the pinned release base plus a tag
/// that must equal `v{SemVer}`, and a manifest whose own URLs disagree with
/// that reconstruction is rejected before any request.
fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .timeout_write(WRITE_TIMEOUT)
        .redirects(0)
        .build()
}

/// A redirect may not change the transport: whatever scheme the pinned
/// initial URL used, every hop has to keep it. That bans the classic
/// `https` → `http` downgrade, and equally any hop to a different scheme
/// altogether.
fn redirect_scheme_allowed(initial_scheme: &str, next: &Url) -> bool {
    next.scheme() == initial_scheme
}

fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn get_following_redirects(agent: &ureq::Agent, url: &Url) -> Result<ureq::Response> {
    let initial_scheme = url.scheme().to_owned();
    let mut current = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        // With following disabled, ureq hands back a 3xx as `Ok`; only 4xx
        // and 5xx arrive as `Err(Status)`. Both shapes are matched so the
        // hop policy does not depend on which one it picks.
        let response = match agent
            .get(current.as_str())
            .set("User-Agent", USER_AGENT)
            .call()
        {
            Ok(response) if is_redirect_status(response.status()) => response,
            Ok(response) => return Ok(response),
            Err(ureq::Error::Status(status, response)) if is_redirect_status(status) => response,
            Err(e) => return Err(Error::Message(format!("request to {current} failed: {e}"))),
        };
        let location = response.header("Location").ok_or_else(|| {
            Error::Message(format!(
                "redirect from {current} carried no Location header"
            ))
        })?;
        let next = current
            .join(location)
            .map_err(|e| Error::Message(format!("invalid redirect target from {current}: {e}")))?;
        if !redirect_scheme_allowed(&initial_scheme, &next) {
            return Err(Error::Message(format!(
                "refusing a {initial_scheme} -> {} redirect to {next}",
                next.scheme()
            )));
        }
        current = next;
    }
    Err(Error::Message(format!(
        "too many redirects starting at {url}"
    )))
}

fn get_json_limited<T: serde::de::DeserializeOwned>(
    agent: &ureq::Agent,
    url: &Url,
    max_bytes: u64,
    label: &str,
) -> Result<T> {
    let response = get_following_redirects(agent, url)?;
    reject_declared_length(&response, max_bytes, label)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::Message(format!(
            "{label} exceeded the {max_bytes}-byte limit while reading"
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| Error::Message(format!("invalid {label} from {url}: {e}")))
}

/// Downloads the archive described by `update` into `dest_dir` and returns
/// its path only after the SHA-256 matched.
///
/// The entry point takes a whole [`UpdateInfo`], never a bare
/// [`ManifestPlatform`], so a caller cannot hand this function a loose
/// platform record it never agreed on. Because those fields are public,
/// every constraint is re-checked here rather than trusted: the asset name
/// must be exactly the one this version publishes, the URL must rebuild to
/// the pinned release URL, and the checksum must be well formed — a
/// hand-assembled `UpdateInfo` pointing somewhere else is rejected before
/// any request is made.
pub fn download_and_verify(update: &UpdateInfo, dest_dir: &Path) -> Result<PathBuf> {
    let platform_key = current_platform_key().ok_or_else(|| {
        Error::Message(format!(
            "updates are not published for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let source = UpdateSource::production(platform_key)?;
    download_and_verify_from(update, dest_dir, &source, ARCHIVE_MAX_BYTES)
}

fn download_and_verify_from(
    update: &UpdateInfo,
    dest_dir: &Path,
    source: &UpdateSource,
    max_bytes: u64,
) -> Result<PathBuf> {
    let version = Version::parse(&update.version)
        .map_err(|e| Error::Message(format!("invalid update version {:?}: {e}", update.version)))?;
    let tag = format!("v{version}");
    let expected_name = expected_asset_name(&version, source.platform_key)?;
    if update.platform.asset_name != expected_name {
        return Err(Error::Message(format!(
            "unexpected update asset name {:?}",
            update.platform.asset_name
        )));
    }
    require_safe_filename(&update.platform.asset_name)?;
    if !valid_sha256(&update.platform.sha256) {
        return Err(Error::Message("invalid update SHA-256".into()));
    }
    let expected_url = source.release_asset_url(&tag, &expected_name)?;
    if !url_equals(&update.platform.url, &expected_url) {
        return Err(Error::Message("update asset URL is not trusted".into()));
    }

    std::fs::create_dir_all(dest_dir)?;
    let agent = http_agent();
    let response = get_following_redirects(&agent, &expected_url)?;
    reject_declared_length(&response, max_bytes, "update archive")?;
    write_verified_archive(
        response.into_reader(),
        dest_dir,
        &expected_name,
        &update.platform.sha256,
        max_bytes,
    )
}

fn write_verified_archive<R: Read>(
    mut reader: R,
    dest_dir: &Path,
    expected_name: &str,
    expected_sha256: &str,
    max_bytes: u64,
) -> Result<PathBuf> {
    require_safe_filename(expected_name)?;
    if !valid_sha256(expected_sha256) {
        return Err(Error::Message("invalid update SHA-256".into()));
    }
    let dest = dest_dir.join(expected_name);
    let mut file = tempfile::Builder::new()
        .prefix(".sci-witch-update-")
        .suffix(".part")
        .tempfile_in(dest_dir)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut downloaded = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        downloaded = downloaded
            .checked_add(n as u64)
            .ok_or_else(|| Error::Message("update archive size overflow".into()))?;
        if downloaded > max_bytes {
            return Err(Error::Message(format!(
                "update archive exceeded the {max_bytes}-byte limit while downloading"
            )));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.as_file_mut().flush()?;
    file.as_file().sync_all()?;

    let actual = hex_encode(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(Error::Message(format!(
            "downloaded archive checksum mismatch: manifest says {}, got {actual}",
            expected_sha256
        )));
    }
    file.persist_noclobber(&dest)
        .map_err(|e| Error::Io(e.error))?;
    Ok(dest)
}

/// A URL that is requested as-is (no joining), so it needs no trailing
/// slash — but still no credentials and no opaque form.
fn parse_request_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)
        .map_err(|e| Error::Message(format!("invalid update request URL {value:?}: {e}")))?;
    if url.cannot_be_a_base() || !url.username().is_empty() || url.password().is_some() {
        return Err(Error::Message(format!(
            "update request URL is not usable: {value:?}"
        )));
    }
    Ok(url)
}

fn parse_base_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)
        .map_err(|e| Error::Message(format!("invalid update source URL {value:?}: {e}")))?;
    if url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
    {
        return Err(Error::Message(format!(
            "update source URL is not a clean base URL: {value:?}"
        )));
    }
    Ok(url)
}

fn require_safe_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(Error::Message(format!("unsafe {label}: {value:?}")));
    }
    Ok(())
}

fn require_safe_filename(value: &str) -> Result<()> {
    require_safe_component(value, "update asset name")?;
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(Error::Message(format!(
            "unsafe update asset name: {value:?}"
        )));
    }
    Ok(())
}

fn expected_asset_name(version: &Version, platform_key: &str) -> Result<String> {
    let suffix = match platform_key {
        "windows-x64" => "Windows-x64",
        "macos-arm64" => "macOS-arm64",
        other => {
            return Err(Error::Message(format!(
                "unsupported update platform {other:?}"
            )))
        }
    };
    Ok(format!("SciWhisper-{version}-{suffix}.zip"))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn url_equals(actual: &str, expected: &Url) -> bool {
    Url::parse(actual).is_ok_and(|actual| actual == *expected)
}

/// Size the server claims up front. Absent or unparsable for a chunked
/// response, which is why the streaming loop enforces its own cap too.
fn declared_length(response: &ureq::Response) -> Option<u64> {
    response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
}

/// Rejects a response whose declared size is already over the limit, before
/// a single body byte is read. The message differs from the streaming
/// guard's on purpose: the two are separate protections and a test should
/// be able to tell which one fired.
fn reject_declared_length(response: &ureq::Response, max_bytes: u64, label: &str) -> Result<()> {
    match declared_length(response) {
        Some(declared) if declared > max_bytes => Err(Error::Message(format!(
            "{label} declares {declared} bytes, over the {max_bytes}-byte limit"
        ))),
        _ => Ok(()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;
    use std::io::{Cursor, ErrorKind};
    use std::sync::Arc;
    use std::thread;
    use tiny_http::{Response, Server};

    const TEST_PLATFORM_KEY: &str = "windows-x64";

    /// Binds a local server and reports its base URL immediately, before any
    /// request is served — so routes can embed self-referential URLs (e.g. a
    /// manifest pointing at an asset on the same server).
    fn bind() -> (Arc<Server>, String) {
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind local test server"));
        let base = format!("http://{}", server.server_addr());
        (server, base)
    }

    fn test_source(base: &str) -> UpdateSource {
        UpdateSource::new(
            &format!("{base}/releases"),
            &format!("{base}/download/"),
            &format!("{base}/tag/"),
            TEST_PLATFORM_KEY,
        )
        .unwrap()
    }

    /// Serves `routes` (path -> (content-type, body)) on `server` for at
    /// most `request_count` requests, returning the number actually served.
    /// The wait is bounded: a test that makes *fewer* requests than expected
    /// must fail its assertion, never hang the suite.
    fn serve(
        server: Arc<Server>,
        routes: Map<String, (&'static str, Vec<u8>)>,
        request_count: usize,
    ) -> thread::JoinHandle<usize> {
        thread::spawn(move || {
            let mut served = 0;
            for _ in 0..request_count {
                let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(5)) else {
                    break;
                };
                served += 1;
                let url = request.url().to_string();
                match routes.get(url.as_str()) {
                    Some((content_type, body)) => {
                        let response = Response::from_data(body.clone()).with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                content_type.as_bytes(),
                            )
                            .unwrap(),
                        );
                        let _ = request.respond(response);
                    }
                    None => {
                        let _ = request
                            .respond(Response::from_string("not found").with_status_code(404));
                    }
                }
            }
            served
        })
    }

    fn json_route(value: serde_json::Value) -> (&'static str, Vec<u8>) {
        ("application/json", serde_json::to_vec(&value).unwrap())
    }

    fn release_entry(
        tag: &str,
        prerelease: bool,
        draft: bool,
        manifest_url: Option<&str>,
    ) -> serde_json::Value {
        let assets = match manifest_url {
            Some(url) => {
                serde_json::json!([{"name": "update-manifest.json", "browser_download_url": url}])
            }
            None => serde_json::json!([]),
        };
        serde_json::json!({
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "assets": assets,
        })
    }

    fn manifest_json(
        source: &UpdateSource,
        version: &str,
        channel: &str,
        schema_version: u32,
        platform_key: &str,
        sha256: &str,
    ) -> serde_json::Value {
        let version = Version::parse(version).unwrap();
        let tag = format!("v{version}");
        let asset_name = expected_asset_name(&version, TEST_PLATFORM_KEY).unwrap();
        let asset_url = source
            .release_asset_url(&tag, &asset_name)
            .unwrap()
            .to_string();
        let notes_url = source.notes_url(&tag).unwrap().to_string();
        serde_json::json!({
            "schema_version": schema_version,
            "version": version.to_string(),
            "channel": channel,
            "published_at": "2026-09-01T00:00:00Z",
            "notes_url": notes_url,
            "platforms": {
                platform_key: {
                    "asset_name": asset_name,
                    "sha256": sha256,
                    "url": asset_url,
                }
            }
        })
    }

    fn add_manifest_route(
        routes: &mut Map<String, (&'static str, Vec<u8>)>,
        source: &UpdateSource,
        version: &str,
        channel: &str,
        schema_version: u32,
        platform_key: &str,
        sha256: &str,
    ) -> String {
        let tag = format!("v{version}");
        let manifest_url = source.release_asset_url(&tag, MANIFEST_ASSET_NAME).unwrap();
        routes.insert(
            manifest_url.path().to_owned(),
            json_route(manifest_json(
                source,
                version,
                channel,
                schema_version,
                platform_key,
                sha256,
            )),
        );
        manifest_url.to_string()
    }

    fn valid_update(source: &UpdateSource, version: &str, sha256: String) -> UpdateInfo {
        let version = Version::parse(version).unwrap();
        let tag = format!("v{version}");
        let asset_name = expected_asset_name(&version, source.platform_key).unwrap();
        UpdateInfo {
            version: version.to_string(),
            notes_url: source.notes_url(&tag).unwrap().to_string(),
            platform: ManifestPlatform {
                url: source
                    .release_asset_url(&tag, &asset_name)
                    .unwrap()
                    .to_string(),
                asset_name,
                sha256,
            },
        }
    }

    fn assert_dir_empty(path: &Path) {
        assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
    }

    #[test]
    fn finds_update_when_remote_is_newer() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "0.2.0",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v0.2.0",
                true,
                false,
                Some(&manifest_url),
            )])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();
        assert_eq!(result.unwrap().version, "0.2.0");
    }

    #[test]
    fn no_update_when_remote_is_older_downgrade_protection() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "0.1.0",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v0.1.0",
                true,
                false,
                Some(&manifest_url),
            )])),
        );
        let handle = serve(server, routes, 1);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        let served = handle.join().unwrap();
        assert_eq!(
            result, None,
            "0.1.0 must not be offered as an update over 0.1.1-rc1"
        );
        // The tag alone settles it, so the manifest is never fetched.
        assert_eq!(served, 1, "only the release list should be requested");
    }

    #[test]
    fn no_update_when_remote_equals_current() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "0.1.1-rc1",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v0.1.1-rc1",
                true,
                false,
                Some(&manifest_url),
            )])),
        );
        let handle = serve(server, routes, 1);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        let served = handle.join().unwrap();
        assert_eq!(result, None);
        // Reinstalling the same version is settled by the tag as well.
        assert_eq!(served, 1, "only the release list should be requested");
    }

    #[test]
    fn skips_release_without_manifest_asset_and_uses_next() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "9.9.9",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![
                release_entry("v10.0.0", true, false, None),
                release_entry("v9.9.9", true, false, Some(&manifest_url)),
            ])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();
        assert_eq!(result.unwrap().version, "9.9.9");
    }

    #[test]
    fn unsupported_schema_version_is_skipped() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "9.9.9",
            "preview",
            2,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v9.9.9",
                true,
                false,
                Some(&manifest_url),
            )])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn wrong_channel_is_skipped() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "9.9.9",
            "stable",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v9.9.9",
                true,
                false,
                Some(&manifest_url),
            )])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn missing_platform_entry_is_skipped() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "9.9.9",
            "preview",
            1,
            "some-other-platform",
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v9.9.9",
                true,
                false,
                Some(&manifest_url),
            )])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn draft_and_non_prerelease_entries_are_ignored() {
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let manifest_url = add_manifest_route(
            &mut routes,
            &source,
            "9.9.9",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![
                release_entry("v9.9.9", false, true, Some(&manifest_url)),
                release_entry("v9.9.9", false, false, Some(&manifest_url)),
                release_entry("v9.9.9", true, false, Some(&manifest_url)),
            ])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();
        assert_eq!(result.unwrap().version, "9.9.9");
    }

    #[test]
    fn production_policy_rejects_a_local_manifest_url_without_requesting_it() {
        let (server, base) = bind();
        let source = UpdateSource::new(
            &format!("{base}/releases"),
            RELEASE_DOWNLOAD_BASE,
            RELEASE_NOTES_BASE,
            TEST_PLATFORM_KEY,
        )
        .unwrap();
        let mut routes = Map::new();
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                "v9.9.9",
                true,
                false,
                Some("http://127.0.0.1:9/private-manifest"),
            )])),
        );
        let handle = serve(server, routes, 1);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn manifest_version_must_match_its_release_tag() {
        let (server, base) = bind();
        let source = test_source(&base);
        let release_tag = "v9.9.8";
        let manifest_url = source
            .release_asset_url(release_tag, MANIFEST_ASSET_NAME)
            .unwrap();
        let mut routes = Map::new();
        routes.insert(
            manifest_url.path().to_owned(),
            json_route(manifest_json(
                &source,
                "9.9.9",
                "preview",
                1,
                TEST_PLATFORM_KEY,
                &"0".repeat(64),
            )),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                release_tag,
                true,
                false,
                Some(manifest_url.as_str()),
            )])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn manifest_asset_url_must_match_the_versioned_release_asset() {
        let (server, base) = bind();
        let source = test_source(&base);
        let tag = "v9.9.9";
        let manifest_url = source.release_asset_url(tag, MANIFEST_ASSET_NAME).unwrap();
        let mut manifest = manifest_json(
            &source,
            "9.9.9",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        manifest["platforms"][TEST_PLATFORM_KEY]["url"] =
            serde_json::Value::String(format!("{base}/private.zip"));
        let mut routes = Map::new();
        routes.insert(manifest_url.path().to_owned(), json_route(manifest));
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![release_entry(
                tag,
                true,
                false,
                Some(manifest_url.as_str()),
            )])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn oversized_json_is_rejected_at_the_reader_boundary() {
        let (server, base) = bind();
        let mut routes = Map::new();
        routes.insert("/large".into(), ("application/json", vec![b' '; 32]));
        let handle = serve(server, routes, 1);

        let result = get_json_limited::<serde_json::Value>(
            &http_agent(),
            &Url::parse(&format!("{base}/large")).unwrap(),
            8,
            "test JSON",
        );
        handle.join().unwrap();

        assert!(result.is_err());
    }

    #[test]
    fn unsupported_platform_name_is_not_mapped_to_macos() {
        assert!(expected_asset_name(&Version::new(1, 0, 0), "linux-x64").is_err());
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => assert_eq!(current_platform_key(), Some("windows-x64")),
            ("macos", "aarch64") => assert_eq!(current_platform_key(), Some("macos-arm64")),
            _ => assert_eq!(current_platform_key(), None),
        }
    }

    #[test]
    fn download_and_verify_succeeds_with_correct_checksum() {
        let (server, base) = bind();
        let source = test_source(&base);
        let body = b"pretend this is a zip archive".to_vec();
        let sha = hex_encode(&Sha256::digest(&body));
        let update = valid_update(&source, "0.2.0", sha);
        let asset_url = Url::parse(&update.platform.url).unwrap();
        let mut routes = Map::new();
        routes.insert(
            asset_url.path().to_owned(),
            ("application/octet-stream", body.clone()),
        );
        let handle = serve(server, routes, 1);

        let dir = tempfile::tempdir().unwrap();
        let path = download_and_verify_from(&update, dir.path(), &source, 1024).unwrap();
        handle.join().unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), body);
    }

    #[test]
    fn download_and_verify_rejects_checksum_mismatch_and_removes_file() {
        let (server, base) = bind();
        let source = test_source(&base);
        let body = b"substituted or corrupted content".to_vec();
        let update = valid_update(&source, "0.2.0", "0".repeat(64));
        let asset_url = Url::parse(&update.platform.url).unwrap();
        let mut routes = Map::new();
        routes.insert(
            asset_url.path().to_owned(),
            ("application/octet-stream", body),
        );
        let handle = serve(server, routes, 1);

        let dir = tempfile::tempdir().unwrap();
        let result = download_and_verify_from(&update, dir.path(), &source, 1024);
        handle.join().unwrap();

        assert!(result.is_err());
        assert_dir_empty(dir.path());
    }

    #[test]
    fn download_rejects_path_traversal_before_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let sibling = dir.path().parent().unwrap().join("victim.txt");
        std::fs::write(&sibling, b"keep me").unwrap();
        let source = UpdateSource::production(TEST_PLATFORM_KEY).unwrap();
        let mut update = valid_update(&source, "0.2.0", "0".repeat(64));
        update.platform.asset_name = "../victim.txt".into();

        let result = download_and_verify_from(&update, dir.path(), &source, 1024);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&sibling).unwrap(), b"keep me");
        assert_dir_empty(dir.path());
        std::fs::remove_file(sibling).unwrap();
    }

    #[test]
    fn download_rejects_untrusted_asset_url_before_requesting_it() {
        let source = UpdateSource::production(TEST_PLATFORM_KEY).unwrap();
        let mut update = valid_update(&source, "0.2.0", "0".repeat(64));
        update.platform.url = "http://127.0.0.1:9/private.zip".into();
        let dir = tempfile::tempdir().unwrap();

        let result = download_and_verify_from(&update, dir.path(), &source, 1024);

        assert!(result.is_err());
        assert_dir_empty(dir.path());
    }

    #[test]
    fn newest_release_wins_regardless_of_list_order() {
        // The API promises no ordering, so the newest valid release must be
        // chosen even when it is listed last.
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let mut entries = Vec::new();
        for version in ["0.1.5", "0.3.0", "0.2.0"] {
            let manifest_url = add_manifest_route(
                &mut routes,
                &source,
                version,
                "preview",
                1,
                TEST_PLATFORM_KEY,
                &"0".repeat(64),
            );
            entries.push(release_entry(
                &format!("v{version}"),
                true,
                false,
                Some(&manifest_url),
            ));
        }
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(entries)),
        );
        // Release list plus exactly one manifest: only the winning candidate
        // is fetched, the two losers are ruled out by their tags alone.
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        let served = handle.join().unwrap();

        assert_eq!(result.unwrap().version, "0.3.0");
        assert_eq!(served, 2);
    }

    #[test]
    fn an_older_release_listed_first_does_not_end_the_search() {
        // The defect this guards: stopping at the first entry that is not
        // newer would hide the 0.2.0 release listed behind it.
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let old_manifest = add_manifest_route(
            &mut routes,
            &source,
            "0.1.0",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        let new_manifest = add_manifest_route(
            &mut routes,
            &source,
            "0.2.0",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![
                release_entry("v0.1.0", true, false, Some(&old_manifest)),
                release_entry("v0.2.0", true, false, Some(&new_manifest)),
            ])),
        );
        let handle = serve(server, routes, 2);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();

        assert_eq!(result.unwrap().version, "0.2.0");
    }

    #[test]
    fn a_broken_newest_manifest_falls_back_to_the_next_best_release() {
        // "Maximum valid SemVer" means valid: the higher tag loses if its
        // manifest does not validate.
        let (server, base) = bind();
        let source = test_source(&base);
        let mut routes = Map::new();
        let broken = add_manifest_route(
            &mut routes,
            &source,
            "0.3.0",
            "stable", // wrong channel
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        let good = add_manifest_route(
            &mut routes,
            &source,
            "0.2.0",
            "preview",
            1,
            TEST_PLATFORM_KEY,
            &"0".repeat(64),
        );
        routes.insert(
            "/releases".into(),
            json_route(serde_json::Value::Array(vec![
                release_entry("v0.2.0", true, false, Some(&good)),
                release_entry("v0.3.0", true, false, Some(&broken)),
            ])),
        );
        let handle = serve(server, routes, 3);

        let result = check_for_update_from(&source, "0.1.1-rc1").unwrap();
        handle.join().unwrap();

        assert_eq!(result.unwrap().version, "0.2.0");
    }

    #[test]
    fn redirects_are_followed_within_the_same_scheme() {
        let (server, base) = bind();
        let target = format!("{base}/moved");
        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(5)) else {
                    return;
                };
                if request.url() == "/start" {
                    let response = Response::from_string("").with_status_code(302).with_header(
                        tiny_http::Header::from_bytes(&b"Location"[..], b"/moved").unwrap(),
                    );
                    let _ = request.respond(response);
                } else {
                    let _ = request.respond(Response::from_string("{\"ok\":true}"));
                }
            }
        });

        let value: serde_json::Value = get_json_limited(
            &http_agent(),
            &Url::parse(&format!("{base}/start")).unwrap(),
            1024,
            "test JSON",
        )
        .unwrap();
        let _ = handle.join();

        assert_eq!(value["ok"], serde_json::Value::Bool(true));
        assert!(target.ends_with("/moved"));
    }

    #[test]
    fn a_scheme_downgrade_redirect_is_refused() {
        // The policy predicate, checked directly: a TLS test server is out
        // of reach here, so the https -> http hop is proven at this level
        // rather than over a real socket.
        assert!(redirect_scheme_allowed(
            "https",
            &Url::parse("https://objects.githubusercontent.com/x").unwrap()
        ));
        assert!(!redirect_scheme_allowed(
            "https",
            &Url::parse("http://objects.githubusercontent.com/x").unwrap()
        ));
        assert!(!redirect_scheme_allowed(
            "https",
            &Url::parse("file:///etc/passwd").unwrap()
        ));
        assert!(redirect_scheme_allowed(
            "http",
            &Url::parse("http://127.0.0.1:1/x").unwrap()
        ));
    }

    #[test]
    fn a_redirect_without_a_location_header_is_an_error() {
        let (server, base) = bind();
        let handle = thread::spawn(move || {
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(5)) {
                let _ = request.respond(Response::from_string("").with_status_code(302));
            }
        });

        let error = get_json_limited::<serde_json::Value>(
            &http_agent(),
            &Url::parse(&format!("{base}/start")).unwrap(),
            1024,
            "test JSON",
        )
        .unwrap_err()
        .to_string();
        let _ = handle.join();

        assert!(error.contains("no Location header"), "{error}");
    }

    #[test]
    fn a_redirect_loop_stops_at_the_hop_limit() {
        let (server, base) = bind();
        let handle = thread::spawn(move || {
            while let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(2)) {
                let response = Response::from_string("").with_status_code(302).with_header(
                    tiny_http::Header::from_bytes(&b"Location"[..], b"/loop").unwrap(),
                );
                let _ = request.respond(response);
            }
        });

        let error = get_json_limited::<serde_json::Value>(
            &http_agent(),
            &Url::parse(&format!("{base}/loop")).unwrap(),
            1024,
            "test JSON",
        )
        .unwrap_err()
        .to_string();
        let _ = handle.join();

        assert!(error.contains("too many redirects"), "{error}");
    }

    #[test]
    fn declared_content_length_over_the_limit_is_rejected_before_streaming() {
        let (server, base) = bind();
        let source = test_source(&base);
        // `Response::from_data` sends a Content-Length, so the declared-size
        // guard must fire before any body byte is read.
        let body = vec![7u8; 32];
        let update = valid_update(&source, "0.2.0", hex_encode(&Sha256::digest(&body)));
        let asset_url = Url::parse(&update.platform.url).unwrap();
        let mut routes = Map::new();
        routes.insert(
            asset_url.path().to_owned(),
            ("application/octet-stream", body),
        );
        let handle = serve(server, routes, 1);
        let dir = tempfile::tempdir().unwrap();

        let error = download_and_verify_from(&update, dir.path(), &source, 8)
            .unwrap_err()
            .to_string();
        handle.join().unwrap();

        assert!(error.contains("declares 32 bytes"), "{error}");
        assert_dir_empty(dir.path());
    }

    #[test]
    fn stream_limit_is_enforced_over_http_without_content_length() {
        // A chunked response declares no size at all, so only the streaming
        // counter can stop it — and it must still leave nothing behind.
        let (server, base) = bind();
        let source = test_source(&base);
        let body = vec![7u8; 4096];
        let update = valid_update(&source, "0.2.0", hex_encode(&Sha256::digest(&body)));
        let asset_path = Url::parse(&update.platform.url).unwrap().path().to_owned();
        let handle = thread::spawn(move || {
            let Ok(request) = server.recv() else {
                return;
            };
            assert_eq!(request.url(), asset_path);
            // `data_length: None` makes tiny_http answer chunked, with no
            // Content-Length header for the client to trust.
            let response = Response::new(
                tiny_http::StatusCode(200),
                Vec::new(),
                Cursor::new(body),
                None,
                None,
            );
            let _ = request.respond(response);
        });
        let dir = tempfile::tempdir().unwrap();

        let error = download_and_verify_from(&update, dir.path(), &source, 64)
            .unwrap_err()
            .to_string();
        let _ = handle.join();

        assert!(error.contains("while downloading"), "{error}");
        assert_dir_empty(dir.path());
    }

    struct InterruptedReader {
        emitted: bool,
    }

    impl Read for InterruptedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.emitted {
                return Err(std::io::Error::new(
                    ErrorKind::ConnectionReset,
                    "interrupted",
                ));
            }
            self.emitted = true;
            let bytes = b"partial archive";
            buf[..bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }
    }

    #[test]
    fn interrupted_download_leaves_no_partial_archive() {
        let dir = tempfile::tempdir().unwrap();

        let result = write_verified_archive(
            InterruptedReader { emitted: false },
            dir.path(),
            "SciWhisper-0.2.0-Windows-x64.zip",
            &"0".repeat(64),
            1024,
        );

        assert!(result.is_err());
        assert_dir_empty(dir.path());
    }

    #[test]
    fn stream_limit_is_enforced_without_a_content_length_header() {
        let dir = tempfile::tempdir().unwrap();
        let body = vec![1u8; 17];

        let result = write_verified_archive(
            Cursor::new(body),
            dir.path(),
            "SciWhisper-0.2.0-Windows-x64.zip",
            &"0".repeat(64),
            16,
        );

        assert!(result.is_err());
        assert_dir_empty(dir.path());
    }

    #[test]
    fn existing_verified_filename_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let name = "SciWhisper-0.2.0-Windows-x64.zip";
        let dest = dir.path().join(name);
        std::fs::write(&dest, b"existing").unwrap();
        let body = b"new archive";

        let result = write_verified_archive(
            Cursor::new(body),
            dir.path(),
            name,
            &hex_encode(&Sha256::digest(body)),
            1024,
        );

        assert!(result.is_err());
        assert_eq!(std::fs::read(dest).unwrap(), b"existing");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
