//! Output helpers: human lines by default, JSON with `--json`.

use serde::Serialize;

pub const EXIT_OK: i32 = 0;
pub const EXIT_NEEDS_HUMAN: i32 = 3;

pub fn emit<T: Serialize>(json: bool, value: &T, human: impl FnOnce() -> String) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(value)
                .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
        );
    } else {
        let s = human();
        if !s.is_empty() {
            println!("{s}");
        }
    }
}

pub fn note(msg: &str) {
    eprintln!("vpak: {msg}");
}
