use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

pub const MAX_ICON_BYTES: usize = 512 * 1024;
pub const MAX_ICON_SIDE: u32 = 256;
pub const SHOWN_SIDE: u32 = 32;
pub const RETRY_AFTER: Duration = Duration::from_secs(600);
pub const ASK_AGAIN_AFTER: Duration = Duration::from_secs(120);
const REFRESH_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_ICONS_PER_SERVER: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Bmp,
}

impl Format {
    fn extension(self) -> Option<&'static str> {
        match self {
            Format::Png => Some("png"),
            Format::Jpeg => Some("jpg"),
            Format::Gif | Format::Bmp => None,
        }
    }
}

const EXTENSIONS: [&str; 2] = ["png", "jpg"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Empty,
    TooManyBytes,
    NotAnImage,
    Unsupported,
    TooLarge,
    Broken,
}

impl Reject {
    pub fn reason(self) -> &'static str {
        match self {
            Reject::Empty => "the file is empty",
            Reject::TooManyBytes => "the file is too big",
            Reject::NotAnImage => "it is not a picture",
            Reject::Unsupported => "only PNG and JPEG pictures can be shown",
            Reject::TooLarge => "the picture is larger than 256 x 256",
            Reject::Broken => "the picture could not be read",
        }
    }
}

pub fn sniff(data: &[u8]) -> Option<(Format, u32, u32)> {
    if data.len() >= 24 && data.starts_with(b"\x89PNG\r\n\x1a\n") && &data[12..16] == b"IHDR" {
        let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((Format::Png, width, height));
    }
    if data.len() >= 10 && (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        let width = u32::from(u16::from_le_bytes([data[6], data[7]]));
        let height = u32::from(u16::from_le_bytes([data[8], data[9]]));
        return Some((Format::Gif, width, height));
    }
    if data.len() >= 26 && data.starts_with(b"BM") {
        let width = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
        let height = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
        return Some((Format::Bmp, width.unsigned_abs(), height.unsigned_abs()));
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        return jpeg_size(data).map(|(width, height)| (Format::Jpeg, width, height));
    }
    None
}

fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let marker = data[at + 1];
        if marker == 0xFF {
            at += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
        if length < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if at + 9 > data.len() {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([data[at + 5], data[at + 6]]));
            let width = u32::from(u16::from_be_bytes([data[at + 7], data[at + 8]]));
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

pub fn accept(data: &[u8]) -> Result<Format, Reject> {
    if data.is_empty() {
        return Err(Reject::Empty);
    }
    if data.len() > MAX_ICON_BYTES {
        return Err(Reject::TooManyBytes);
    }
    let (format, width, height) = sniff(data).ok_or(Reject::NotAnImage)?;
    if width == 0 || height == 0 || width > MAX_ICON_SIDE || height > MAX_ICON_SIDE {
        return Err(Reject::TooLarge);
    }
    match format {
        Format::Png | Format::Jpeg => Ok(format),
        Format::Gif | Format::Bmp => Err(Reject::Unsupported),
    }
}

pub fn server_folder(server_uid: &str) -> String {
    if server_uid.is_empty() {
        return "unknown".to_string();
    }
    server_uid.bytes().map(|byte| format!("{byte:02x}")).collect()
}

pub fn standard_icon(id: u32) -> Option<usize> {
    ps_client::STANDARD_ICONS.iter().position(|known| *known == id)
}

pub fn shrink(pixels: &[u8], width: u32, height: u32, side: u32) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h, side) = (width as usize, height as usize, side.max(1) as usize);
    if w == 0 || h == 0 || pixels.len() != w * h * 4 || (w <= side && h <= side) {
        return None;
    }
    let longest = w.max(h);
    let out_w = (w * side / longest).max(1);
    let out_h = (h * side / longest).max(1);
    let mut out = Vec::with_capacity(out_w * out_h * 4);
    for y in 0..out_h {
        let (top, bottom) = (y * h / out_h, ((y + 1) * h / out_h).max(y * h / out_h + 1));
        for x in 0..out_w {
            let (left, right) = (x * w / out_w, ((x + 1) * w / out_w).max(x * w / out_w + 1));
            let mut sum = [0u64; 4];
            for row in top..bottom {
                for column in left..right {
                    let at = (row * w + column) * 4;
                    let alpha = u64::from(pixels[at + 3]);
                    sum[0] += u64::from(pixels[at]) * alpha;
                    sum[1] += u64::from(pixels[at + 1]) * alpha;
                    sum[2] += u64::from(pixels[at + 2]) * alpha;
                    sum[3] += alpha;
                }
            }
            let count = ((bottom - top) * (right - left)) as u64;
            for channel in &sum[..3] {
                out.push(if sum[3] == 0 { 0 } else { ((channel + sum[3] / 2) / sum[3]) as u8 });
            }
            out.push(((sum[3] + count / 2) / count) as u8);
        }
    }
    Some((out, out_w as u32, out_h as u32))
}

