mod app;
mod atomic;
mod cli;
mod codex;
mod paths;
mod profile;
mod relay;
mod validation;

use anyhow::Result;
use clap::Parser;

use crate::app::App;
use crate::atomic::RealFileWriter;
use crate::cli::Cli;
use crate::paths::AppPaths;
use crate::relay::RealRelayManager;

pub fn run() -> Result<()> {
    let action = Cli::parse().into_action()?;
    let app = App::new(AppPaths::discover()?, RealFileWriter, RealRelayManager);
    let output = app.execute(action)?;
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}

/// Windowless login entry point. Errors go to the same log as the worker.
pub fn startup() -> Result<()> {
    use std::path::PathBuf;

    #[derive(Parser)]
    struct StartupArgs {
        #[arg(long)]
        data_dir: PathBuf,
    }

    let args = StartupArgs::try_parse()?;
    let paths = AppPaths::discover()?.with_data_dir(args.data_dir);
    let result = relay::start_at_login(&paths);
    if let Err(error) = &result {
        relay::log_error(&paths, error);
    }
    result
}
