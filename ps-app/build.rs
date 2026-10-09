use std::path::Path;

const ICON: u16 = 3;
const ICON_GROUP: u16 = 14;
const MAY_MOVE_AND_BE_DROPPED: u16 = 0x1010;
const MAY_MOVE_BE_DROPPED_AND_IS_READ_ONLY: u16 = 0x1030;
const ENGLISH: u16 = 0x0409;
const FIRST: u16 = 1;

fn resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xff, 0xff]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xff, 0xff]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&ENGLISH.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn number(bytes: &[u8], at: usize, width: usize) -> Option<usize> {
    let part = bytes.get(at..at + width)?;
    Some(part.iter().rev().fold(0usize, |sum, byte| (sum << 8) | usize::from(*byte)))
}

fn icon_resources(file: &[u8]) -> Option<Vec<u8>> {
    let count = number(file, 4, 2).filter(|count| *count > 0 && *count < 64)?;
    if number(file, 0, 2)? != 0 || number(file, 2, 2)? != 1 {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0]);
    out.extend_from_slice(&[0u8; 16]);
    let mut group = Vec::new();
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for index in 0..count {
        let entry = 6 + 16 * index;
        let size = number(file, entry + 8, 4)?;
        let from = number(file, entry + 12, 4)?;
        let picture = file.get(from..from.checked_add(size)?)?;
        let id = FIRST + index as u16;
        resource(&mut out, ICON, id, MAY_MOVE_AND_BE_DROPPED, picture);
        group.extend_from_slice(file.get(entry..entry + 12)?);
        group.extend_from_slice(&id.to_le_bytes());
    }
    resource(&mut out, ICON_GROUP, FIRST, MAY_MOVE_BE_DROPPED_AND_IS_READ_ONLY, &group);
    Some(out)
}

fn main() {
    slint_build::compile("ui/main.slint").expect("compiling ui/main.slint failed");
    println!("cargo:rerun-if-changed=ui/app-icon.ico");
    let target = |name: &str, wanted: &str| std::env::var(name).is_ok_and(|value| value == wanted);
    if target("CARGO_CFG_TARGET_OS", "windows") && target("CARGO_CFG_TARGET_ENV", "msvc") {
        let file = std::fs::read("ui/app-icon.ico").expect("ui/app-icon.ico could not be read");
        let resources = icon_resources(&file).expect("ui/app-icon.ico is not an icon file");
        let out = Path::new(&std::env::var("OUT_DIR").expect("cargo names the output folder")).join("app-icon.res");
        std::fs::write(&out, resources).expect("the icon resource could not be written");
        println!("cargo:rustc-link-arg-bins={}", out.display());
    }
}
