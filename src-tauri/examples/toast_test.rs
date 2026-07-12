//! Manual toast smoke test: `cargo run --example toast_test`.

use tauri_winrt_notification::Toast;
use winreg::enums::{HKEY_CURRENT_USER, REG_EXPAND_SZ};
use winreg::{RegKey, RegValue};

fn expand_sz(value: &str) -> RegValue {
    let mut bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect();
    bytes.shrink_to_fit();
    RegValue {
        vtype: REG_EXPAND_SZ,
        bytes,
    }
}

fn main() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(r"Software\Classes\AppUserModelId\com.skim.app")
        .unwrap();
    key.set_raw_value("DisplayName", &expand_sz("Skim")).unwrap();
    key.set_raw_value(
        "IconUri",
        &expand_sz(r"C:\skim\src-tauri\icons\128x128.png"),
    )
    .unwrap();
    println!("aumid re-registered with REG_EXPAND_SZ + IconUri");

    let r = Toast::new("com.skim.app")
        .title("Test 4 — skim aumid, expand_sz")
        .text1("кнопка должна быть ниже")
        .add_button("Прочитано", "read:1")
        .show();
    println!("4 -> {r:?}");
    std::thread::sleep(std::time::Duration::from_secs(3));
}
