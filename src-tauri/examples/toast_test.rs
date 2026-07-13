//! Manual toast smoke test: `cargo run --example toast_test`.
//!
//! Relies on the installed app: the NSIS Start Menu shortcut carries the
//! AUMID and the app registers DisplayName/IconUri (and drops the icon to
//! ~\.skim) at startup — run Skim once before this so the toast shows the
//! proper name and icon.

use tauri_winrt_notification::{IconCrop, Toast};

fn main() {
    // Same location the app uses; must stay outside hidden directories
    // (AppData) or the notification platform silently drops the image.
    let icon = std::path::PathBuf::from(format!(
        "{}/.skim/notify-icon.png",
        std::env::var("USERPROFILE")
            .expect("USERPROFILE")
            .replace('\\', "/")
    ));
    let r = Toast::new("com.skim.app")
        .icon(&icon, IconCrop::Square, "Skim")
        .title("Skim — toast smoke test")
        .text1("Слева должен быть логотип Skim")
        .add_button("Прочитано", "read:1")
        .show();
    println!("toast -> {r:?}");
    std::thread::sleep(std::time::Duration::from_secs(3));
}
