use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use sha2::{Digest, Sha256};

use crate::platform::{self, WebReply};

pub const PROGRAM: &str = "Parrotfish.exe";
pub const HELPER: &str = "update.exe";
pub const PAGE: &str = "https://github.com/littlephish/Parrotfish/releases/latest";
const REPOSITORY: &str = "https://github.com/littlephish/Parrotfish";
const SUMS: &str = "SHA256SUMS.txt";
const STAGING: &str = "update";
const UNPACKED: &str = "unpacked";
const NEWEST_NOTE: &str = "latest.txt";
const CARRIED_ALONG: &str = "unins000.msg";
const AGENT: &str = concat!("Parrotfish/", env!("CARGO_PKG_VERSION"));
const MAX_HOPS: usize = 5;
const MAX_NOTE: usize = 64 * 1024;
const MAX_ARCHIVE: usize = 300 * 1024 * 1024;
const MAX_UNPACKED: usize = 600 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_NAME: usize = 80;
const OWN_FILES: [&str; 9] = [
    "parrotfish.exe",
    "update.exe",
    "readme.md",
    "third-party-notices.txt",
    "unins000.exe",
    "unins000.dat",
    "unins000.msg",
    "update-log.txt",
    "phishspeak.exe",
];
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3",
    "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut number = || -> Option<u32> {
            let part = parts.next()?;
            let plain = !part.is_empty() && part.len() <= 5 && part.bytes().all(|b| b.is_ascii_digit());
            if plain {
                part.parse().ok()
            } else {
                None
            }
        };
        let version = Version(number()?, number()?, number()?);
        parts.next().is_none().then_some(version)
    }

    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}.{}.{}", self.0, self.1, self.2)
    }
}

pub trait Web {
    fn get(&self, url: &str, limit: usize, progress: &mut dyn FnMut(usize, Option<u64>)) -> Result<WebReply, String>;
}

pub struct Internet;

impl Web for Internet {
    fn get(&self, url: &str, limit: usize, progress: &mut dyn FnMut(usize, Option<u64>)) -> Result<WebReply, String> {
        platform::web_get(url, AGENT, limit, progress)
    }
}

pub fn version_in(location: &str) -> Option<Version> {
    let path = location.strip_prefix(REPOSITORY).or_else(|| location.strip_prefix("/littlephish/Parrotfish"))?;
    Version::parse(path.strip_prefix("/releases/tag/v")?)
}

pub fn archive_name(version: Version) -> String {
    format!("Parrotfish-{version}-windows-x64.zip")
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let plain = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    plain.then_some(host)
}

pub fn trusted(url: &str) -> bool {
    host_of(url).is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host == "github.com" || host.ends_with(".githubusercontent.com")
    })
}

fn onward(from: &str, location: &str) -> String {
    match (location.starts_with('/'), host_of(from)) {
        (true, Some(host)) => format!("https://{host}{location}"),
        _ => location.to_string(),
    }
}

pub fn fetch(
    web: &dyn Web,
    url: &str,
    limit: usize,
    progress: &mut dyn FnMut(usize, Option<u64>),
) -> Result<Vec<u8>, String> {
    let mut url = url.to_string();
    for _ in 0..=MAX_HOPS {
        if !trusted(&url) {
            return Err("the download was sent to a place this program does not trust".to_string());
        }
        let reply = web.get(&url, limit, progress)?;
        match reply.status {
            200 => return Ok(reply.body),
            301 | 302 | 303 | 307 | 308 if !reply.location.is_empty() => url = onward(&url, &reply.location),
            404 => return Err("GitHub does not have that file".to_string()),
            status => return Err(format!("GitHub answered with status {status}")),
        }
    }
    Err("the download was passed on too many times".to_string())
}

pub fn newest(web: &dyn Web) -> Result<Version, String> {
    let reply = web.get(PAGE, MAX_NOTE, &mut |_, _| {})?;
    match reply.status {
        301 | 302 | 303 | 307 | 308 => {
            version_in(&reply.location).ok_or_else(|| "GitHub did not name a release this program can read".to_string())
        }
        404 => Err("no release has been published".to_string()),
        status => Err(format!("GitHub answered with status {status}")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    GitHub,
    Folder(PathBuf),
}

impl Source {
    pub fn chosen(folder: Option<std::ffi::OsString>) -> Self {
        match folder {
            Some(folder) if !folder.is_empty() => Source::Folder(PathBuf::from(folder)),
            _ => Source::GitHub,
        }
    }

    pub fn newest(&self, web: &dyn Web) -> Result<Version, String> {
        match self {
            Source::GitHub => newest(web),
            Source::Folder(folder) => {
                let text = fs::read_to_string(folder.join(NEWEST_NOTE)).map_err(|e| format!("{NEWEST_NOTE} could not be read: {e}"))?;
                let text = text.trim();
                Version::parse(text.strip_prefix('v').unwrap_or(text)).ok_or_else(|| format!("{NEWEST_NOTE} does not name a version"))
            }
        }
    }

    fn file(
        &self,
        web: &dyn Web,
        version: Version,
        name: &str,
        limit: usize,
        progress: &mut dyn FnMut(usize, Option<u64>),
    ) -> Result<Vec<u8>, String> {
        match self {
            Source::GitHub => fetch(web, &format!("{REPOSITORY}/releases/download/v{version}/{name}"), limit, progress),
            Source::Folder(folder) => {
                let data = fs::read(folder.join(name)).map_err(|e| format!("{name} could not be read: {e}"))?;
                if data.len() > limit {
                    return Err("the file is larger than it should be".to_string());
                }
                progress(data.len(), Some(data.len() as u64));
                Ok(data)
            }
        }
    }
}

pub fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, listed) = line.trim().split_once(' ')?;
        let listed = listed.trim_start_matches([' ', '*']);
        let hex = sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit());
        (hex && listed == name).then(|| sum.to_ascii_lowercase())
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn checked_archive(
    source: &Source,
    web: &dyn Web,
    version: Version,
    progress: &mut dyn FnMut(usize, Option<u64>),
) -> Result<Vec<u8>, String> {
    let name = archive_name(version);
    let sums = source.file(web, version, SUMS, MAX_NOTE, &mut |_, _| {})?;
    let sums = String::from_utf8(sums).map_err(|_| "the release's list of checksums is not text".to_string())?;
    let wanted = sum_for(&sums, &name).ok_or_else(|| format!("the release lists no checksum for {name}"))?;
    let archive = source.file(web, version, &name, MAX_ARCHIVE, progress)?;
    if sha256_hex(&archive) != wanted {
        return Err("the download does not match the checksum published with the release, so it was thrown away".to_string());
    }
    Ok(archive)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    let pair = bytes.get(at..at.checked_add(2)?)?;
    Some(usize::from(u16::from_le_bytes([pair[0], pair[1]])))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let four = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
}

