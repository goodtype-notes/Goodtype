use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use tauri::Emitter;

#[cfg(test)]
const URL: &str =
    "https://github.com/goodtype-notes/Goodtype-Addons/releases/latest/download/index.json";
#[cfg(test)]
const SIG_URL: &str =
    "https://github.com/goodtype-notes/Goodtype-Addons/releases/latest/download/index.json.sig";
const KEY: [u8; 32] = [
    0xde, 0x1d, 0x19, 0x23, 0x38, 0x67, 0x1b, 0x55, 0x6a, 0x00, 0xfe, 0xe7, 0xd2, 0xd6, 0xbb, 0x47,
    0x04, 0xc2, 0xa3, 0xba, 0xcb, 0x65, 0x2a, 0x4b, 0xd5, 0x20, 0x90, 0x03, 0xff, 0x01, 0xd6, 0xd9,
];
const SNAPSHOT: &[u8] = include_bytes!("registry-snapshot/index.json");
const SIGNATURE: &[u8] = include_bytes!("registry-snapshot/index.json.sig");
const MAX_INDEX: u64 = MIB;
const MAX_CACHE: usize = 2 * MIB as usize;
const DAY: u64 = 86_400;
const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;
const MAX_STATE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Problem {
    Offline,
    Status,
    NotFound,
    Signature,
    Expired,
    Rollback,
    Invalid,
}

