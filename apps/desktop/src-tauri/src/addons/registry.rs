use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use tauri::Emitter;

const URL: &str = "https://github.com/Just8it/Goodtype-Addons/releases/latest/download/index.json";
const SIG_URL: &str =
    "https://github.com/Just8it/Goodtype-Addons/releases/latest/download/index.json.sig";
const KEY: [u8; 32] = [
    0xde, 0x1d, 0x19, 0x23, 0x38, 0x67, 0x1b, 0x55, 0x6a, 0x00, 0xfe, 0xe7, 0xd2, 0xd6, 0xbb, 0x47,
    0x04, 0xc2, 0xa3, 0xba, 0xcb, 0x65, 0x2a, 0x4b, 0xd5, 0x20, 0x90, 0x03, 0xff, 0x01, 0xd6, 0xd9,
];
const SNAPSHOT: &[u8] = include_bytes!("registry-snapshot/index.json");
const SIGNATURE: &[u8] = include_bytes!("registry-snapshot/index.json.sig");
const MAX_INDEX: u64 = MIB;
const MAX_CACHE: usize = 2 * MIB as usize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    format: u8,
    sequence: u64,
    issued: String,
    expires: String,
    addons: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    id: String,
    repository: String,
    public_key: String,
    versions: Vec<Release>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Release {
    pub version: String,
    api: String,
    target: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

impl Release {
    pub fn view(&self) -> ReleaseView {
        ReleaseView {
            version: self.version.clone(),
            size_bytes: self.size,
        }
    }
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn numeric_version(version: &str) -> Option<[u128; 3]> {
    if !manifest::release_version(version) {
        return None;
    }
    let mut parts = version.split('.');
    Some([
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ])
}

// The contract uses UTC timestamps only. Keep the fractional suffix for exact ordering.
fn timestamp(value: &str) -> Option<(i64, &str)> {
    let value = value.strip_suffix('Z')?;
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) if digits(fraction) => (whole, fraction.trim_end_matches('0')),
        Some(_) => return None,
        None => (value, ""),
    };
    let bytes = whole.as_bytes();
    if bytes.len() != 19 || [4, 7, 10, 13, 16].map(|i| bytes[i]) != *b"--T::" {
        return None;
    }
    let number = |start, end| -> Option<i64> {
        let part = whole.get(start..end)?;
        digits(part).then(|| part.parse().ok()).flatten()
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    let leap = |year| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31,
        28 + i64::from(leap(year)),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let month = usize::try_from(month - 1).ok()?;
    if day < 1 || day > *months.get(month)? || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days_before = |year: i64| {
        let previous = year - 1;
        365 * year + previous.div_euclid(4) - previous.div_euclid(100) + previous.div_euclid(400)
    };
    let days =
        days_before(year) - days_before(1970) + months[..month].iter().sum::<i64>() + day - 1;
    Some((days * 86_400 + hour * 3600 + minute * 60 + second, fraction))
}

fn valid_url(url: &str, repo: &str) -> bool {
    let Some(asset) = url.strip_prefix(&format!("https://github.com{repo}/releases/download/"))
    else {
        return false;
    };
    asset.split_once('/').is_some_and(|(tag, file)| {
        [tag, file].into_iter().all(|part| {
            let lower = part.to_ascii_lowercase();
            !part.is_empty()
                && !matches!(lower.replace("%2e", ".").as_str(), "." | "..")
                && !["%2f", "%5c"].iter().any(|escaped| lower.contains(escaped))
                && part.split('%').skip(1).all(|escape| {
                    escape
                        .get(..2)
                        .is_some_and(|value| update::is_hash(value, 2))
                })
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@%".contains(&byte)
                })
        })
    })
}