pub fn plain_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= MAX_NAME
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b' '))
        && !matches!(bytes[0], b'.' | b' ')
        && !matches!(bytes[bytes.len() - 1], b'.' | b' ')
        && !DEVICE_NAMES.contains(&stem.as_str())
}

fn end_record(archive: &[u8]) -> Option<usize> {
    let last = archive.len().checked_sub(22)?;
    let first = last.saturating_sub(65_535);
    (first..=last).rev().find(|at| {
        archive[*at..*at + 4] == [0x50, 0x4b, 0x05, 0x06] && u16_at(archive, at + 20).is_some_and(|comment| at + 22 + comment == archive.len())
    })
}

pub fn unpack(archive: &[u8]) -> Result<Vec<Entry>, String> {
    let broken = || "the download is not a zip this program can read".to_string();
    let end = end_record(archive).ok_or_else(broken)?;
    let count = u16_at(archive, end + 10).ok_or_else(broken)?;
    let mut at = u32_at(archive, end + 16).ok_or_else(broken)? as usize;
    if count == 0 || count > MAX_ENTRIES || count == 0xffff || at == 0xffff_ffff {
        return Err(broken());
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if u32_at(archive, at) != Some(0x0201_4b50) {
            return Err(broken());
        }
        let flags = u16_at(archive, at + 8).ok_or_else(broken)?;
        let method = u16_at(archive, at + 10).ok_or_else(broken)?;
        let crc = u32_at(archive, at + 16).ok_or_else(broken)?;
        let packed = u32_at(archive, at + 20).ok_or_else(broken)? as usize;
        let size = u32_at(archive, at + 24).ok_or_else(broken)? as usize;
        let name_length = u16_at(archive, at + 28).ok_or_else(broken)?;
        let extra_length = u16_at(archive, at + 30).ok_or_else(broken)?;
        let comment_length = u16_at(archive, at + 32).ok_or_else(broken)?;
        let local = u32_at(archive, at + 42).ok_or_else(broken)? as usize;
        let name = archive.get(at + 46..at + 46 + name_length).ok_or_else(broken)?;
        let name = std::str::from_utf8(name).map_err(|_| broken())?.to_string();
        at += 46 + name_length + extra_length + comment_length;
        if flags & 1 != 0 || !matches!(method, 0 | 8) {
            return Err(broken());
        }
        if !plain_name(&name) || entries.iter().any(|known| known.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("the download holds something this program will not unpack ({name:?})"));
        }
        total = total.saturating_add(size);
        if total > MAX_UNPACKED {
            return Err("the download unpacks to more than it should".to_string());
        }
        if u32_at(archive, local) != Some(0x0403_4b50) {
            return Err(broken());
        }
        let start = local + 30 + u16_at(archive, local + 26).ok_or_else(broken)? + u16_at(archive, local + 28).ok_or_else(broken)?;
        let stored = archive.get(start..start.checked_add(packed).ok_or_else(broken)?).ok_or_else(broken)?;
        let data = if method == 0 {
            stored.to_vec()
        } else {
            miniz_oxide::inflate::decompress_to_vec_with_limit(stored, size).map_err(|_| format!("{name} in the download is damaged"))?
        };
        if data.len() != size || crc32fast::hash(&data) != crc {
            return Err(format!("{name} in the download is damaged"));
        }
        entries.push(Entry { name, data });
    }
    if !entries.iter().any(|entry| entry.name.eq_ignore_ascii_case(PROGRAM)) {
        return Err(format!("the download does not hold {PROGRAM}"));
    }
    Ok(entries)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Ready,
    OtherName(String),
    Shared(String),
}

pub fn own_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let stamped = |stamp: &str| stamp.len() > 1 && stamp.starts_with('-') && stamp[1..].bytes().all(|b| b.is_ascii_digit());
    let base = match name.rsplit_once(".old") {
        Some((base, stamp)) if stamp.is_empty() || stamped(stamp) => base,
        _ => name.as_str(),
    };
    OWN_FILES.contains(&base)
}

pub fn stranger(listing: &[(String, bool)]) -> Option<String> {
    listing
        .iter()
        .find(|(name, folder)| if *folder { false } else { !own_file(name) })
        .map(|(name, _)| name.clone())
}

fn listing(folder: &Path) -> Option<Vec<(String, bool)>> {
    let read = fs::read_dir(folder).ok()?;
    Some(read.flatten().map(|entry| (entry.file_name().to_string_lossy().to_string(), entry.path().is_dir())).collect())
}

pub fn place(program: &Path) -> Place {
    let name = program.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    if !name.eq_ignore_ascii_case(PROGRAM) {
        return Place::OtherName(name);
    }
    let Some(found) = program.parent().and_then(listing) else {
        return Place::Shared(String::new());
    };
    if let Some(name) = stranger(&found) {
        return Place::Shared(name);
    }
    let staging = program.parent().map(|folder| folder.join(STAGING)).filter(|staging| staging.is_dir());
    let ours = staging.and_then(|staging| listing(&staging)).unwrap_or_default();
    if ours.iter().any(|(name, folder)| !folder || !name.eq_ignore_ascii_case(UNPACKED)) {
        return Place::Shared(STAGING.to_string());
    }
    Place::Ready
}

