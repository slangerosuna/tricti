#!/bin/bash

# TriCTI VS Code Extension Installation Script

set -euo pipefail

INSTALL_VIM=false
VIM_INSTALL_ARGS=()

usage() {
    cat <<EOF
Usage: ./install.sh [--with-vim] [--vim-path PATH] [--nvim-path PATH]

Installs the TriCTI VS Code extension. Pass --with-vim to also install the accompanying
Vim/Neovim syntax files (you can forward optional --vim-path/--nvim-path arguments to
control where they are installed).
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --with-vim)
            INSTALL_VIM=true
            shift
            ;;
        --vim-path|--nvim-path)
            INSTALL_VIM=true
            if [[ $# -lt 2 ]]; then
                echo "Error: $1 requires an argument" >&2
                exit 1
            fi
            VIM_INSTALL_ARGS+=("$1" "$2")
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage
            exit 1
            ;;
    esac
done

echo "Installing TriCTI VS Code Extension..."

# Check if VS Code is installed
if ! command -v code &> /dev/null; then
    echo "Error: VS Code 'code' command not found. Please install VS Code and ensure it's in your PATH."
    exit 1
fi

# Get the directory of this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Package the extension if .vsix doesn't exist
if [ ! -f "$SCRIPT_DIR/tricti-extension.vsix" ]; then
    echo "Packaging extension..."
    if command -v vsce &> /dev/null; then
        cd "$SCRIPT_DIR"
        vsce package --out tricti-extension.vsix
    else
        echo "Error: vsce not found. Please install it with: npm install -g vsce"
        exit 1
    fi
fi

# Install the extension
echo "Installing extension..."
code --install-extension "$SCRIPT_DIR/tricti-extension.vsix"

echo "✅ TriCTI VS Code Extension installed successfully!"
if $INSTALL_VIM; then
    echo "Installing TriCTI Vim/Neovim syntax..."
    bash "$SCRIPT_DIR/../vim/install.sh" "${VIM_INSTALL_ARGS[@]}"
fi
echo "You may need to restart VS Code for the extension to take effect."
echo "Open any .tri file to see syntax highlighting."