fn verify(bytes: &[u8], sig: &[u8], key: &[u8; 32]) -> Result<Index, String> {
    if bytes.len() as u64 > MAX_INDEX {
        return Err("addon index exceeds its size limit".into());
    }
    update::verify_signature(bytes, sig, key)?;
    // As with the manifest, closed Serde structs reject both unknown and repeated fields.
    let mut index: Index = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let dates = timestamp(&index.issued).zip(timestamp(&index.expires));
    if index.format != 1
        || index.sequence > 9_007_199_254_740_991
        || !dates.is_some_and(|(issued, expires)| issued < expires)
    {
        return Err("invalid addon index header".into());
    }
    let mut ids = HashSet::new();
    for entry in &index.addons {
        let repo = entry
            .repository
            .strip_prefix('/')
            .and_then(|path| path.split_once('/'));
        if !manifest::identifier(&entry.id)
            || !ids.insert(&entry.id)
            || !update::is_hash(&entry.public_key, 64)
            || !repo.is_some_and(|(owner, name)| {
                !owner.is_empty()
                    && owner
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    && !name.is_empty()
                    && name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            })
        {
            return Err("invalid or duplicate addon entry".into());
        }
        if let Some(addon) = CATALOG.iter().find(|addon| addon.id == entry.id)
            && (entry.repository != addon.repository
                || !addon
                    .public_key
                    .is_some_and(|key| hex(&key).eq_ignore_ascii_case(&entry.public_key)))
        {
            return Err(format!("addon index identity mismatch for {}", entry.id));
        }
        let mut versions = HashSet::new();
        for release in &entry.versions {
            if !numeric_version(&release.version).is_some_and(|value| versions.insert(value))
                || !release
                    .api
                    .strip_prefix("goodtype-api 0.")
                    .is_some_and(digits)
                || release.target != TARGET
                || !valid_url(&release.url, &entry.repository)
                || !(1..=MAX_PACKAGE_BYTES).contains(&release.size)
                || !update::is_hash(&release.sha256, 64)
            {
                return Err(format!("invalid or duplicate release for {}", entry.id));
            }
        }
    }
    index.addons.retain(|entry| {
        let known = CATALOG.iter().any(|addon| addon.id == entry.id);
        if !known {
            eprintln!("Ignoring unknown addon in registry: {}", entry.id);
        }
        known
    });
    Ok(index)
}

impl Index {
    fn expired(&self, now: Duration) -> bool {
        let (seconds, fraction) = timestamp(&self.expires).expect("verified index timestamp");
        let now_fraction = format!("{:09}", now.subsec_nanos());
        (seconds, fraction) <= (now.as_secs() as i64, now_fraction.trim_end_matches('0'))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    sequence: u64,
    last_check: u64,
    bytes: String,
    signature: String,
}

pub(super) struct Registry {
    index: Index,
    pub last_check: Option<u64>,
}

impl Registry {
    pub fn release(&self, addon: &Addon, installed: Option<&str>) -> Option<&Release> {
        self.release_at(addon, installed, now())
    }

    fn release_at(
        &self,
        addon: &Addon,
        installed: Option<&str>,
        now: Duration,
    ) -> Option<&Release> {
        if self.index.expired(now) {
            return None;
        }
        let baseline = installed.and_then(numeric_version);
        self.index
            .addons
            .iter()
            .find(|entry| entry.id == addon.id)?
            .versions
            .iter()
            .filter(|release| release.api == manifest::API && release.target == TARGET)
            .filter(|release| {
                baseline.is_none_or(|version| numeric_version(&release.version).unwrap() > version)
            })
            .max_by_key(|release| numeric_version(&release.version))
    }

    pub fn expired(&self) -> bool {
        self.index.expired(now())
    }
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn load(path: &Path) -> Result<Registry, String> {
    let snapshot = verify(SNAPSHOT, SIGNATURE, &KEY)?;
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(Registry {
            index: snapshot,
            last_check: None,
        });
    }
    let cache: Cache = serde_json::from_slice(&read_bounded(path, MAX_CACHE)?)
        .map_err(|error| error.to_string())?;
    let decode = |value| STANDARD.decode(value).map_err(|error| error.to_string());
    let index = verify(&decode(cache.bytes)?, &decode(cache.signature)?, &KEY)?;
    if index.sequence != cache.sequence {
        return Err("addon cache sequence mismatch".into());
    }
    Ok(Registry {
        index: if index.sequence >= snapshot.sequence {
            index
        } else {
            snapshot
        },
        last_check: Some(cache.last_check),
    })
}

pub(super) fn current(app: &tauri::AppHandle) -> Result<Registry, String> {
    load(&cache_path(app)?)
}

fn cache_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?
        .join("addon-registry.json"))
}