pub fn stage(folder: &Path, entries: &[Entry]) -> Result<PathBuf, String> {
    let unpacked = folder.join(STAGING).join(UNPACKED);
    let cannot = |what: &str, problem: std::io::Error| format!("the program's folder could not be written to ({what}: {problem})");
    if unpacked.exists() {
        fs::remove_dir_all(&unpacked).map_err(|e| cannot("clearing an earlier download", e))?;
    }
    fs::create_dir_all(&unpacked).map_err(|e| cannot("making room for the download", e))?;
    for entry in entries {
        fs::write(unpacked.join(&entry.name), &entry.data).map_err(|e| cannot(&entry.name, e))?;
    }
    let carried = folder.join(CARRIED_ALONG);
    if carried.is_file() && !entries.iter().any(|entry| entry.name.eq_ignore_ascii_case(CARRIED_ALONG)) {
        fs::copy(&carried, unpacked.join(CARRIED_ALONG)).map_err(|e| cannot(CARRIED_ALONG, e))?;
    }
    Ok(unpacked)
}

fn seat_for(helper: &Path) -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    for name in ["Parrotfish-updater".to_string(), format!("Parrotfish-updater-{}", std::process::id())] {
        let seat = base.join(name).join(HELPER);
        let made = seat.parent().is_some_and(|folder| fs::create_dir_all(folder).is_ok());
        if made && fs::copy(helper, &seat).is_ok() {
            return Ok(seat);
        }
    }
    Err(format!("{HELPER} could not be copied to the temporary folder"))
}

#[cfg(windows)]
fn launch(seat: &Path, unpacked: &Path, folder: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const DETACHED: u32 = 0x0000_0008;
    const OWN_GROUP: u32 = 0x0000_0200;
    const OUT_OF_THE_JOB: u32 = 0x0100_0000;
    [DETACHED | OWN_GROUP | OUT_OF_THE_JOB, DETACHED | OWN_GROUP].into_iter().any(|flags| {
        let mut helper = Command::new(seat);
        helper.arg(unpacked).arg(folder).arg(PROGRAM).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(home) = seat.parent() {
            helper.current_dir(home);
        }
        helper.creation_flags(flags).spawn().is_ok()
    })
}

#[cfg(not(windows))]
fn launch(_seat: &Path, _unpacked: &Path, _folder: &Path) -> bool {
    false
}

pub fn helper_for(folder: &Path, unpacked: &Path) -> Option<PathBuf> {
    [unpacked.join(HELPER), folder.join(HELPER)].into_iter().find(|path| path.is_file())
}

