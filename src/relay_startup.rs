#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    if codex_switch::startup().is_err() {
        std::process::exit(1);
    }
}
