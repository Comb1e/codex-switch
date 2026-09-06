#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo 'Cargo was not found. Install Rust from https://rustup.rs and try again.' >&2
  exit 1
fi

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cargo install --path "$project_root" --force

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
installed="$cargo_home/bin/cs"
if [[ ! -x "$installed" ]]; then
  echo "Installation completed but cs was not found at $installed" >&2
  exit 1
fi

"$installed" --version
case ":$PATH:" in
  *":$cargo_home/bin:"*) ;;
  *) echo "Add $cargo_home/bin to PATH to run cs from any Bash session." ;;
esac
