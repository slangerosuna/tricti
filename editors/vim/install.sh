#!/bin/bash

# TriCTI Vim/Neovim syntax installation script

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

VIM_TARGET="${VIM_RUNTIME_PATH:-$HOME/.vim/pack/tricti/start/tricti}"
NVIM_TARGET="${NVIM_RUNTIME_PATH:-$HOME/.local/share/nvim/site/pack/tricti/start/tricti}"

copy_payload() {
  local target="$1"
  mkdir -p "$target"
  for dir in syntax ftdetect ftplugin plugin; do
    rm -rf "$target/$dir"
    cp -R "$SCRIPT_DIR/$dir" "$target/"
  done
}

if [[ -n "${1:-}" ]]; then
  case "$1" in
    --vim-path)
      [[ -n "${2:-}" ]] || { echo "Error: --vim-path requires an argument" >&2; exit 1; }
      VIM_TARGET="$2"
      shift 2
      ;;
    --nvim-path)
      [[ -n "${2:-}" ]] || { echo "Error: --nvim-path requires an argument" >&2; exit 1; }
      NVIM_TARGET="$2"
      shift 2
      ;;
    --help|-h)
      cat <<EOF
Usage: ./install.sh [--vim-path PATH] [--nvim-path PATH]

Installs the TriCTI Vim/Neovim syntax files. By default the files are copied to:
  Vim:   $VIM_TARGET
  Neovim: $NVIM_TARGET

Use --vim-path or --nvim-path to override these locations. Set VIM_RUNTIME_PATH or
NVIM_RUNTIME_PATH environment variables to change the defaults for repeat runs.
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
fi

if [[ -n "${1:-}" ]]; then
  echo "Unexpected arguments: $*" >&2
  exit 1
fi

printf "Installing TriCTI syntax for Vim at %s\n" "$VIM_TARGET"
copy_payload "$VIM_TARGET"

printf "Installing TriCTI syntax for Neovim at %s\n" "$NVIM_TARGET"
copy_payload "$NVIM_TARGET"

echo "✅ TriCTI syntax installed for Vim/Neovim. Restart your editor if open."
