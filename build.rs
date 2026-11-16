//! exe に yomitori.manifest（Per-Monitor DPI v2・Common Controls v6）と assets/icon/yomitori.ico を埋め込む。
//! アイコンは .ico から .res（RT_ICON + RT_GROUP_ICON）をここで組み立てて MSVC のリンカに渡す
//! （rc.exe を Windows SDK から探さずに済むため）

use std::path::Path;

const ICON: &str = "assets/icon/yomitori.ico";
/// アプリのアイコンのリソース ID。src/platform/icon.rs と合わせる
const ICON_GROUP_ID: u16 = 1;

fn main() {
    println!("cargo:rerun-if-changed=yomitori.manifest");
    println!("cargo:rerun-if-changed={ICON}");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("msvc") {
        return;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo が渡す");
    let manifest = Path::new(&dir).join("yomitori.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );

    let ico = std::fs::read(Path::new(&dir).join(ICON)).expect("アイコンを読めない");
    let res = icon_res(&ico);
    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo が渡す")).join("yomitori-icon.res");
    std::fs::write(&out, res).expect(".res を書けない");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// .res の 1 件。種類と名前は数値（0xFFFF + ID）で書く
fn resource(out: &mut Vec<u8>, kind: u16, name: u16, memory_flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&memory_flags.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // LanguageId（中立）
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn icon_res(ico: &[u8]) -> Vec<u8> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    assert!(
        u16_at(ico, 0) == 0 && u16_at(ico, 2) == 1,
        "{ICON} がアイコンの形式ではない"
    );
    let count = u16_at(ico, 4) as usize;

    let mut out = Vec::new();
    // .res の先頭は中身の無い 1 件（形式の決まり）
    resource(&mut out, 0, 0, 0, &[]);

    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let e = 6 + 16 * i;
        let size = u32_at(ico, e + 8) as usize;
        let offset = u32_at(ico, e + 12) as usize;
        let id = (i + 1) as u16;
        resource(&mut out, RT_ICON, id, 0x1010, &ico[offset..offset + size]);
        // GRPICONDIRENTRY は ICONDIRENTRY の先頭 12 バイト + 画像の ID（2 バイト）
        group.extend_from_slice(&ico[e..e + 12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    resource(&mut out, RT_GROUP_ICON, ICON_GROUP_ID, 0x1030, &group);
    out
}