fn registry_url_allowed(initial: &str, url: &str, redirected: bool) -> bool {
    if !redirected {
        return url == initial;
    }
    // GitHub's latest-download endpoint redirects through a tagged release on the same repo.
    if valid_url(url, "/Just8it/Goodtype-Addons")
        && url.rsplit('/').next() == initial.rsplit('/').next()
    {
        return true;
    }
    https_parts(url).is_some_and(|(host, _)| ASSET_HOSTS.contains(&host.as_str()))
}

fn fetch() -> Result<(Vec<u8>, Vec<u8>), String> {
    let get = |initial, limit| {
        update::fetch(initial, limit, |url, redirected| {
            registry_url_allowed(initial, url, redirected)
        })
    };
    Ok((get(URL, MAX_INDEX)?, get(SIG_URL, 64)?))
}

fn accept(index: Index, floor: u64, now: Duration) -> Result<Index, String> {
    if index.sequence < floor {
        return Err("addon index sequence rolled back".into());
    }
    if index.expired(now) {
        return Err("addon index expired".into());
    }
    Ok(index)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Check {
    Startup,
    Manager,
    Manual,
}

fn due(mode: Check, enabled: bool, last: Option<u64>, now: u64) -> bool {
    matches!(mode, Check::Manual)
        || enabled
            && last.is_none_or(|last| {
                now.saturating_sub(last)
                    >= match mode {
                        Check::Startup => 86_400,
                        Check::Manager => 3_600,
                        Check::Manual => unreachable!(),
                    }
            })
}

#[derive(Default)]
pub(crate) struct Checks {
    attempt: Mutex<Option<u64>>,
    failed: AtomicBool,
    checking: AtomicBool,
    outdated: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CheckView {
    last_check: Option<u64>,
    pub checking: bool,
    pub failed: bool,
    expired: bool,
}

impl Checks {
    pub(super) fn view(&self, registry: Option<&Registry>) -> CheckView {
        CheckView {
            last_check: registry.and_then(|registry| registry.last_check),
            checking: self.checking.load(Ordering::Relaxed),
            failed: registry.is_none() || self.failed.load(Ordering::Relaxed),
            expired: registry.is_some_and(Registry::expired)
                || self.outdated.load(Ordering::Relaxed),
        }
    }

    fn perform(
        &self,
        path: &Path,
        mode: Check,
        enabled: bool,
        clock: impl Fn() -> Duration,
        fetch: impl FnOnce() -> Result<(Vec<u8>, Vec<u8>), String>,
    ) -> Result<Option<Index>, String> {
        if !enabled && !matches!(mode, Check::Manual) {
            return Ok(None);
        }
        let Ok(mut attempt) = self.attempt.try_lock() else {
            return Ok(None);
        };
        let lock_path = path.with_extension("lock");
        fs::create_dir_all(path.parent().ok_or("addon cache path has no parent")?)
            .map_err(|error| error.to_string())?;
        crate::store::reject_symlink(&lock_path)?;
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|error| error.to_string())?;
        // Serialize acceptance across app instances as well as concurrent IPC calls.
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Ok(None),
            Err(fs::TryLockError::Error(error)) => return Err(error.to_string()),
        }
        let registry = load(path)?;
        let now = clock();
        if !due(
            mode,
            enabled,
            registry.last_check.max(*attempt),
            now.as_secs(),
        ) {
            return Ok(None);
        }
        *attempt = Some(now.as_secs());
        self.checking.store(true, Ordering::Relaxed);
        let result = (|| {
            let (bytes, signature) = fetch()?;
            let index = verify(&bytes, &signature, &KEY)?;
            let now = clock();
            let expired = index.expired(now);
            let result = accept(index, registry.index.sequence, now);
            self.outdated.store(expired, Ordering::Relaxed);
            let index = result?;
            crate::store::write_json(
                path,
                &Cache {
                    sequence: index.sequence,
                    last_check: now.as_secs(),
                    bytes: STANDARD.encode(bytes),
                    signature: STANDARD.encode(signature),
                },
                MAX_CACHE,
            )?;
            Ok(Some(index))
        })();
        self.failed.store(result.is_err(), Ordering::Relaxed);
        self.checking.store(false, Ordering::Relaxed);
        result
    }
}

