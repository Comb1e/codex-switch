use std::ffi::OsString;
use std::io::{self, IsTerminal, Read};

use anyhow::{Result, bail};
use clap::{ArgGroup, Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "cs",
    version,
    about = "Switch named Codex endpoint and API-key profiles; omit the name to resume the last profile"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Save a named endpoint and API-key profile.
    Add(AddArgs),
    /// List saved profiles without showing API keys.
    List,
    /// Show the current provider and all saved profiles without API keys.
    Status,
    /// Print the active profile, or "unmanaged".
    Current,
    /// Remove a saved profile.
    Remove(RemoveArgs),
    /// Internal command used by the local relay process.
    #[command(name = "__relay", hide = true)]
    Relay {
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,
    },
    /// Refresh startup registration and restart a previously activated relay.
    #[command(name = "__relay-install", hide = true)]
    RelayInstall,
    /// A profile name may be used directly, for example: cs work
    #[command(external_subcommand)]
    Profile(Vec<OsString>),
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("api-key-source")
        .args(["apikey", "apikey_stdin"])
        .multiple(false)
))]
struct AddArgs {
    /// Profile name: lowercase letters, digits, '.', '_' and '-'.
    #[arg(long)]
    name: String,
    /// Absolute HTTP(S) API base URL.
    #[arg(long)]
    baseurl: String,
    /// API key. Prefer --apikey-stdin or the hidden prompt for shared machines.
    #[arg(long)]
    apikey: Option<String>,
    /// Read the API key from standard input.
    #[arg(long)]
    apikey_stdin: bool,
    /// Replace an existing profile with the same name.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// Saved profile name.
    name: String,
    /// Remove an active profile while leaving the applied Codex files unchanged.
    #[arg(long)]
    force: bool,
}

pub enum Action {
    Add {
        name: String,
        base_url: String,
        api_key: String,
        force: bool,
    },
    List,
    Status,
    Current,
    Remove {
        name: String,
        force: bool,
    },
    Activate {
        name: String,
    },
    Resume,
    Relay {
        data_dir: Option<std::path::PathBuf>,
    },
    RelayInstall,
}

impl Cli {
    pub fn into_action(self) -> Result<Action> {
        match self.command {
            Some(Command::Add(args)) => Ok(Action::Add {
                name: args.name,
                base_url: args.baseurl,
                api_key: read_api_key(args.apikey, args.apikey_stdin)?,
                force: args.force,
            }),
            Some(Command::List) => Ok(Action::List),
            Some(Command::Status) => Ok(Action::Status),
            Some(Command::Current) => Ok(Action::Current),
            Some(Command::Remove(args)) => Ok(Action::Remove {
                name: args.name,
                force: args.force,
            }),
            Some(Command::Relay { data_dir }) => Ok(Action::Relay { data_dir }),
            Some(Command::RelayInstall) => Ok(Action::RelayInstall),
            Some(Command::Profile(parts)) => {
                if parts.len() != 1 {
                    bail!("activation accepts exactly one profile name");
                }
                let name = parts[0]
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("profile name must be valid UTF-8"))?;
                Ok(Action::Activate {
                    name: name.to_owned(),
                })
            }
            None => Ok(Action::Resume),
        }
    }
}

fn read_api_key(argument: Option<String>, from_stdin: bool) -> Result<String> {
    if let Some(key) = argument {
        return Ok(key);
    }

    if from_stdin {
        let mut key = String::new();
        io::stdin().read_to_string(&mut key)?;
        return Ok(key.trim_end_matches(['\r', '\n']).to_owned());
    }

    if !io::stdin().is_terminal() {
        bail!("no API key supplied; use --apikey or --apikey-stdin");
    }

    rpassword::prompt_password("API key: ").map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Action, Cli};

    #[test]
    fn parses_direct_profile_activation() {
        let action = Cli::try_parse_from(["cs", "work"])
            .unwrap()
            .into_action()
            .unwrap();
        assert!(matches!(action, Action::Activate { name } if name == "work"));
    }

    #[test]
    fn parses_add_with_inline_key() {
        let action = Cli::try_parse_from([
            "cs",
            "add",
            "--name",
            "work",
            "--baseurl",
            "https://example.test/v1",
            "--apikey",
            "secret",
        ])
        .unwrap()
        .into_action()
        .unwrap();
        assert!(matches!(action, Action::Add { name, .. } if name == "work"));
    }

    #[test]
    fn rejects_multiple_profile_arguments() {
        let error = Cli::try_parse_from(["cs", "work", "extra"])
            .unwrap()
            .into_action()
            .err()
            .unwrap();
        assert!(error.to_string().contains("exactly one"));
    }
}