fn decode(path: &Path) -> Option<Image> {
    let loaded = std::panic::catch_unwind(|| Image::load_from_path(path).ok()).ok().flatten()?;
    let buffer = loaded.to_rgba8()?;
    let (width, height) = (buffer.width(), buffer.height());
    if width == 0 || height == 0 || width > MAX_ICON_SIDE || height > MAX_ICON_SIDE {
        return None;
    }
    match shrink(buffer.as_bytes(), width, height, SHOWN_SIDE) {
        Some((pixels, width, height)) => {
            Some(Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&pixels, width, height)))
        }
        None => Some(loaded),
    }
}

fn file_name(id: u32, extension: &str) -> String {
    format!("icon_{id}.{extension}")
}

fn load(folder: &Path, id: u32) -> Option<(Image, bool)> {
    for extension in EXTENSIONS {
        let path = folder.join(file_name(id, extension));
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        if accept(&data).ok().and_then(Format::extension) != Some(extension) {
            continue;
        }
        let Some(image) = decode(&path) else {
            continue;
        };
        let age = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .unwrap_or_default();
        return Some((image, age > REFRESH_AFTER));
    }
    None
}

fn prune(folder: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut icons: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("icon_") {
            continue;
        }
        if name.ends_with(".part") {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let modified = entry.metadata().and_then(|meta| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        icons.push((modified, path));
    }
    if icons.len() <= keep {
        return;
    }
    icons.sort();
    for (_, path) in icons.iter().take(icons.len() - keep) {
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Clone)]
pub enum Lookup {
    Standard(usize),
    Ready(Image),
    Refresh(Image),
    Ask,
    Waiting,
    Nothing,
}

enum State {
    Fresh,
    Due,
    Asked(Instant),
    Failed(Instant),
    Refused,
}

struct Slot {
    image: Option<Image>,
    state: State,
}

#[derive(Default)]
struct Shelf {
    slots: HashMap<u32, Slot>,
}

pub struct IconStore {
    root: PathBuf,
    shelves: HashMap<String, Shelf>,
}

impl IconStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root, shelves: HashMap::new() }
    }

    fn folder(&self, server_uid: &str) -> PathBuf {
        self.root.join(server_folder(server_uid))
    }

    fn shelf(&mut self, server_uid: &str) -> &mut Shelf {
        if !self.shelves.contains_key(server_uid) {
            self.shelves.insert(server_uid.to_string(), Shelf::default());
        }
        self.shelves.get_mut(server_uid).expect("the shelf was just added")
    }

    pub fn lookup(&mut self, server_uid: &str, id: u32, now: Instant) -> Lookup {
        self.find(server_uid, id, now, true)
    }

    pub fn peek(&mut self, server_uid: &str, id: u32, now: Instant) -> Lookup {
        self.find(server_uid, id, now, false)
    }

    fn find(&mut self, server_uid: &str, id: u32, now: Instant, may_ask: bool) -> Lookup {
        if id == 0 {
            return Lookup::Nothing;
        }
        if let Some(index) = standard_icon(id) {
            return Lookup::Standard(index);
        }
        let folder = self.folder(server_uid);
        let shelf = self.shelf(server_uid);
        if !shelf.slots.contains_key(&id) {
            if shelf.slots.len() >= MAX_ICONS_PER_SERVER {
                return Lookup::Nothing;
            }
            let slot = match load(&folder, id) {
                Some((image, false)) => Slot { image: Some(image), state: State::Fresh },
                Some((image, true)) => Slot { image: Some(image), state: State::Due },
                None => Slot { image: None, state: State::Due },
            };
            shelf.slots.insert(id, slot);
        }
        let slot = shelf.slots.get_mut(&id).expect("the slot was just added");
        let ask = may_ask
            && match slot.state {
                State::Fresh | State::Refused => false,
                State::Due => true,
                State::Asked(at) => now.duration_since(at) >= ASK_AGAIN_AFTER,
                State::Failed(at) => now.duration_since(at) >= RETRY_AFTER,
            };
        if ask {
            slot.state = State::Asked(now);
        }
        match (&slot.image, ask) {
            (Some(image), true) => Lookup::Refresh(image.clone()),
            (Some(image), false) => Lookup::Ready(image.clone()),
            (None, true) => Lookup::Ask,
            (None, false) if matches!(slot.state, State::Asked(_)) => Lookup::Waiting,
            (None, false) => Lookup::Nothing,
        }
    }

    pub fn arrived(&mut self, server_uid: &str, id: u32, data: &[u8]) -> Result<(), Reject> {
        let folder = self.folder(server_uid);
        let outcome = store(&folder, id, data);
        let shelf = self.shelf(server_uid);
        match outcome {
            Ok(image) => {
                shelf.slots.insert(id, Slot { image: Some(image), state: State::Fresh });
                Ok(())
            }
            Err(reason) => {
                let slot = shelf.slots.entry(id).or_insert(Slot { image: None, state: State::Due });
                slot.state = State::Refused;
                Err(reason)
            }
        }
    }

    pub fn failed(&mut self, server_uid: &str, id: u32, now: Instant) {
        let slot = self.shelf(server_uid).slots.entry(id).or_insert(Slot { image: None, state: State::Due });
        slot.state = State::Failed(now);
    }

    pub fn forget_pending(&mut self, server_uid: &str) {
        let Some(shelf) = self.shelves.get_mut(server_uid) else {
            return;
        };
        shelf.slots.retain(|_, slot| slot.image.is_some() || !matches!(slot.state, State::Asked(_)));
        for slot in shelf.slots.values_mut() {
            if matches!(slot.state, State::Asked(_)) {
                slot.state = State::Due;
            }
        }
    }
}