fn check(app: &tauri::AppHandle, mode: Check) -> Result<(), String> {
    if !platform_supported() {
        return Ok(());
    }
    let enabled = manager::auto_check(app);
    let path = cache_path(app)?;
    let checks = app.state::<Checks>();
    if let Some(index) = checks.perform(&path, mode, enabled, now, fetch)? {
        let registry = Registry {
            index,
            last_check: None,
        };
        let has_update = CATALOG.iter().any(|addon| {
            install_dir(app, addon)
                .ok()
                .and_then(|dir| dir.active())
                .is_some_and(|(version, _)| registry.release(addon, Some(&version)).is_some())
        });
        if has_update {
            let _ = app.emit("addon-update-available", ());
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn addon_check_registry(app: tauri::AppHandle, mode: Check) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || check(&app, mode))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::{Value, json};

    fn signed(value: &Value) -> Result<Index, String> {
        let key = SigningKey::from_bytes(&[17; 32]);
        let bytes = serde_json::to_vec(value).unwrap();
        verify(
            &bytes,
            &key.sign(&bytes).to_bytes(),
            key.verifying_key().as_bytes(),
        )
    }

    fn fixture() -> Value {
        serde_json::from_slice(SNAPSHOT).unwrap()
    }

    fn at(date: &str) -> Duration {
        Duration::from_secs(timestamp(date).unwrap().0 as u64)
    }

    #[test]
    fn bundled_registry_authenticates_exact_bytes() {
        let index = verify(SNAPSHOT, SIGNATURE, &KEY).unwrap();
        assert_eq!(index.sequence, 1);
        assert_eq!(index.addons.len(), 2);
        assert!(verify(SNAPSHOT, &[], &KEY).is_err());
        assert!(verify(SNAPSHOT, &[0; 64], &KEY).is_err());
        let mut modified = SNAPSHOT.to_vec();
        modified.push(b' ');
        assert!(verify(&modified, SIGNATURE, &KEY).is_err());
    }

    #[test]
    fn bundled_floor_and_expiry_are_enforced() {
        let file = tempfile::tempdir().unwrap().path().join("registry.json");
        let floor = load(&file).unwrap().index.sequence;
        let mut input = fixture();
        input["sequence"] = json!(0);
        assert!(accept(signed(&input).unwrap(), floor, at("2026-11-22T00:00:00Z")).is_err());
        let index = signed(&fixture()).unwrap();
        assert!(accept(index, floor, at("2027-03-21T11:36:11Z")).is_err());
        assert!(timestamp("2100-02-29T00:00:00Z").is_none());
        assert!(timestamp("2000-02-29T00:00:00.1Z").is_some());
        assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some((0, "")));
    }

    #[test]
    fn unknown_ids_are_ignored_and_catalog_binding_is_strict() {
        let mut input = fixture();
        input["addons"][0]["id"] = json!("unlisted");
        assert_eq!(signed(&input).unwrap().addons.len(), 1);
        for (path, value) in [
            ("/addons/0/publicKey", json!("1".repeat(64))),
            ("/addons/0/repository", json!("/Another/repo")),
            (
                "/addons/0/versions/0/url",
                json!("https://github.com/Another/repo/releases/download/v1/test.zip"),
            ),
            ("/addons/0/versions/0/size", json!(MAX_PACKAGE_BYTES + 1)),
            ("/issued", json!("2026-11-21T12:00:00+00:00")),
        ] {
            let mut input = fixture();
            *input.pointer_mut(path).unwrap() = value;
            assert!(signed(&input).is_err(), "{path}");
        }
    }

    #[test]
    fn signed_indexes_reject_unknown_and_duplicate_fields_and_entries() {
        let mut input = fixture();
        input["addons"][0]["versions"][0]["extra"] = json!(true);
        assert!(signed(&input).is_err());

        let mut input = fixture();
        let addon = input["addons"][0].clone();
        input["addons"].as_array_mut().unwrap().push(addon);
        assert!(signed(&input).is_err());

        let mut input = fixture();
        let release = input["addons"][0]["versions"][0].clone();
        input["addons"][0]["versions"]
            .as_array_mut()
            .unwrap()
            .push(release);
        assert!(signed(&input).is_err());

        let bytes = String::from_utf8(SNAPSHOT.to_vec()).unwrap().replace(
            "\"size\": 14881982",
            "\"size\": 14881982, \"size\": 14881982",
        );
        let key = SigningKey::from_bytes(&[17; 32]);
        let bytes = bytes.as_bytes();
        assert!(
            verify(
                bytes,
                &key.sign(bytes).to_bytes(),
                key.verifying_key().as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn selects_latest_numeric_compatible_release() {
        let mut input = fixture();
        let versions = input["addons"][0]["versions"].as_array_mut().unwrap();
        for (version, api) in [
            ("0.1.9", manifest::API),
            ("0.1.10", manifest::API),
            ("0.2.0", "goodtype-api 0.2"),
        ] {
            let mut release = versions[0].clone();
            release["version"] = json!(version);
            release["api"] = json!(api);
            versions.push(release);
        }
        let index = signed(&input).unwrap();
        let registry = Registry {
            index,
            last_check: None,
        };
        let addon = &catalog::GOODNOTES;
        // Use a fixed check time so the test remains useful after the bundled index expires.
        assert_eq!(
            registry
                .release_at(addon, Some("0.1.9"), at("2026-11-22T00:00:00Z"))
                .unwrap()
                .version,
            "0.1.10"
        );
        assert!(
            registry
                .release_at(addon, Some("0.1.10"), at("2026-11-22T00:00:00Z"))
                .is_none()
        );
    }

    #[test]
    fn disabled_checks_throttle_and_offline_leaves_the_cache_unchanged() {
        let local = tempfile::tempdir().unwrap();
        let path = local.path().join("registry.json");
        let checks = Checks::default();
        let start = at("2026-11-22T00:00:00Z");
        for mode in [Check::Startup, Check::Manager] {
            assert!(
                checks
                    .perform(&path, mode, false, || start, || panic!("disabled fetch"))
                    .unwrap()
                    .is_none()
            );
        }
        let snapshot = || Ok((SNAPSHOT.to_vec(), SIGNATURE.to_vec()));
        assert!(
            checks
                .perform(&path, Check::Manual, false, || start, snapshot)
                .unwrap()
                .is_some()
        );
        let cached = fs::read(&path).unwrap();
        let restarted = Checks::default();
        for (mode, seconds) in [(Check::Manager, 3_599), (Check::Startup, 86_399)] {
            assert!(
                restarted
                    .perform(
                        &path,
                        mode,
                        true,
                        || start + Duration::from_secs(seconds),
                        || panic!("throttled fetch")
                    )
                    .unwrap()
                    .is_none()
            );
        }
        let offline = || Err("offline".into());
        assert!(
            restarted
                .perform(
                    &path,
                    Check::Manager,
                    true,
                    || start + Duration::from_secs(3_600),
                    offline
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), cached);
        assert_eq!(load(&path).unwrap().last_check, Some(start.as_secs()));
        assert!(
            checks
                .perform(
                    &path,
                    Check::Startup,
                    true,
                    || start + Duration::from_secs(86_400),
                    snapshot
                )
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn registry_redirects_stay_with_the_named_asset_or_github_asset_hosts() {
        let tagged =
            "https://github.com/Just8it/Goodtype-Addons/releases/download/index-1/index.json";
        assert!(registry_url_allowed(URL, URL, false));
        assert!(!registry_url_allowed(URL, tagged, false));
        assert!(registry_url_allowed(URL, tagged, true));
        assert!(registry_url_allowed(
            URL,
            "https://release-assets.githubusercontent.com/path?token=value",
            true
        ));
        assert!(!registry_url_allowed(SIG_URL, tagged, true));
        assert!(!registry_url_allowed(
            URL,
            &tagged.replace("Goodtype-Addons", "AnotherRepo"),
            true
        ));
        assert!(!registry_url_allowed(
            URL,
            "http://release-assets.githubusercontent.com/path",
            true
        ));
    }

    #[test]
    #[ignore = "fetches the public GitHub registry"]
    fn live_registry_verifies_the_published_index() {
        let (bytes, signature) = fetch().unwrap();
        let snapshot = verify(SNAPSHOT, SIGNATURE, &KEY).unwrap();
        let index = accept(
            verify(&bytes, &signature, &KEY).unwrap(),
            snapshot.sequence,
            now(),
        )
        .unwrap();
        println!(
            "Verified registry sequence {} with {} catalog addons, expiring {}.",
            index.sequence,
            index.addons.len(),
            index.expires
        );
    }
}
