//! Manual toast smoke test: `cargo run --example toast_test`.
//!
//! Relies on the installed app: the NSIS Start Menu shortcut carries the
//! AUMID and the app registers DisplayName/IconUri (and drops the icon to
//! ~\.skim) at startup — run Skim once before this so the toast shows the
//! proper name and icon.

use tauri_winrt_notification::Toast;

fn main() {
    let r = Toast::new("com.skim.app")
        .title("Skim — toast smoke test")
        .text1("Логотип должен быть в шапке, рядом с именем")
        .add_button("Прочитано", "read:1")
        .show();
    println!("toast -> {r:?}");
    std::thread::sleep(std::time::Duration::from_secs(3));
}