fn store(folder: &Path, id: u32, data: &[u8]) -> Result<Image, Reject> {
    let format = accept(data)?;
    let extension = format.extension().ok_or(Reject::Unsupported)?;
    let path = folder.join(file_name(id, extension));
    let part = folder.join(format!("{}.part", file_name(id, extension)));
    let written = std::fs::create_dir_all(folder)
        .and_then(|_| std::fs::write(&part, data))
        .and_then(|_| std::fs::rename(&part, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
        return Err(Reject::Broken);
    }
    for other in EXTENSIONS.iter().filter(|other| **other != extension) {
        let _ = std::fs::remove_file(folder.join(file_name(id, other)));
    }
    match decode(&path) {
        Some(image) => {
            prune(folder, MAX_ICONS_PER_SERVER);
            Ok(image)
        }
        None => {
            let _ = std::fs::remove_file(&path);
            Err(Reject::Broken)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        data
    }

    fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
        data
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    fn png(side: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            let mut tagged = tag.to_vec();
            tagged.extend_from_slice(body);
            out.extend_from_slice(&tagged);
            out.extend_from_slice(&crc32(&tagged).to_be_bytes());
        }
        let mut raw = Vec::new();
        for y in 0..side {
            raw.push(0);
            for x in 0..side {
                raw.extend_from_slice(&pixel(x, y));
            }
        }
        let (mut a, mut b) = (1u32, 0u32);
        for byte in &raw {
            a = (a + u32::from(*byte)) % 65521;
            b = (b + a) % 65521;
        }
        let mut packed = vec![0x78, 0x01];
        let mut blocks = raw.chunks(65535).peekable();
        while let Some(block) = blocks.next() {
            packed.push(u8::from(blocks.peek().is_none()));
            packed.extend_from_slice(&(block.len() as u16).to_le_bytes());
            packed.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            packed.extend_from_slice(block);
        }
        packed.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut header = Vec::new();
        header.extend_from_slice(&side.to_be_bytes());
        header.extend_from_slice(&side.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &packed);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("phishspeak-icon-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn reads_dimensions_from_headers() {
        assert_eq!(sniff(&png_header(16, 16)), Some((Format::Png, 16, 16)));
        assert_eq!(sniff(b"GIF89a\x10\x00\x20\x00\x00\x00\x00"), Some((Format::Gif, 16, 32)));
        assert_eq!(sniff(&jpeg_header(48, 24)), Some((Format::Jpeg, 48, 24)));
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 16]);
        bmp.extend_from_slice(&16i32.to_le_bytes());
        bmp.extend_from_slice(&(-16i32).to_le_bytes());
        assert_eq!(sniff(&bmp), Some((Format::Bmp, 16, 16)));
        assert_eq!(sniff(b"<html><body>404</body></html>"), None);
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(&png_header(16, 16)[..20]), None);
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF]), None);
        assert_eq!(sniff(&[]), None);
        assert_eq!(sniff(&png(16, |_, _| [1, 2, 3, 255])), Some((Format::Png, 16, 16)));
    }

    #[test]
    fn rejects_what_must_not_be_decoded() {
        assert_eq!(accept(&png_header(16, 16)), Ok(Format::Png));
        assert_eq!(accept(&jpeg_header(256, 256)), Ok(Format::Jpeg));
        assert_eq!(accept(&png_header(20000, 20000)), Err(Reject::TooLarge));
        assert_eq!(accept(&png_header(257, 16)), Err(Reject::TooLarge));
        assert_eq!(accept(&png_header(0, 16)), Err(Reject::TooLarge));
        assert_eq!(accept(&jpeg_header(16, 300)), Err(Reject::TooLarge));
        assert_eq!(accept(b"GIF89a\x10\x00\x10\x00\x00\x00\x00"), Err(Reject::Unsupported));
        assert_eq!(accept(&[]), Err(Reject::Empty));
        assert_eq!(accept(b"not an image at all"), Err(Reject::NotAnImage));
        assert_eq!(accept(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Err(Reject::NotAnImage));
        let mut big = png_header(16, 16);
        big.resize(MAX_ICON_BYTES + 1, 0);
        assert_eq!(accept(&big), Err(Reject::TooManyBytes));
    }

    #[test]
    fn each_server_gets_its_own_folder() {
        let a = server_folder("SwHpaSmzsKpQ4ksmdkMFOMpBhqA=");
        let b = server_folder("swhpasmzskpq4ksmdkmfompbhqa=");
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(server_folder("../../x"), "2e2e2f2e2e2f78");
        assert_eq!(server_folder(""), "unknown");
        assert_eq!(standard_icon(300), Some(2));
        assert_eq!(standard_icon(301), None);
    }

    #[test]
    fn an_icon_is_asked_for_once_and_retried_later() {
        let root = scratch("asking");
        let mut store = IconStore::new(root.clone());
        let start = Instant::now();
        assert!(matches!(store.lookup("serverA", 0, start), Lookup::Nothing));
        assert!(matches!(store.lookup("serverA", 600, start), Lookup::Standard(4)));
        assert!(matches!(store.lookup("serverA", 77, start), Lookup::Ask));
        assert!(matches!(store.lookup("serverA", 77, start), Lookup::Waiting));
        assert!(matches!(store.lookup("serverB", 77, start), Lookup::Ask));
        assert!(matches!(store.lookup("serverA", 77, start + ASK_AGAIN_AFTER), Lookup::Ask));
        store.failed("serverA", 77, start);
        assert!(matches!(store.lookup("serverA", 77, start + Duration::from_secs(60)), Lookup::Nothing));
        assert!(matches!(store.lookup("serverA", 77, start + RETRY_AFTER + Duration::from_secs(1)), Lookup::Ask));
        assert_eq!(store.arrived("serverA", 77, b"junk"), Err(Reject::NotAnImage));
        assert_eq!(store.arrived("serverA", 77, b"GIF89a\x10\x00\x10\x00\x00\x00\x00"), Err(Reject::Unsupported));
        assert!(matches!(store.lookup("serverA", 77, start), Lookup::Nothing));
        let much_later = start + RETRY_AFTER * 100;
        assert!(matches!(store.lookup("serverA", 77, much_later), Lookup::Nothing), "a refused file is not fetched again");
        assert!(Reject::Unsupported.reason().contains("PNG"));
        assert!(!root.join(server_folder("serverA")).exists());

        assert!(matches!(store.lookup("serverA", 78, start), Lookup::Ask));
        assert!(matches!(store.lookup("serverB", 77, start), Lookup::Waiting));
        store.forget_pending("serverA");
        assert!(matches!(store.lookup("serverA", 78, start), Lookup::Ask));
        assert!(matches!(store.lookup("serverB", 77, start), Lookup::Waiting));
        assert!(matches!(store.peek("serverD", 3, start), Lookup::Nothing));
        assert!(matches!(store.peek("serverD", 600, start), Lookup::Standard(4)));
        assert!(matches!(store.lookup("serverD", 3, start), Lookup::Ask));
        assert!(matches!(store.peek("serverD", 3, start), Lookup::Waiting));
        store.failed("serverC", 5, start);
        assert!(matches!(store.lookup("serverC", 5, start), Lookup::Nothing));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_good_icon_is_kept_and_found_again() {
        let root = scratch("keeping");
        let start = Instant::now();
        let small = png(16, |x, y| [(x * 16) as u8, (y * 16) as u8, 200, 255]);
        let large = png(64, |x, _| if x < 32 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        let folder = root.join(server_folder("serverA"));
        {
            let mut store = IconStore::new(root.clone());
            assert!(matches!(store.lookup("serverA", 9, start), Lookup::Ask));
            assert_eq!(store.arrived("serverA", 9, &small), Ok(()));
            assert_eq!(store.arrived("serverA", 10, &large), Ok(()));
            let Lookup::Ready(image) = store.lookup("serverA", 9, start) else {
                panic!("the icon should be ready");
            };
            assert_eq!((image.size().width, image.size().height), (16, 16));
            let pixels = image.to_rgba8().unwrap();
            assert_eq!(&pixels.as_bytes()[..4], &[0, 0, 200, 255]);
            assert_eq!(&pixels.as_bytes()[(16 * 5 + 3) * 4..(16 * 5 + 3) * 4 + 4], &[48, 80, 200, 255]);
            let Lookup::Ready(image) = store.lookup("serverA", 10, start) else {
                panic!("the large icon should be ready");
            };
            assert_eq!((image.size().width, image.size().height), (SHOWN_SIDE, SHOWN_SIDE));
            let pixels = image.to_rgba8().unwrap();
            assert_eq!(&pixels.as_bytes()[..4], &[255, 0, 0, 255]);
            assert_eq!(&pixels.as_bytes()[31 * 4..32 * 4], &[0, 0, 255, 255]);
            assert!(matches!(store.lookup("serverB", 9, start), Lookup::Ask));
        }
        assert!(folder.join("icon_9.png").is_file() && folder.join("icon_10.png").is_file());
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);

        let mut again = IconStore::new(root.clone());
        assert!(matches!(again.lookup("serverA", 9, start), Lookup::Ready(_)));
        assert!(matches!(again.lookup("serverA", 9, start), Lookup::Ready(_)));

        let week_old = SystemTime::now() - REFRESH_AFTER - Duration::from_secs(60);
        std::fs::write(folder.join("icon_11.png"), &large).unwrap();
        std::fs::File::options().write(true).open(folder.join("icon_11.png")).unwrap().set_modified(week_old).unwrap();
        let mut stale = IconStore::new(root.clone());
        let Lookup::Refresh(old) = stale.lookup("serverA", 11, start) else {
            panic!("an old file is shown while a new copy is fetched");
        };
        assert_eq!(old.size().width, SHOWN_SIDE);
        assert!(matches!(stale.lookup("serverA", 11, start), Lookup::Ready(_)));
        stale.failed("serverA", 11, start);
        assert!(matches!(stale.lookup("serverA", 11, start + Duration::from_secs(5)), Lookup::Ready(_)));
        assert!(matches!(stale.lookup("serverA", 11, start + RETRY_AFTER), Lookup::Refresh(_)));
        stale.forget_pending("serverA");
        assert!(matches!(stale.lookup("serverA", 11, start + RETRY_AFTER), Lookup::Refresh(_)));
        assert_eq!(stale.arrived("serverA", 11, &small), Ok(()));
        let Lookup::Ready(image) = stale.lookup("serverA", 11, start) else {
            panic!("the replaced icon should be ready");
        };
        assert_eq!(image.size().width, 16);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_only_looks_like_an_image_is_dropped() {
        let root = scratch("broken");
        let start = Instant::now();
        let mut store = IconStore::new(root.clone());
        let mut broken = png_header(16, 16);
        broken.extend_from_slice(&[0x12; 200]);
        assert_eq!(store.arrived("serverA", 5, &broken), Err(Reject::Broken));
        assert!(matches!(store.lookup("serverA", 5, start), Lookup::Nothing));
        let folder = root.join(server_folder("serverA"));
        assert_eq!(std::fs::read_dir(&folder).map(|entries| entries.count()).unwrap_or(0), 0);

        let good = png(16, |_, _| [9, 9, 9, 255]);
        std::fs::write(folder.join("icon_6.jpg"), &good).unwrap();
        std::fs::write(folder.join("icon_7.png"), b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        let mut fresh = IconStore::new(root.clone());
        assert!(matches!(fresh.lookup("serverA", 6, start), Lookup::Ask), "a file whose name and content disagree is not loaded");
        assert!(matches!(fresh.lookup("serverA", 7, start), Lookup::Ask));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_server_cannot_fill_the_store_or_the_disk() {
        let root = scratch("limits");
        let start = Instant::now();
        let mut store = IconStore::new(root.clone());
        for id in 1..=MAX_ICONS_PER_SERVER as u32 {
            assert!(matches!(store.lookup("serverA", 1000 + id, start), Lookup::Ask));
        }
        assert!(matches!(store.lookup("serverA", 5000, start), Lookup::Nothing));
        assert!(matches!(store.lookup("serverA", 1001, start), Lookup::Waiting));
        assert!(matches!(store.lookup("serverB", 5000, start), Lookup::Ask));

        let folder = root.join("pruned");
        std::fs::create_dir_all(&folder).unwrap();
        for id in 0..12u64 {
            let path = folder.join(format!("icon_{id}.png"));
            std::fs::write(&path, b"x").unwrap();
            let when = SystemTime::now() - Duration::from_secs(1000 - id * 10);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
        std::fs::write(folder.join("icon_3.png.part"), b"x").unwrap();
        std::fs::write(folder.join("notes.txt"), b"x").unwrap();
        prune(&folder, 5);
        let mut left: Vec<String> =
            std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["icon_10.png", "icon_11.png", "icon_7.png", "icon_8.png", "icon_9.png", "notes.txt"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn large_pictures_are_averaged_down() {
        assert_eq!(shrink(&[0; 16 * 16 * 4], 16, 16, 32), None);
        assert_eq!(shrink(&[0; 8], 2, 2, 1), None);
        let mut pixels = Vec::new();
        for y in 0..4u8 {
            for x in 0..4u8 {
                pixels.extend_from_slice(match (x < 2, y < 2) {
                    (true, true) => &[200, 0, 0, 255],
                    (false, true) => &[0, 100, 0, 255],
                    (true, false) => &[0, 0, 0, 0],
                    (false, false) => &[10, 20, 30, 255],
                });
            }
        }
        pixels[(2 * 4 + 2) * 4..(2 * 4 + 2) * 4 + 4].copy_from_slice(&[250, 250, 250, 0]);
        let (out, width, height) = shrink(&pixels, 4, 4, 2).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(&out[..4], &[200, 0, 0, 255]);
        assert_eq!(&out[4..8], &[0, 100, 0, 255]);
        assert_eq!(&out[8..12], &[0, 0, 0, 0]);
        assert_eq!(&out[12..16], &[10, 20, 30, 191]);
        let wide = vec![255u8; 8 * 2 * 4];
        let (out, width, height) = shrink(&wide, 8, 2, 4).unwrap();
        assert_eq!((width, height, out.len()), (4, 1, 16));
    }
}
