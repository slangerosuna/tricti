const vscode = require('vscode');

/**
 * @param {vscode.ExtensionContext} context
 */
function activate(context) {
    // No runtime activation needed for pure syntax highlighting.
}

function deactivate() {}

module.exports = {
    activate,
    deactivate,
};