impl From<String> for Problem {
    fn from(_: String) -> Self {
        Self::Invalid
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    #[serde(skip)]
    digest: String,
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
    #[serde(default, deserialize_with = "optional_object")]
    docs: Option<Docs>,
}

fn optional_object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Docs {
    pub commit: String,
    pub summary: Summary,
    pub readme: DocAsset,
    #[serde(default, deserialize_with = "optional_object")]
    pub preview: Option<DocAsset>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Summary {
    en: String,
    de: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DocAsset {
    pub path: String,
    pub size: usize,
    pub sha256: String,
}

pub(super) const MAX_README: usize = 262_144;
pub(super) const MAX_PREVIEW: usize = 2_097_152;

impl DocAsset {
    fn valid(&self, max_size: usize) -> bool {
        !self.path.is_empty()
            && !self.path.starts_with('/')
            && !self.path.contains("..")
            && self
                .path
                .split('/')
                .all(|part| !part.is_empty() && part != ".")
            && self
                .path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
            && self.size <= max_size
            && update::is_hash(&self.sha256, 64)
    }
}

impl Docs {
    fn valid(&self) -> bool {
        update::is_hash(&self.commit, 40)
            && !self.commit.bytes().any(|b| b.is_ascii_uppercase())
            && [&self.summary.en, &self.summary.de]
                .into_iter()
                .all(|text| {
                    (1..=160).contains(&text.chars().count()) && !text.chars().any(char::is_control)
                })
            && self.readme.valid(MAX_README)
            && self
                .preview
                .as_ref()
                .is_none_or(|asset| asset.valid(MAX_PREVIEW))
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Release {
    pub version: String,
    api: String,
    target: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    min_goodtype: Option<String>,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(flatten)]
    unknown: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Release {
    fn compatible(&self, runtime: Option<&str>) -> bool {
        self.api == manifest::API
            && self.target == TARGET
            && self.unknown.is_empty()
            && self.requires.len() <= 16
            && self.requires.iter().collect::<HashSet<_>>().len() == self.requires.len()
            && self
                .requires
                .iter()
                .all(|feature| manifest::supported_feature(feature))
            && self.min_goodtype.as_ref().is_none_or(|minimum| {
                numeric_version(minimum).is_some_and(|minimum| {
                    runtime
                        .and_then(numeric_version)
                        .is_none_or(|running| running >= minimum)
                })
            })
    }

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

pub(super) fn numeric_version(version: &str) -> Option<[u128; 3]> {
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

pub(super) fn valid_url(url: &str, repo: &str) -> bool {
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

fn verify(bytes: &[u8], sig: &[u8], key: &[u8; 32]) -> Result<Index, Problem> {
    if bytes.len() as u64 > MAX_INDEX {
        return Err(Problem::Invalid);
    }
    update::verify_signature(bytes, sig, key).map_err(|_| Problem::Signature)?;
    // Closed Serde structs reject both unknown and repeated fields.
    let mut index: Index = serde_json::from_slice(bytes).map_err(|_| Problem::Invalid)?;
    if index.format == 1 {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| Problem::Invalid)?;
        if value["addons"].as_array().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["versions"].as_array().is_some_and(|versions| {
                    versions.iter().any(|release| {
                        release.as_object().is_some_and(|fields| {
                            fields.contains_key("minGoodtype") || fields.contains_key("requires")
                        })
                    })
                })
            })
        }) {
            return Err(Problem::Invalid);
        }
    }
    index.digest = hex(&Sha256::digest(bytes));
    let dates = timestamp(&index.issued).zip(timestamp(&index.expires));
    if ![1, 2].contains(&index.format)
        || index.sequence > MAX_SEQUENCE
        || !dates.is_some_and(|(issued, expires)| issued < expires)
    {
        return Err(Problem::Invalid);
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
            || entry.docs.as_ref().is_some_and(|docs| !docs.valid())
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
            return Err(Problem::Invalid);
        }
        if let Some(addon) = CATALOG.iter().find(|addon| addon.id == entry.id)
            && (entry.repository != addon.repository
                || !addon
                    .public_key
                    .is_some_and(|key| hex(&key).eq_ignore_ascii_case(&entry.public_key)))
        {
            return Err(Problem::Invalid);
        }
        let mut versions = HashSet::new();
        for release in &entry.versions {
            if index.format == 1
                && (release.min_goodtype.is_some()
                    || !release.requires.is_empty()
                    || !release.unknown.is_empty())
            {
                return Err(Problem::Invalid);
            }
            if !numeric_version(&release.version).is_some_and(|value| versions.insert(value))
                || !release
                    .api
                    .strip_prefix("goodtype-api 0.")
                    .is_some_and(digits)
                || !valid_url(&release.url, &entry.repository)
                || !(1..=MAX_PACKAGE_BYTES).contains(&release.size)
                || !update::is_hash(&release.sha256, 64)
            {
                return Err(Problem::Invalid);
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
    #[serde(default = "format_one")]
    format: u8,
    sequence: u64,
    last_check: Option<u64>,
    bytes: String,
    signature: String,
}

fn format_one() -> u8 {
    1
}

pub(super) struct Registry {
    runtime: Option<String>,
    index: Index,
    pub last_check: Option<u64>,
    floor: Floor,
    cached: bool,
    last_attempt: Option<u64>,
}

impl Registry {
    pub fn blocked(&self, addon: &Addon, installed: Option<&str>) -> Option<Option<String>> {
        let newest = self
            .index
            .addons
            .iter()
            .find(|entry| entry.id == addon.id)?
            .versions
            .iter()
            .max_by_key(|release| numeric_version(&release.version))?;
        let baseline = installed
            .and_then(numeric_version)
            .into_iter()
            .chain(
                self.details_release(addon)
                    .and_then(|release| numeric_version(&release.version)),
            )
            .max();
        (!newest.compatible(self.runtime.as_deref())
            && baseline.is_none_or(|version| numeric_version(&newest.version).unwrap() > version))
        .then(|| {
            newest
                .min_goodtype
                .clone()
                .filter(|version| numeric_version(version).is_some())
        })
    }

    /// A listed `version` this app cannot run, with its minimum Goodtype version when known.
    /// Unlisted versions (sideloads, legacy archives) are left to the manifest checks.
    pub fn unrunnable(&self, addon: &Addon, version: &str) -> Option<Option<String>> {
        let release = self
            .index
            .addons
            .iter()
            .find(|entry| entry.id == addon.id)?
            .versions
            .iter()
            .find(|release| release.version == version)?;
        (!release.compatible(self.runtime.as_deref())).then(|| {
            release
                .min_goodtype
                .clone()
                .filter(|version| numeric_version(version).is_some())
        })
    }

    pub fn docs(&self, addon: &Addon) -> Option<&Docs> {
        self.index
            .addons
            .iter()
            .find(|entry| entry.id == addon.id)?
            .docs
            .as_ref()
    }

    pub fn details_release(&self, addon: &Addon) -> Option<&Release> {
        self.compatible_releases(addon)?
            .max_by_key(|release| numeric_version(&release.version))
    }

    fn compatible_releases(&self, addon: &Addon) -> Option<impl Iterator<Item = &Release>> {
        Some(
            self.index
                .addons
                .iter()
                .find(|entry| entry.id == addon.id)?
                .versions
                .iter()
                .filter(|release| release.compatible(self.runtime.as_deref())),
        )
    }

    pub fn release(&self, addon: &Addon, installed: Option<&str>) -> Option<&Release> {
        self.release_at(addon, installed, false, now())
    }

    pub fn install_release(&self, addon: &Addon, installed: Option<&str>) -> Option<&Release> {
        self.release_at(addon, installed, true, now())
    }

    fn release_at(
        &self,
        addon: &Addon,
        installed: Option<&str>,
        include_current: bool,
        now: Duration,
    ) -> Option<&Release> {
        if self.index.expired(now) {
            return None;
        }
        let baseline = installed.and_then(numeric_version);
        self.compatible_releases(addon)?
            .filter(|release| {
                baseline.is_none_or(|version| {
                    let candidate = numeric_version(&release.version).unwrap();
                    candidate > version || (include_current && candidate == version)
                })
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Floor {
    sequence: u64,
    digest: String,
    #[serde(default = "format_one")]
    format: u8,
    #[serde(default)]
    other_digest: Option<String>,
}

impl From<&Index> for Floor {
    fn from(index: &Index) -> Self {
        Self {
            sequence: index.sequence,
            digest: index.digest.clone(),
            format: index.format,
            other_digest: None,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    floor: Option<Floor>,
    last_attempt: Option<u64>,
    last_success: Option<u64>,
}

fn save_state(path: &Path, state: &State) -> Result<(), Problem> {
    let bytes = crate::store::json_bytes(state, MAX_STATE)?;
    crate::store::write_atomic(&path.with_extension("state.bak"), &bytes)?;
    crate::store::write_atomic(&path.with_extension("state.json"), &bytes)?;
    Ok(())
}

fn load(path: &Path, key: &[u8; 32]) -> Result<Registry, Problem> {
    let snapshot = verify(SNAPSHOT, SIGNATURE, &KEY)?;
    let read_state = |extension| -> Option<State> {
        serde_json::from_slice(&read_bounded(&path.with_extension(extension), MAX_STATE).ok()?).ok()
    };
    let primary = read_state("state.json");
    let backup = read_state("state.bak");
    let mut floor = Floor::from(&snapshot);
    // Recover the highest surviving floor even when the cache is missing or damaged.
    // Fully deleting all app data resets protection to the bundled floor.
    for state in [&primary, &backup].into_iter().flatten() {
        if let Some(saved) = &state.floor
            && saved.sequence <= MAX_SEQUENCE
            && update::is_hash(&saved.digest, 64)
            && (saved.sequence > floor.sequence
                || saved.sequence == floor.sequence
                    && floor.other_digest.is_none()
                    && (saved.format == floor.format && saved.digest == floor.digest
                        || saved.format != floor.format
                            && saved.other_digest.as_ref() == Some(&floor.digest)))
        {
            floor = saved.clone();
        }
    }
    let cache = (|| {
        let cache: Cache = serde_json::from_slice(&read_bounded(path, MAX_CACHE).ok()?).ok()?;
        let index = verify(
            &STANDARD.decode(cache.bytes).ok()?,
            &STANDARD.decode(cache.signature).ok()?,
            key,
        )
        .ok()?;
        (index.sequence == cache.sequence && index.format == cache.format)
            .then_some((index, cache.last_check))
    })();
    let old_check = cache.as_ref().and_then(|(_, check)| *check);
    let state = primary.or(backup).unwrap_or_default();
    let index = cache.map(|(index, _)| index).filter(|index| {
        index.sequence > floor.sequence
            || (index.sequence == floor.sequence
                && (index.digest == floor.digest && index.format == floor.format
                    || index.format != floor.format
                        && floor
                            .other_digest
                            .as_ref()
                            .is_none_or(|digest| *digest == index.digest)))
    });
    if let Some(index) = &index
        && index.sequence > floor.sequence
    {
        floor = Floor::from(index);
    }
    Ok(Registry {
        runtime: None,
        cached: index.is_some(),
        index: index.unwrap_or(snapshot),
        last_check: state.last_success.or(old_check),
        last_attempt: state.last_attempt.or(old_check),
        floor,
    })
}

pub(super) fn current(app: &tauri::AppHandle) -> Result<Registry, Problem> {
    let mut registry = load(&cache_path(app)?, &KEY)?;
    registry.runtime = Some(app.package_info().version.to_string());
    Ok(registry)
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
    if valid_url(url, "/goodtype-notes/Goodtype-Addons")
        && url.rsplit('/').next() == initial.rsplit('/').next()
    {
        return true;
    }
    https_parts(url).is_some_and(|(host, _)| ASSET_HOSTS.contains(&host.as_str()))
}

fn fetch() -> Result<(Vec<u8>, Vec<u8>), Problem> {
    let mut format = 0;
    let (bytes, signature) = fetch_preferred(|file| {
        format = if file == "index-v2.json" { 2 } else { 1 };
        fetch_format(file)
    })?;
    served_as(&bytes, &signature, &KEY, format)?;
    Ok((bytes, signature))
}

/// Both formats share one key and sequence, so a signed format-1 index served under the format-2
/// name would otherwise be accepted and hide every release that carries requirements.
fn served_as(bytes: &[u8], signature: &[u8], key: &[u8; 32], format: u8) -> Result<(), Problem> {
    (verify(bytes, signature, key)?.format == format)
        .then_some(())
        .ok_or(Problem::Invalid)
}

fn fetch_preferred(
    mut fetch_format: impl FnMut(&str) -> Result<(Vec<u8>, Vec<u8>), Problem>,
) -> Result<(Vec<u8>, Vec<u8>), Problem> {
    match fetch_format("index-v2.json") {
        Err(Problem::NotFound) => fetch_format("index.json"),
        result => result,
    }
}

fn fetch_format(file: &str) -> Result<(Vec<u8>, Vec<u8>), Problem> {
    let initial = format!(
        "https://github.com/goodtype-notes/Goodtype-Addons/releases/latest/download/{file}"
    );
    // `latest/download` resolves each asset separately, and GitHub caches those redirects
    // independently: right after a new release it served index-1's index.json with index-2's
    // signature (observed 2026-11-21). So remember which tagged release the index came from and
    // take the signature from that same release.
    let tag = std::cell::RefCell::new(None);
    let index = update::fetch(&initial, MAX_INDEX, |url, redirected| {
        let allowed = registry_url_allowed(&initial, url, redirected);
        if allowed && redirected && tag.borrow().is_none() {
            *tag.borrow_mut() = release_tag(url);
        }
        allowed
    })?;
    let signature_url = match tag.into_inner() {
        Some(tag) => format!("{RELEASES}/{tag}/{file}.sig"),
        None => format!("{initial}.sig"),
    };
    let signature = update::fetch(&signature_url, 64, |url, redirected| {
        registry_url_allowed(&signature_url, url, redirected)
    })
    .map_err(|problem| {
        if problem == Problem::NotFound {
            Problem::Status
        } else {
            problem
        }
    })?;
    Ok((index, signature))
}

const RELEASES: &str = "https://github.com/goodtype-notes/Goodtype-Addons/releases/download";

/// `index-<n>` from a same-repository tagged download URL of `index.json`, else nothing.
fn release_tag(url: &str) -> Option<String> {
    let tag = url
        .strip_prefix(RELEASES)?
        .strip_prefix('/')?
        .strip_suffix("/index.json")
        .or_else(|| {
            url.strip_prefix(RELEASES)?
                .strip_prefix('/')?
                .strip_suffix("/index-v2.json")
        })?;
    let number = tag.strip_prefix("index-")?;
    (!number.is_empty() && number.len() <= 16 && number.bytes().all(|b| b.is_ascii_digit()))
        .then(|| tag.to_owned())
}

fn accept(index: &Index, floor: &Floor, now: Duration) -> Result<(), Problem> {
    if index.sequence < floor.sequence
        || (index.sequence == floor.sequence
            && if index.format == floor.format {
                index.digest != floor.digest
            } else {
                floor
                    .other_digest
                    .as_ref()
                    .is_some_and(|digest| *digest != index.digest)
            })
    {
        return Err(Problem::Rollback);
    }
    if index.expired(now) {
        return Err(Problem::Expired);
    }
    Ok(())
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
                last > now.saturating_add(DAY)
                    || now.saturating_sub(last)
                        >= match mode {
                            Check::Startup => DAY,
                            Check::Manager => 3_600,
                            Check::Manual => unreachable!(),
                        }
            })
}

#[derive(Default)]
pub(crate) struct Checks {
    gate: Mutex<()>,
    problem: Mutex<Option<Problem>>,
    checking: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CheckView {
    last_check: Option<u64>,
    pub checking: bool,
    pub problem: Option<Problem>,
    expired: bool,
}

impl Checks {
    pub(super) fn view(&self, registry: Option<&Registry>) -> CheckView {
        CheckView {
            last_check: registry.and_then(|registry| registry.last_check),
            checking: self.checking.load(Ordering::Relaxed),
            problem: self
                .problem
                .lock()
                .unwrap()
                .or_else(|| registry.is_none().then_some(Problem::Invalid)),
            expired: registry.is_some_and(Registry::expired),
        }
    }

    fn perform(
        &self,
        path: &Path,
        mode: Check,
        enabled: bool,
        (clock, key): (impl Fn() -> Duration, &[u8; 32]),
        fetch: impl FnOnce() -> Result<(Vec<u8>, Vec<u8>), Problem>,
    ) -> Result<Option<Index>, Problem> {
        if !enabled && !matches!(mode, Check::Manual) {
            return Ok(None);
        }
        let Ok(_gate) = self.gate.try_lock() else {
            return Ok(None);
        };
        let lock_path = path.with_extension("lock");
        fs::create_dir_all(path.parent().ok_or(Problem::Invalid)?)
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
            Err(fs::TryLockError::Error(_)) => return Err(Problem::Invalid),
        }
        let registry = load(path, key)?;
        let now = clock();
        if !due(mode, enabled, registry.last_attempt, now.as_secs()) {
            return Ok(None);
        }
        let mut state = State {
            floor: Some(registry.floor.clone()),
            last_attempt: Some(now.as_secs()),
            last_success: registry.last_check,
        };
        self.checking.store(true, Ordering::Relaxed);
        let result = (|| {
            save_state(path, &state)?;
            let (bytes, signature) = fetch()?;
            let index = verify(&bytes, &signature, key)?;
            let now = clock();
            accept(&index, &registry.floor, now)?;
            let replaced = !registry.cached
                || index.sequence > registry.floor.sequence
                || index.format != registry.index.format;
            if replaced {
                // Publish the floor before the index so deleting the cache cannot undo acceptance.
                let mut floor = Floor::from(&index);
                if index.sequence == registry.floor.sequence {
                    floor.other_digest = if index.format != registry.floor.format {
                        Some(registry.floor.digest.clone())
                    } else {
                        registry.floor.other_digest.clone()
                    };
                }
                state.floor = Some(floor);
                save_state(path, &state)?;
                crate::store::write_json(
                    path,
                    &Cache {
                        format: index.format,
                        sequence: index.sequence,
                        last_check: None,
                        bytes: STANDARD.encode(bytes),
                        signature: STANDARD.encode(signature),
                    },
                    MAX_CACHE,
                )?;
            }
            state.last_success = Some(now.as_secs());
            save_state(path, &state)?;
            Ok(replaced.then_some(index))
        })();
        *self.problem.lock().unwrap() = result.as_ref().err().copied();
        self.checking.store(false, Ordering::Relaxed);
        result
    }
}

fn check(app: &tauri::AppHandle, mode: Check) -> Result<(), Problem> {
    if !platform_supported() {
        return Ok(());
    }
    let enabled = manager::auto_check(app);
    let path = cache_path(app)?;
    let checks = app.state::<Checks>();
    let result = checks.perform(&path, mode, enabled, (now, &KEY), fetch);
    if let Err(problem) = result {
        eprintln!("Addon registry check failed: {problem:?}");
        *checks.problem.lock().unwrap() = Some(problem);
    }
    if let Some(index) = result? {
        let registry = Registry {
            runtime: Some(app.package_info().version.to_string()),
            index,
            ..load(&path, &KEY)?
        };
        let has_update = CATALOG.iter().any(|addon| {
            install_dir(app, addon)
                .ok()
                .and_then(|dir| dir.active())
                .is_some_and(|(version, _)| registry.release(addon, Some(&version)).is_some())
        });
        let skipped = CATALOG.iter().any(|addon| {
            install_dir(app, addon)
                .ok()
                .and_then(|dir| dir.active())
                .is_some_and(|(version, _)| registry.blocked(addon, Some(&version)).is_some())
        });
        if has_update {
            let _ = app.emit("addon-update-available", ());
        }
        if skipped {
            let _ = app.emit("addon-update-needs-app", ());
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn addon_check_registry(
    app: tauri::AppHandle,
    mode: Check,
) -> Result<(), Problem> {
    tauri::async_runtime::spawn_blocking(move || check(&app, mode))
        .await
        .map_err(|_| Problem::Invalid)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::{Value, json};

    fn signed(value: &Value) -> Result<Index, Problem> {
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
    fn format_two_tolerates_unknown_release_fields_and_features() {
        let mut input = fixture();
        input["format"] = json!(2);
        input["addons"][0]["versions"][0]["minGoodtype"] = json!("0.3.10");
        let index = signed(&input).unwrap();
        let release = &index.addons[0].versions[0];
        assert!(!release.compatible(Some("0.3.9")));
        assert!(release.compatible(Some("0.3.10")));
        assert!(release.compatible(Some("0.3.1-local")));
        assert!(release.compatible(None));
        let mut legacy = input.clone();
        legacy["format"] = json!(1);
        assert!(signed(&legacy).is_err());
        input["addons"][0]["versions"][0]["requires"] = json!(["future-feature"]);
        assert!(!signed(&input).unwrap().addons[0].versions[0].compatible(None));
        input["addons"][0]["versions"][0]["requires"] = json!([]);
        input["addons"][0]["versions"][0]["future"] = json!(true);
        assert!(!signed(&input).unwrap().addons[0].versions[0].compatible(None));
        input["addons"][0]["future"] = json!(true);
        assert!(signed(&input).is_err());
    }

    #[test]
    fn preferred_index_falls_back_only_on_404() {
        for problem in [
            Problem::Offline,
            Problem::Status,
            Problem::Signature,
            Problem::Expired,
            Problem::Rollback,
            Problem::Invalid,
            Problem::NotFound,
        ] {
            let mut calls = Vec::new();
            let result = fetch_preferred(|file| {
                calls.push(file.to_owned());
                if file == "index-v2.json" {
                    Err(problem)
                } else {
                    Ok((vec![], vec![]))
                }
            });
            assert_eq!(
                calls.len(),
                if problem == Problem::NotFound { 2 } else { 1 }
            );
            assert_eq!(result.is_ok(), problem == Problem::NotFound);
        }
    }

    #[test]
    fn an_index_must_match_the_file_it_was_served_as() {
        let signing = SigningKey::from_bytes(&[23; 32]);
        let key = signing.verifying_key();
        for format in [1, 2] {
            let mut input = fixture();
            input["format"] = json!(format);
            let bytes = serde_json::to_vec(&input).unwrap();
            let signature = signing.sign(&bytes).to_bytes();
            assert_eq!(
                served_as(&bytes, &signature, key.as_bytes(), format),
                Ok(())
            );
            assert_eq!(
                served_as(&bytes, &signature, key.as_bytes(), 3 - format),
                Err(Problem::Invalid)
            );
        }
    }

    #[test]
    fn cache_records_format_and_preserves_shared_floor_across_restart() {
        let local = tempfile::tempdir().unwrap();
        let path = local.path().join("registry.json");
        let signing = SigningKey::from_bytes(&[19; 32]);
        let verifying = signing.verifying_key();
        let key = verifying.as_bytes();
        // Above the bundled snapshot's sequence 4, whose real digest would otherwise reject the fixture.
        let time = at("2026-11-22T00:00:00Z");
        let download = |format, sequence, changed| {
            let mut input = fixture();
            input["format"] = json!(format);
            input["sequence"] = json!(sequence);
            let mut bytes = serde_json::to_vec(&input).unwrap();
            if changed {
                bytes.push(b' ');
            }
            let signature = signing.sign(&bytes).to_bytes().to_vec();
            Ok((bytes, signature))
        };
        for format in [1, 2, 1] {
            Checks::default()
                .perform(&path, Check::Manual, true, (|| time, key), || {
                    download(format, 5, false)
                })
                .unwrap();
            let cache: Cache = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(cache.format, format);
            assert_eq!(load(&path, key).unwrap().floor.sequence, 5);
        }
        for (format, sequence, changed) in [(2, 4, false), (2, 5, true), (1, 5, true)] {
            assert!(matches!(
                Checks::default().perform(&path, Check::Manual, true, (|| time, key), || download(
                    format, sequence, changed
                )),
                Err(Problem::Rollback)
            ));
        }
    }

    #[test]
    fn formats_share_rollback_floor() {
        let one = signed(&fixture()).unwrap();
        let mut input = fixture();
        input["format"] = json!(2);
        let two = signed(&input).unwrap();
        let time = at("2026-11-22T00:00:00Z");
        accept(&two, &Floor::from(&one), time).unwrap();
        accept(&one, &Floor::from(&two), time).unwrap();
        input["sequence"] = json!(one.sequence - 1);
        assert_eq!(
            accept(&signed(&input).unwrap(), &Floor::from(&one), time),
            Err(Problem::Rollback)
        );
    }

    #[test]
    fn incompatible_update_keeps_installed_and_selects_newest_compatible() {
        let mut input = fixture();
        input["format"] = json!(2);
        let versions = input["addons"][0]["versions"].as_array_mut().unwrap();
        for (version, minimum) in [("8.0.0", "0.3.0"), ("9.0.0", "99.0.0")] {
            let mut release = versions[0].clone();
            release["version"] = json!(version);
            release["minGoodtype"] = json!(minimum);
            versions.push(release);
        }
        let index = signed(&input).unwrap();
        let registry = Registry {
            runtime: Some("0.3.10".into()),
            floor: Floor::from(&index),
            index,
            last_check: None,
            last_attempt: None,
            cached: false,
        };
        let time = at("2026-11-22T00:00:00Z");
        assert_eq!(
            registry
                .release_at(&catalog::GOODNOTES, None, false, time)
                .unwrap()
                .version,
            "8.0.0"
        );
        assert!(
            registry
                .release_at(&catalog::GOODNOTES, Some("8.0.0"), false, time)
                .is_none()
        );
        assert_eq!(
            registry.blocked(&catalog::GOODNOTES, Some("8.0.0")),
            Some(Some("99.0.0".into()))
        );
        // Rollback consults the same rule: a listed release this app cannot run is refused,
        // while unlisted versions are left to the manifest checks.
        assert_eq!(
            registry.unrunnable(&catalog::GOODNOTES, "9.0.0"),
            Some(Some("99.0.0".into()))
        );
        assert_eq!(registry.unrunnable(&catalog::GOODNOTES, "8.0.0"), None);
        assert_eq!(registry.unrunnable(&catalog::GOODNOTES, "7.7.7"), None);
    }

    #[test]
    fn bundled_registry_authenticates_exact_bytes() {
        let index = verify(SNAPSHOT, SIGNATURE, &KEY).unwrap();
        assert_eq!(index.sequence, 4);
        assert_eq!(index.addons.len(), 6);
        assert!(verify(SNAPSHOT, &[], &KEY).is_err());
        assert!(verify(SNAPSHOT, &[0; 64], &KEY).is_err());
        let mut modified = SNAPSHOT.to_vec();
        modified.push(b' ');
        assert!(verify(&modified, SIGNATURE, &KEY).is_err());
    }

    #[test]
    fn bundled_floor_and_expiry_are_enforced() {
        let file = tempfile::tempdir().unwrap().path().join("registry.json");
        let floor = load(&file, &KEY).unwrap().floor;
        let mut input = fixture();
        input["sequence"] = json!(0);
        assert!(accept(&signed(&input).unwrap(), &floor, at("2026-11-22T00:00:00Z")).is_err());
        let index = verify(SNAPSHOT, SIGNATURE, &KEY).unwrap();
        assert_eq!(
            accept(&index, &floor, at("2027-03-24T09:51:51Z")),
            Err(Problem::Expired)
        );
        assert!(timestamp("2100-02-29T00:00:00Z").is_none());
        assert!(timestamp("2000-02-29T00:00:00.1Z").is_some());
        assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some((0, "")));
    }

    #[test]
    fn unknown_ids_are_ignored_and_catalog_binding_is_strict() {
        let mut input = fixture();
        let listed = input["addons"].as_array().unwrap().len();
        input["addons"][0]["id"] = json!("unlisted");
        assert_eq!(signed(&input).unwrap().addons.len(), listed - 1);
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

        let bytes = String::from_utf8(SNAPSHOT.to_vec()).unwrap().replacen(
            "\"size\": ",
            "\"size\": 1, \"size\": ",
            1,
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
    fn optional_docs_are_strict_and_validate_paths_and_limits() {
        let mut index = fixture();
        let entry = &mut index["addons"][0];
        let docs = json!({
            "commit": "a".repeat(40),
            "summary": { "en": "A sample", "de": "Ein Beispiel" },
            "readme": { "path": "ADDON.md", "size": MAX_README, "sha256": "0".repeat(64) },
            "preview": { "path": "docs/preview.png", "size": MAX_PREVIEW, "sha256": "0".repeat(64) }
        });
        entry["docs"] = docs.clone();
        assert!(signed(&index).unwrap().addons[0].docs.is_some());

        for path in [
            "../ADDON.md",
            "/ADDON.md",
            "a/../b",
            "a/./b",
            "a//b",
            "a/b/",
            "a\\b",
            "a%2Fb",
            "a..b",
            "a?b",
        ] {
            index["addons"][0]["docs"]["readme"]["path"] = json!(path);
            assert!(matches!(signed(&index), Err(Problem::Invalid)), "{path}");
        }
        for (field, value) in [
            ("commit", json!("A".repeat(40))),
            ("summary", json!({ "en": "Bad\nsummary", "de": "Beispiel" })),
            (
                "summary",
                json!({ "en": "a".repeat(161), "de": "Beispiel" }),
            ),
            ("preview", Value::Null),
            ("extra", json!(true)),
        ] {
            index["addons"][0]["docs"] = docs.clone();
            index["addons"][0]["docs"][field] = value;
            assert!(matches!(signed(&index), Err(Problem::Invalid)), "{field}");
        }
        index["addons"][0]["docs"] = docs;
        index["addons"][0]["docs"]["preview"]["size"] = json!(MAX_PREVIEW + 1);
        assert!(matches!(signed(&index), Err(Problem::Invalid)));
        index["addons"][0]["docs"] = Value::Null;
        assert!(matches!(signed(&index), Err(Problem::Invalid)));
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
            runtime: None,
            floor: Floor::from(&index),
            index,
            cached: false,
            last_attempt: None,
            last_check: None,
        };
        let addon = &catalog::GOODNOTES;
        // Use a fixed check time so the test remains useful after the bundled index expires.
        assert_eq!(
            registry
                .release_at(addon, Some("0.1.9"), false, at("2026-11-22T00:00:00Z"))
                .unwrap()
                .version,
            "0.1.10"
        );
        assert_eq!(
            registry
                .release_at(addon, Some("0.1.10"), true, at("2026-11-22T00:00:00Z"))
                .unwrap()
                .version,
            "0.1.10"
        );
        assert!(
            registry
                .release_at(addon, Some("0.1.10"), false, at("2026-11-22T00:00:00Z"))
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
                    .perform(&path, mode, false, (|| start, &KEY), || panic!(
                        "disabled fetch"
                    ))
                    .unwrap()
                    .is_none()
            );
        }
        let snapshot = || Ok((SNAPSHOT.to_vec(), SIGNATURE.to_vec()));
        assert!(
            checks
                .perform(&path, Check::Manual, false, (|| start, &KEY), snapshot)
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
                        (|| start + Duration::from_secs(seconds), &KEY),
                        || panic!("throttled fetch")
                    )
                    .unwrap()
                    .is_none()
            );
        }
        let offline = || Err(Problem::Offline);
        assert!(
            restarted
                .perform(
                    &path,
                    Check::Manager,
                    true,
                    (|| start + Duration::from_secs(3_600), &KEY),
                    offline
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), cached);
        assert_eq!(load(&path, &KEY).unwrap().last_check, Some(start.as_secs()));
        let restarted = Checks::default();
        let previous_attempt = start + Duration::from_secs(3_600);
        assert!(
            restarted
                .perform(
                    &path,
                    Check::Startup,
                    true,
                    (|| previous_attempt + Duration::from_secs(DAY - 1), &KEY),
                    || panic!("failed attempt should throttle after restarting")
                )
                .unwrap()
                .is_none()
        );

        let corrected_clock = previous_attempt - Duration::from_secs(DAY + 1);
        assert!(
            restarted
                .perform(
                    &path,
                    Check::Startup,
                    true,
                    (|| corrected_clock, &KEY),
                    snapshot
                )
                .unwrap()
                .is_none()
        );
        let state = load(&path, &KEY).unwrap();
        assert_eq!(state.last_attempt, Some(corrected_clock.as_secs()));
        assert_eq!(state.last_check, Some(corrected_clock.as_secs()));
        assert_eq!(fs::read(&path).unwrap(), cached);
    }

    #[test]
    fn damaged_cache_recovers_without_lowering_the_floor() {
        let local = tempfile::tempdir().unwrap();
        let path = local.path().join("registry.json");
        let signing = SigningKey::from_bytes(&[19; 32]);
        let verifying = signing.verifying_key();
        let key = verifying.as_bytes();
        let start = at("2026-11-22T00:00:00Z");
        let make = |sequence| {
            let mut value = fixture();
            value["sequence"] = json!(sequence);
            serde_json::to_vec(&value).unwrap()
        };
        let download = |bytes: Vec<u8>| {
            let sig = signing.sign(&bytes).to_bytes().to_vec();
            Ok((bytes, sig))
        };
        let checks = Checks::default();
        checks
            .perform(&path, Check::Manual, false, (|| start, key), || {
                download(make(5))
            })
            .unwrap();
        let good_cache = fs::read(&path).unwrap();
        let mut invalid_signature: Value = serde_json::from_slice(&good_cache).unwrap();
        invalid_signature["signature"] = json!(STANDARD.encode([0; 64]));
        let mut mismatch: Value = serde_json::from_slice(&good_cache).unwrap();
        mismatch["sequence"] = json!(6);
        for (mode, damage) in [
            (Check::Startup, Some(b"{".to_vec())),
            (
                Check::Manual,
                Some(serde_json::to_vec(&invalid_signature).unwrap()),
            ),
            (Check::Manual, Some(serde_json::to_vec(&mismatch).unwrap())),
            (Check::Manual, None),
        ] {
            if let Some(bytes) = damage {
                fs::write(&path, bytes).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
            fs::write(path.with_extension("state.json"), b"{").unwrap();
            let state = load(&path, key).unwrap();
            assert_eq!(state.index.sequence, 4);
            assert_eq!(state.floor.sequence, 5);
            let later = || start + Duration::from_secs(DAY);
            assert!(matches!(
                checks.perform(&path, mode, true, (later, key), || download(make(4))),
                Err(Problem::Rollback)
            ));
            let mut changed = make(5);
            changed.push(b' ');
            assert!(matches!(
                checks.perform(&path, Check::Manual, true, (later, key), || download(
                    changed
                )),
                Err(Problem::Rollback)
            ));
            assert!(
                checks
                    .perform(&path, Check::Manual, false, (later, key), || download(
                        make(5)
                    ))
                    .unwrap()
                    .is_some()
            );
            assert_eq!(load(&path, key).unwrap().index.sequence, 5);
        }
    }

    #[test]
    fn registry_ipc_errors_are_closed_codes_without_urls() {
        for (problem, code) in [
            (Problem::Offline, "offline"),
            (Problem::Status, "status"),
            (Problem::Signature, "signature"),
            (Problem::Expired, "expired"),
            (Problem::Rollback, "rollback"),
            (
                Problem::from("https://example.org/?token=value".to_string()),
                "invalid",
            ),
        ] {
            let serialized = serde_json::to_string(&problem).unwrap();
            assert_eq!(serialized, format!("\"{code}\""));
            assert!(!serialized.contains("http"));
        }
    }

    #[test]
    fn signature_comes_from_the_release_that_served_the_index() {
        assert_eq!(
            release_tag(&format!("{RELEASES}/index-2/index.json")).as_deref(),
            Some("index-2")
        );
        for url in [
            format!("{RELEASES}/index-2/index.json.sig"),
            format!("{RELEASES}/v1/index.json"),
            format!("{RELEASES}/index-/index.json"),
            format!("{RELEASES}/index-2x/index.json"),
            format!("{RELEASES}/index-2/../index.json"),
            "https://github.com/Other/Repo/releases/download/index-2/index.json".to_owned(),
            "https://release-assets.githubusercontent.com/index-2/index.json".to_owned(),
        ] {
            assert_eq!(release_tag(&url), None, "{url}");
        }
        // The tagged signature URL is itself a permitted initial URL with asset-host redirects.
        let signature = format!("{RELEASES}/index-2/index.json.sig");
        assert!(registry_url_allowed(&signature, &signature, false));
        assert!(registry_url_allowed(
            &signature,
            "https://release-assets.githubusercontent.com/path?token=value",
            true
        ));
    }

    #[test]
    fn registry_redirects_stay_with_the_named_asset_or_github_asset_hosts() {
        let tagged = "https://github.com/goodtype-notes/Goodtype-Addons/releases/download/index-1/index.json";
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
        let index = verify(&bytes, &signature, &KEY).unwrap();
        accept(&index, &Floor::from(&snapshot), now()).unwrap();
        println!(
            "Verified registry sequence {} with {} catalog addons, expiring {}.",
            index.sequence,
            index.addons.len(),
            index.expires
        );
    }
}
