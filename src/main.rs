fn main() {
    if let Err(error) = codex_switch::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
