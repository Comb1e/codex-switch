mod app;
mod atomic;
mod cli;
mod codex;
mod paths;
mod profile;
mod validation;

use anyhow::Result;
use clap::Parser;

use crate::app::App;
use crate::atomic::RealFileWriter;
use crate::cli::Cli;
use crate::paths::AppPaths;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let action = Cli::parse().into_action()?;
    let app = App::new(AppPaths::discover()?, RealFileWriter);
    let output = app.execute(action)?;
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}
