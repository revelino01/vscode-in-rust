const vscode = require('vscode');

/**
 * @param {vscode.ExtensionContext} context
 */
function activate(context) {
  console.log('[Extension] sample-hello-world is now active in RustCode!');

  const disposable = vscode.commands.registerCommand('sample.helloWorld', function () {
    vscode.window.showInformationMessage('Hello World from RustCode Extension Host!');
  });

  if (context && context.subscriptions) {
    context.subscriptions.push(disposable);
  }
}

function deactivate() {
  console.log('[Extension] sample-hello-world deactivated');
}

module.exports = {
  activate,
  deactivate
};