pub fn hand_over(folder: &Path, unpacked: &Path) -> Result<(), String> {
    let helper = helper_for(folder, unpacked).ok_or_else(|| format!("{HELPER} is missing, so this copy cannot replace itself"))?;
    let seat = seat_for(&helper)?;
    if launch(&seat, unpacked, folder) {
        Ok(())
    } else {
        Err(format!("Windows did not start {HELPER}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Unknown,
    Looking,
    Unreachable(String),
    Newest,
    Found(Version),
    Bringing(Version, u8),
    Restarting,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    Nothing,
    Install,
    Page,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wording {
    pub banner: String,
    pub about: String,
    pub offer: Offer,
}

pub fn wording(step: &Step, place: &Place) -> Wording {
    let quiet = |about: &str| Wording { banner: String::new(), about: about.to_string(), offer: Offer::Nothing };
    let both = |text: String, offer: Offer| Wording { banner: text.clone(), about: text, offer };
    match step {
        Step::Unknown => quiet(""),
        Step::Looking => quiet("Looking for a newer version\u{2026}"),
        Step::Unreachable(problem) => quiet(&format!("Could not look for a newer version: {problem}.")),
        Step::Newest => quiet("You have the newest version."),
        Step::Found(version) => match place {
            Place::Ready => both(format!("Parrotfish {version} is available. Updating restarts Parrotfish."), Offer::Install),
            Place::OtherName(name) => both(
                format!("Parrotfish {version} is available. This copy is called {name}, so it will not replace itself."),
                Offer::Page,
            ),
            Place::Shared(name) if name.is_empty() => both(
                format!("Parrotfish {version} is available. This copy could not look at its own folder, so it will not replace itself."),
                Offer::Page,
            ),
            Place::Shared(name) => both(
                format!(
                    "Parrotfish {version} is available. This copy shares its folder with other things ({name}), so it will not replace itself there."
                ),
                Offer::Page,
            ),
        },
        Step::Bringing(version, percent) => both(format!("Getting Parrotfish {version}: {percent} %"), Offer::Nothing),
        Step::Restarting => both("Restarting to finish the update.".to_string(), Offer::Nothing),
        Step::Failed(problem) => both(format!("The update did not work: {problem}. Nothing was changed."), Offer::Page),
    }
}

pub fn after_restart(from: &str) -> Option<String> {
    let before = Version::parse(from)?;
    let now = Version::current();
    Some(if now > before {
        format!("Parrotfish was updated from {before} to {now}.")
    } else {
        "The update did not go through. The file update-log.txt in the program's folder says what happened.".to_string()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Newest(Version),
    Fetched(u8),
    Staged(PathBuf),
    Failed(String),
}

pub fn look(source: Source) -> Receiver<Report> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().name("ps-update".into()).spawn(move || {
        let report = match source.newest(&Internet) {
            Ok(version) => Report::Newest(version),
            Err(problem) => Report::Failed(problem),
        };
        let _ = tx.send(report);
    });
    rx
}

pub fn percent(so_far: usize, of: Option<u64>) -> Option<u8> {
    let of = of.filter(|of| *of > 0)?;
    Some(((so_far as u64).saturating_mul(100) / of).min(100) as u8)
}

pub fn bring(source: Source, version: Version, folder: PathBuf) -> Receiver<Report> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().name("ps-update".into()).spawn(move || {
        let mut shown = None;
        let mut progress = |so_far: usize, of: Option<u64>| {
            let now = percent(so_far, of);
            if now.is_some() && now != shown {
                shown = now;
                let _ = tx.send(Report::Fetched(now.unwrap_or(0)));
            }
        };
        let staged = checked_archive(&source, &Internet, version, &mut progress)
            .and_then(|archive| unpack(&archive))
            .and_then(|entries| stage(&folder, &entries));
        let _ = tx.send(match staged {
            Ok(unpacked) => Report::Staged(unpacked),
            Err(problem) => Report::Failed(problem),
        });
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[test]
    fn versions_are_read_and_compared_by_their_numbers() {
        assert_eq!(Version::parse("0.5.1"), Some(Version(0, 5, 1)));
        assert_eq!(Version::parse("12.0.340").map(|version| version.to_string()).as_deref(), Some("12.0.340"));
        for odd in ["", "1", "1.2", "1.2.3.4", "v1.2.3", "1.2.x", "1..3", "1.2.3 ", "-1.2.3", "1.2.3-rc1", "1.2.999999"] {
            assert_eq!(Version::parse(odd), None, "{odd:?}");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert!(Version(0, 5, 1) > Version(0, 5, 0));
        assert_eq!(Version(0, 5, 0).cmp(&Version(0, 5, 0)), std::cmp::Ordering::Equal);
        assert!(Version::current() > Version(0, 0, 0), "the program knows its own version");
    }

    #[test]
    fn the_newest_release_is_read_from_where_github_points() {
        assert_eq!(version_in("https://github.com/littlephish/Parrotfish/releases/tag/v0.6.2"), Some(Version(0, 6, 2)));
        assert_eq!(version_in("/littlephish/Parrotfish/releases/tag/v1.0.0"), Some(Version(1, 0, 0)));
        for elsewhere in [
            "https://github.com/littlephish/Parrotfish/releases",
            "https://github.com/littlephish/Parrotfish/releases/tag/0.6.2",
            "https://github.com/littlephish/Parrotfish/releases/tag/v0.6.2/extra",
            "https://github.com/littlephish/Parrotfish-nightly/releases/tag/v9.9.9",
            "https://github.com/someone/Parrotfish/releases/tag/v9.9.9",
            "https://example.org/littlephish/Parrotfish/releases/tag/v9.9.9",
            "http://github.com/littlephish/Parrotfish/releases/tag/v9.9.9",
            "",
        ] {
            assert_eq!(version_in(elsewhere), None, "{elsewhere:?}");
        }
        assert_eq!(archive_name(Version(0, 6, 2)), "Parrotfish-0.6.2-windows-x64.zip");
    }

    #[test]
    fn only_githubs_own_hosts_are_trusted_and_only_encrypted() {
        for good in [
            "https://github.com/littlephish/Parrotfish/releases/latest",
            "https://GitHub.com/x",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2?sig=3",
            "https://objects.githubusercontent.com/a",
        ] {
            assert!(trusted(good), "{good}");
        }
        for bad in [
            "http://github.com/littlephish/Parrotfish",
            "https://github.com.evil.example/x",
            "https://evilgithub.com/x",
            "https://githubusercontent.com.evil.example/x",
            "https://notgithubusercontent.com/x",
            "https://github.com@evil.example/x",
            "https://github.com:8443/x",
            "https://evil.example/github.com",
            "ftp://github.com/x",
            "",
        ] {
            assert!(!trusted(bad), "{bad}");
        }
    }

    struct Scripted {
        answers: HashMap<String, WebReply>,
        asked: RefCell<Vec<String>>,
    }

    impl Scripted {
        fn new(answers: &[(&str, u32, &str, &[u8])]) -> Self {
            let answers = answers
                .iter()
                .map(|(url, status, location, body)| {
                    let reply = WebReply { status: *status, location: location.to_string(), length: None, body: body.to_vec() };
                    (url.to_string(), reply)
                })
                .collect();
            Self { answers, asked: RefCell::new(Vec::new()) }
        }
    }

    impl Web for Scripted {
        fn get(&self, url: &str, limit: usize, progress: &mut dyn FnMut(usize, Option<u64>)) -> Result<WebReply, String> {
            self.asked.borrow_mut().push(url.to_string());
            let reply = self.answers.get(url).cloned().ok_or_else(|| "the connection could not be made, or was cut".to_string())?;
            if reply.body.len() > limit {
                return Err("the file is larger than it should be".to_string());
            }
            if reply.status == 200 {
                progress(reply.body.len(), Some(reply.body.len() as u64));
            }
            Ok(reply)
        }
    }

    const FILE: &str = "https://github.com/littlephish/Parrotfish/releases/download/v0.6.2/SHA256SUMS.txt";
    const STORE: &str = "https://release-assets.githubusercontent.com/asset/77?sig=abc";

    #[test]
    fn a_download_is_followed_to_githubs_file_store_and_nowhere_else() {
        let web = Scripted::new(&[(FILE, 302, STORE, b""), (STORE, 200, "", b"the file")]);
        assert_eq!(fetch(&web, FILE, 100, &mut |_, _| {}), Ok(b"the file".to_vec()));
        assert_eq!(*web.asked.borrow(), vec![FILE.to_string(), STORE.to_string()]);

        let lured = Scripted::new(&[(FILE, 302, "https://evil.example/Parrotfish.zip", b""), ("https://evil.example/Parrotfish.zip", 200, "", b"bad")]);
        assert!(fetch(&lured, FILE, 100, &mut |_, _| {}).is_err());
        assert_eq!(*lured.asked.borrow(), vec![FILE.to_string()], "the untrusted place was never asked");

        let plain = Scripted::new(&[(FILE, 302, "http://github.com/x", b"")]);
        assert!(fetch(&plain, FILE, 100, &mut |_, _| {}).is_err(), "a hop to an unencrypted address was followed");

        let relative = Scripted::new(&[(FILE, 301, "/moved/here", b""), ("https://github.com/moved/here", 200, "", b"moved")]);
        assert_eq!(fetch(&relative, FILE, 100, &mut |_, _| {}), Ok(b"moved".to_vec()));

        let circle = Scripted::new(&[(FILE, 302, FILE, b"")]);
        assert_eq!(fetch(&circle, FILE, 100, &mut |_, _| {}), Err("the download was passed on too many times".to_string()));
        assert_eq!(circle.asked.borrow().len(), MAX_HOPS + 1);

        let gone = Scripted::new(&[(FILE, 404, "", b"")]);
        assert_eq!(fetch(&gone, FILE, 100, &mut |_, _| {}), Err("GitHub does not have that file".to_string()));
        let odd = Scripted::new(&[(FILE, 503, "", b"")]);
        assert_eq!(fetch(&odd, FILE, 100, &mut |_, _| {}), Err("GitHub answered with status 503".to_string()));
        let nowhere = Scripted::new(&[(FILE, 302, "", b"")]);
        assert!(fetch(&nowhere, FILE, 100, &mut |_, _| {}).is_err(), "a redirect without a place to go");
        assert!(fetch(&Scripted::new(&[]), "https://evil.example/x", 100, &mut |_, _| {}).is_err());
    }

    #[test]
    fn the_newest_version_is_asked_of_github_without_following_it() {
        let web = Scripted::new(&[(PAGE, 302, "https://github.com/littlephish/Parrotfish/releases/tag/v0.7.0", b"")]);
        assert_eq!(newest(&web), Ok(Version(0, 7, 0)));
        assert_eq!(web.asked.borrow().len(), 1);
        let none = Scripted::new(&[(PAGE, 302, "https://github.com/littlephish/Parrotfish/releases", b"")]);
        assert!(newest(&none).is_err());
        let missing = Scripted::new(&[(PAGE, 404, "", b"")]);
        assert_eq!(newest(&missing), Err("no release has been published".to_string()));
        let page = Scripted::new(&[(PAGE, 200, "", b"<html>")]);
        assert_eq!(newest(&page), Err("GitHub answered with status 200".to_string()));
        assert!(newest(&Scripted::new(&[])).is_err(), "offline");
        assert_eq!(Source::GitHub.newest(&web), Ok(Version(0, 7, 0)));
    }

    #[test]
    fn the_checksum_list_is_read_by_file_name() {
        let sums = format!("{}  Parrotfish-0.6.2-windows-x64.zip\n{} *Parrotfish-0.6.2-setup.exe\nshort  other.zip\n", "ab".repeat(32), "CD".repeat(32));
        assert_eq!(sum_for(&sums, "Parrotfish-0.6.2-windows-x64.zip"), Some("ab".repeat(32)));
        assert_eq!(sum_for(&sums, "Parrotfish-0.6.2-setup.exe"), Some("cd".repeat(32)));
        assert_eq!(sum_for(&sums, "other.zip"), None, "a sum of the wrong length");
        assert_eq!(sum_for(&sums, "windows-x64.zip"), None, "part of a name is not the name");
        assert_eq!(sum_for("", "x"), None);
        assert_eq!(sum_for(&format!("{}  x\n", "zz".repeat(32)), "x"), None);
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    fn le16(value: usize) -> [u8; 2] {
        (value as u16).to_le_bytes()
    }

    fn le32(value: usize) -> [u8; 4] {
        (value as u32).to_le_bytes()
    }

    fn zip_of(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, data, squeeze) in files {
            let packed = if *squeeze { miniz_oxide::deflate::compress_to_vec(data, 9) } else { data.to_vec() };
            let method = if *squeeze { 8 } else { 0 };
            let crc = crc32fast::hash(data);
            let offset = out.len();
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0]);
            out.extend_from_slice(&le16(method));
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&le32(packed.len()));
            out.extend_from_slice(&le32(data.len()));
            out.extend_from_slice(&le16(name.len()));
            out.extend_from_slice(&le16(0));
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);
            directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0]);
            directory.extend_from_slice(&le16(method));
            directory.extend_from_slice(&[0, 0, 0, 0]);
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&le32(packed.len()));
            directory.extend_from_slice(&le32(data.len()));
            directory.extend_from_slice(&le16(name.len()));
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&le32(offset));
            directory.extend_from_slice(name.as_bytes());
        }
        let start = out.len();
        out.extend_from_slice(&directory);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&le16(files.len()));
        out.extend_from_slice(&le16(files.len()));
        out.extend_from_slice(&le32(directory.len()));
        out.extend_from_slice(&le32(start));
        out.extend_from_slice(&le16(0));
        out
    }

    fn release() -> Vec<u8> {
        let program: Vec<u8> = (0..40_000u32).map(|n| (n % 251) as u8).collect();
        zip_of(&[
            ("Parrotfish.exe", &program, true),
            ("update.exe", b"the helper", true),
            ("README.md", b"# Parrotfish\n", false),
            ("THIRD-PARTY-NOTICES.txt", b"notices\n", true),
        ])
    }

    #[test]
    fn a_release_archive_unpacks_to_its_files() {
        let entries = unpack(&release()).expect("a good archive");
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Parrotfish.exe", "update.exe", "README.md", "THIRD-PARTY-NOTICES.txt"]);
        assert_eq!(entries[0].data.len(), 40_000);
        assert_eq!((entries[0].data[250], entries[0].data[251]), (250, 0));
        assert_eq!(entries[2].data, b"# Parrotfish\n");
        let commented = [release(), b"a comment at the end".to_vec()].concat();
        assert!(unpack(&commented).is_err(), "bytes after the end record are not a comment it declared");
    }

    #[test]
    fn what_pythons_zipfile_writes_is_read() {
        let made: Vec<u8> = PYTHON_ZIP.to_vec();
        let entries = unpack(&made).expect("the archive the release script would write");
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].name.as_str(), entries[0].data.as_slice()), ("Parrotfish.exe", &b"program bytes program bytes program bytes program bytes"[..]));
        assert_eq!((entries[1].name.as_str(), entries[1].data.as_slice()), ("README.md", &b"read me"[..]));
    }

    const PYTHON_ZIP: &[u8] = &[
        80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 0, 96, 73, 93, 71, 16, 155, 202, 19, 0,
        0, 0, 55, 0, 0, 0, 14, 0, 0, 0, 80, 97, 114, 114, 111, 116, 102, 105, 115, 104,
        46, 101, 120, 101, 43, 40, 202, 79, 47, 74, 204, 85, 72, 170, 44, 73, 45, 86, 40, 32,
        150, 7, 0, 80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 0, 96, 73, 93, 225, 120, 114,
        123, 9, 0, 0, 0, 7, 0, 0, 0, 9, 0, 0, 0, 82, 69, 65, 68, 77, 69, 46,
        109, 100, 43, 74, 77, 76, 81, 200, 77, 5, 0, 80, 75, 1, 2, 20, 0, 20, 0, 0,
        0, 8, 0, 0, 96, 73, 93, 71, 16, 155, 202, 19, 0, 0, 0, 55, 0, 0, 0, 14,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0, 80, 97, 114,
        114, 111, 116, 102, 105, 115, 104, 46, 101, 120, 101, 80, 75, 1, 2, 20, 0, 20, 0, 0,
        0, 8, 0, 0, 96, 73, 93, 225, 120, 114, 123, 9, 0, 0, 0, 7, 0, 0, 0, 9,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 63, 0, 0, 0, 82, 69, 65,
        68, 77, 69, 46, 109, 100, 80, 75, 5, 6, 0, 0, 0, 0, 2, 0, 2, 0, 115, 0,
        0, 0, 111, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn a_damaged_or_odd_archive_is_refused_whole() {
        let good = release();
        assert!(unpack(&good).is_ok());
        for cut in [0, 10, 21, good.len() / 2, good.len() - 1] {
            assert!(unpack(&good[..cut]).is_err(), "cut to {cut} bytes");
        }
        let mut flipped = good.clone();
        flipped[60] ^= 0xff;
        assert_eq!(unpack(&flipped), Err("Parrotfish.exe in the download is damaged".to_string()));
        assert!(unpack(b"<html>404 Not Found</html>").is_err());
        assert!(unpack(&[]).is_err());

        let plain = zip_of(&[("Parrotfish.exe", b"program", false)]);
        let mut wrong_sum = plain.clone();
        let at = wrong_sum.windows(7).position(|part| part == b"program").unwrap();
        wrong_sum[at] = b'P';
        assert_eq!(unpack(&wrong_sum), Err("Parrotfish.exe in the download is damaged".to_string()), "stored, so only the checksum can tell");

        let mut locked = plain.clone();
        let directory = locked.windows(4).position(|part| part == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        locked[directory + 8] |= 1;
        assert!(unpack(&locked).is_err(), "an encrypted entry");
        let mut other_method = plain.clone();
        other_method[directory + 10] = 12;
        assert!(unpack(&other_method).is_err(), "a way of packing this program does not know");
        let mut swollen = zip_of(&[("Parrotfish.exe", &[7u8; 5000], true)]);
        let directory = swollen.windows(4).position(|part| part == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        swollen[directory + 24..directory + 28].copy_from_slice(&le32(100));
        assert!(unpack(&swollen).is_err(), "it unpacks to more than it says");

        assert_eq!(unpack(&zip_of(&[("README.md", b"only words", false)])), Err("the download does not hold Parrotfish.exe".to_string()));
        assert!(unpack(&zip_of(&[])).is_err());
        let crowd: Vec<(String, Vec<u8>)> = (0..MAX_ENTRIES + 1).map(|n| (format!("file{n}.txt"), vec![1u8])).collect();
        let crowd: Vec<(&str, &[u8], bool)> = crowd.iter().map(|(name, data)| (name.as_str(), data.as_slice(), false)).collect();
        assert!(unpack(&zip_of(&crowd)).is_err(), "more entries than a release has");
    }

    #[test]
    fn nothing_is_unpacked_outside_the_folder_or_under_a_risky_name() {
        for name in [
            "../Parrotfish.exe",
            "..\\Parrotfish.exe",
            "folder/Parrotfish.exe",
            "folder\\thing.txt",
            "C:evil.exe",
            "/absolute.txt",
            "..",
            ".hidden",
            "trailing.",
            "trailing ",
            " leading",
            "nul",
            "NUL.txt",
            "com1.exe",
            "tab\tname",
            "caf\u{e9}.txt",
            "stream.txt:ads",
            "",
        ] {
            assert!(!plain_name(name), "{name:?}");
            let archive = zip_of(&[("Parrotfish.exe", b"program", false), (name, b"x", false)]);
            assert!(unpack(&archive).is_err(), "{name:?}");
        }
        let long = "a".repeat(MAX_NAME + 1);
        assert!(!plain_name(&long) && plain_name(&long[1..]));
        for name in ["Parrotfish.exe", "update.exe", "README.md", "THIRD-PARTY-NOTICES.txt", "new file-2_b.txt", "console.txt"] {
            assert!(plain_name(name), "{name:?}");
        }
        let twice = zip_of(&[("Parrotfish.exe", b"program", false), ("parrotfish.EXE", b"other", false)]);
        assert!(unpack(&twice).is_err(), "two entries that are one file on Windows");
    }

    #[test]
    fn a_download_is_only_used_when_it_matches_the_published_checksum() {
        let archive = release();
        let folder = scratch("source");
        let version = Version(9, 9, 9);
        fs::write(folder.join(archive_name(version)), &archive).unwrap();
        fs::write(folder.join(SUMS), format!("{}  {}\n", sha256_hex(&archive), archive_name(version))).unwrap();
        fs::write(folder.join(NEWEST_NOTE), "v9.9.9\n").unwrap();
        let source = Source::Folder(folder.clone());
        let nobody = Scripted::new(&[]);
        assert_eq!(source.newest(&nobody), Ok(version));
        let mut seen = Vec::new();
        assert_eq!(checked_archive(&source, &nobody, version, &mut |so_far, of| seen.push((so_far, of))), Ok(archive.clone()));
        assert_eq!(seen, vec![(archive.len(), Some(archive.len() as u64))], "only the archive counts as progress");
        assert!(nobody.asked.borrow().is_empty(), "a folder is read without the network");

        let mut tampered = archive.clone();
        tampered[100] ^= 1;
        fs::write(folder.join(archive_name(version)), &tampered).unwrap();
        let refused = checked_archive(&source, &nobody, version, &mut |_, _| {});
        assert!(refused.is_err_and(|problem| problem.contains("does not match the checksum")));

        fs::write(folder.join(SUMS), format!("{}  Parrotfish-9.9.9-setup.exe\n", sha256_hex(&tampered))).unwrap();
        assert_eq!(
            checked_archive(&source, &nobody, version, &mut |_, _| {}),
            Err("the release lists no checksum for Parrotfish-9.9.9-windows-x64.zip".to_string())
        );
        fs::remove_file(folder.join(SUMS)).unwrap();
        assert!(checked_archive(&source, &nobody, version, &mut |_, _| {}).is_err(), "no checksum list, no update");
        fs::write(folder.join(NEWEST_NOTE), "soon").unwrap();
        assert!(source.newest(&nobody).is_err());
        let _ = fs::remove_dir_all(&folder);

        let sums = format!("{}  {}\n", sha256_hex(&archive), archive_name(version));
        let (sums_url, archive_url) = (
            "https://github.com/littlephish/Parrotfish/releases/download/v9.9.9/SHA256SUMS.txt",
            "https://github.com/littlephish/Parrotfish/releases/download/v9.9.9/Parrotfish-9.9.9-windows-x64.zip",
        );
        let web = Scripted::new(&[(sums_url, 200, "", sums.as_bytes()), (archive_url, 302, STORE, b""), (STORE, 200, "", &archive)]);
        assert_eq!(checked_archive(&Source::GitHub, &web, version, &mut |_, _| {}), Ok(archive));
        assert_eq!(Source::chosen(None), Source::GitHub);
        assert_eq!(Source::chosen(Some("".into())), Source::GitHub);
        assert_eq!(Source::chosen(Some("D:\\release".into())), Source::Folder(PathBuf::from("D:\\release")));
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("parrotfish-update-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn only_the_programs_own_files_count_as_its_own() {
        for own in [
            "Parrotfish.exe",
            "PARROTFISH.EXE",
            "update.exe",
            "README.md",
            "THIRD-PARTY-NOTICES.txt",
            "unins000.exe",
            "unins000.dat",
            "unins000.msg",
            "update-log.txt",
            "PhishSpeak.exe",
            "Parrotfish.exe.old",
            "update.exe.old-1787363793",
        ] {
            assert!(own_file(own), "{own}");
        }
        for other in [
            "notes.txt",
            "Parrotfish.exe.bak",
            "Parrotfish.exe.older",
            "Parrotfish.exe.old-",
            "Parrotfish.exe.old-12a",
            "notes.old",
            "holiday.old-2019",
            "unins001.exe",
            "desktop.ini",
            "ps-app.exe",
            "",
        ] {
            assert!(!own_file(other), "{other}");
        }
        let file = |name: &str| (name.to_string(), false);
        let folder = |name: &str| (name.to_string(), true);
        assert_eq!(stranger(&[file("Parrotfish.exe"), file("update.exe"), folder("update"), file("unins000.dat")]), None);
        assert_eq!(stranger(&[file("Parrotfish.exe"), file("holiday.jpg")]), Some("holiday.jpg".to_string()));
        assert_eq!(stranger(&[file("Parrotfish.exe"), folder("Screenshots")]), Some("Screenshots".to_string()));
        assert_eq!(stranger(&[folder("Parrotfish.exe")]), Some("Parrotfish.exe".to_string()), "a folder named like the program");
        assert_eq!(stranger(&[file("update")]), Some("update".to_string()), "a file named like the staging folder");
        assert_eq!(stranger(&[]), None);
    }

    #[test]
    fn a_copy_only_replaces_itself_in_a_folder_of_its_own() {
        let folder = scratch("place");
        let program = folder.join("Parrotfish.exe");
        fs::write(&program, "program").unwrap();
        fs::write(folder.join("update.exe"), "helper").unwrap();
        fs::write(folder.join("README.md"), "read me").unwrap();
        assert_eq!(place(&program), Place::Ready);
        fs::create_dir_all(folder.join("update").join("unpacked")).unwrap();
        fs::write(folder.join("update").join("unpacked").join("anything.txt"), "left by an earlier try").unwrap();
        assert_eq!(place(&program), Place::Ready, "its own staging folder is not a stranger");
        fs::write(folder.join("update").join("mine.txt"), "somebody's file").unwrap();
        assert_eq!(place(&program), Place::Shared("update".to_string()), "a folder named update that holds something else");
        fs::remove_file(folder.join("update").join("mine.txt")).unwrap();

        fs::write(folder.join("holiday.jpg"), "a photo").unwrap();
        assert_eq!(place(&program), Place::Shared("holiday.jpg".to_string()));
        fs::remove_file(folder.join("holiday.jpg")).unwrap();
        fs::create_dir_all(folder.join("Screenshots")).unwrap();
        assert_eq!(place(&program), Place::Shared("Screenshots".to_string()));
        fs::remove_dir_all(folder.join("Screenshots")).unwrap();
        assert_eq!(place(&program), Place::Ready);

        let renamed = folder.join("ps-app.exe");
        fs::write(&renamed, "program").unwrap();
        assert_eq!(place(&renamed), Place::OtherName("ps-app.exe".to_string()));
        assert_eq!(place(&program), Place::Shared("ps-app.exe".to_string()));
        assert_eq!(place(&folder.join("gone").join("Parrotfish.exe")), Place::Shared(String::new()), "a folder that cannot be listed");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_new_files_are_laid_out_beside_the_program_for_the_helper() {
        let folder = scratch("stage");
        fs::write(folder.join("Parrotfish.exe"), "old program").unwrap();
        fs::write(folder.join("unins000.msg"), "the uninstaller's texts").unwrap();
        fs::create_dir_all(folder.join("update").join("unpacked")).unwrap();
        fs::write(folder.join("update").join("unpacked").join("stale.txt"), "from a try that failed").unwrap();
        let entries = unpack(&release()).unwrap();
        let unpacked = stage(&folder, &entries).expect("a folder that can be written to");
        assert_eq!(unpacked, folder.join("update").join("unpacked"));
        let mut laid_out: Vec<String> = fs::read_dir(&unpacked).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        laid_out.sort();
        assert_eq!(laid_out, ["Parrotfish.exe", "README.md", "THIRD-PARTY-NOTICES.txt", "unins000.msg", "update.exe"]);
        assert_eq!(fs::read(unpacked.join("Parrotfish.exe")).unwrap().len(), 40_000);
        assert_eq!(fs::read_to_string(unpacked.join("unins000.msg")).unwrap(), "the uninstaller's texts", "carried along so that it is kept");
        assert_eq!(fs::read_to_string(folder.join("Parrotfish.exe")).unwrap(), "old program", "the program itself is not touched yet");

        fs::remove_file(folder.join("unins000.msg")).unwrap();
        let again = stage(&folder, &entries).unwrap();
        assert!(!again.join("unins000.msg").exists() && !again.join("stale.txt").exists());

        assert_eq!(helper_for(&folder, &again), Some(again.join("update.exe")), "the helper that came with the new release is used");
        fs::write(folder.join("update.exe"), "the helper that was installed").unwrap();
        assert_eq!(helper_for(&folder, &again), Some(again.join("update.exe")), "also when an older one is installed");
        fs::remove_file(again.join("update.exe")).unwrap();
        assert_eq!(helper_for(&folder, &again), Some(folder.join("update.exe")), "a release without one falls back on the installed one");
        fs::remove_file(folder.join("update.exe")).unwrap();
        assert_eq!(helper_for(&folder, &again), None);
        assert!(hand_over(&folder, &again).is_err_and(|problem| problem.contains("update.exe is missing")));
        let blocked = folder.join("blocked");
        fs::write(&blocked, "a file where the folder should be").unwrap();
        assert!(stage(&blocked, &entries).is_err_and(|problem| problem.contains("could not be written to")));
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn what_is_shown_says_what_can_be_done_from_where_the_copy_sits() {
        let version = Version(0, 7, 0);
        let ready = wording(&Step::Found(version), &Place::Ready);
        assert_eq!(ready.banner, "Parrotfish 0.7.0 is available. Updating restarts Parrotfish.");
        assert_eq!((ready.about.as_str(), ready.offer), (ready.banner.as_str(), Offer::Install));
        let shared = wording(&Step::Found(version), &Place::Shared("holiday.jpg".to_string()));
        assert!(shared.banner.contains("holiday.jpg") && shared.banner.contains("will not replace itself"));
        assert_eq!(shared.offer, Offer::Page, "it is never offered where the helper would clear out somebody's files");
        assert_eq!(wording(&Step::Found(version), &Place::OtherName("ps-app.exe".to_string())).offer, Offer::Page);
        assert_eq!(wording(&Step::Found(version), &Place::Shared(String::new())).offer, Offer::Page);
        for silent in [Step::Unknown, Step::Looking, Step::Newest, Step::Unreachable("the server took too long to answer".to_string())] {
            let shown = wording(&silent, &Place::Ready);
            assert_eq!((shown.banner.as_str(), shown.offer), ("", Offer::Nothing), "{silent:?}");
        }
        assert_eq!(wording(&Step::Newest, &Place::Ready).about, "You have the newest version.");
        assert_eq!(
            wording(&Step::Unreachable("the server took too long to answer".to_string()), &Place::Ready).about,
            "Could not look for a newer version: the server took too long to answer."
        );
        let fetching = wording(&Step::Bringing(version, 42), &Place::Ready);
        assert_eq!((fetching.banner.as_str(), fetching.offer), ("Getting Parrotfish 0.7.0: 42 %", Offer::Nothing));
        assert_eq!(wording(&Step::Restarting, &Place::Ready).offer, Offer::Nothing);
        let failed = wording(&Step::Failed("GitHub does not have that file".to_string()), &Place::Ready);
        assert_eq!(failed.banner, "The update did not work: GitHub does not have that file. Nothing was changed.");
        assert_eq!(failed.offer, Offer::Page);
    }

    #[test]
    fn after_a_restart_the_program_says_whether_the_update_took() {
        let now = Version::current();
        let older = Version(now.0, now.1, now.2.wrapping_sub(1));
        if now.2 > 0 {
            assert_eq!(after_restart(&older.to_string()), Some(format!("Parrotfish was updated from {older} to {now}.")));
        }
        let unchanged = after_restart(&now.to_string()).expect("a version it can read");
        assert!(unchanged.starts_with("The update did not go through."), "{unchanged}");
        assert!(after_restart(&Version(now.0 + 1, 0, 0).to_string()).is_some_and(|text| text.starts_with("The update did not go through.")));
        assert_eq!(after_restart(""), None);
        assert_eq!(after_restart("soon"), None);
    }

    #[test]
    fn progress_is_a_percentage_when_the_size_is_known() {
        assert_eq!(percent(0, Some(200)), Some(0));
        assert_eq!(percent(50, Some(200)), Some(25));
        assert_eq!(percent(200, Some(200)), Some(100));
        assert_eq!(percent(999, Some(200)), Some(100));
        assert_eq!(percent(50, None), None);
        assert_eq!(percent(50, Some(0)), None);
    }
}
