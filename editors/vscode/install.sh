#!/bin/bash

# TriCTI VS Code Extension Installation Script

set -e

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
echo "You may need to restart VS Code for the extension to take effect."
echo "Open any .tri file to see syntax highlighting."