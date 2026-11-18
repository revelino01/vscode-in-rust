// Test script for ext-host-js shim

const vscode = require('./vscode-api/index');
const EventEmitter = require('./node-compat/events');
const path = require('./node-compat/path');

console.log('[Test] Testing Position and Range...');
const p1 = new vscode.Position(1, 5);
const p2 = new vscode.Position(2, 0);
assert(p1.isBefore(p2), 'p1 should be before p2');
assert(!p1.isEqual(p2), 'p1 should not equal p2');

const range = new vscode.Range(p1, p2);
assert(!range.isEmpty, 'range should not be empty');
assert(!range.isSingleLine, 'range should not be single line');
assert(range.contains(new vscode.Position(1, 10)), 'range should contain (1, 10)');

console.log('[Test] Testing Uri...');
const uri = vscode.Uri.file('/path/to/file.rs');
assert(uri.fsPath === '/path/to/file.rs', 'Uri fsPath mismatch');

console.log('[Test] Testing EventEmitter...');
const ee = new EventEmitter();
let emitted = false;
ee.on('testEvent', (val) => {
  emitted = val;
});
ee.emit('testEvent', true);
assert(emitted === true, 'EventEmitter did not deliver event');

console.log('[Test] Testing Path shim...');
assert(path.join('a', 'b', 'c.rs') === 'a/b/c.rs', 'path.join failed');
assert(path.basename('/a/b/c.rs') === 'c.rs', 'path.basename failed');
assert(path.extname('/a/b/c.rs') === '.rs', 'path.extname failed');

console.log('[Test] Testing vscode namespace APIs...');
assert(typeof vscode.commands.registerCommand === 'function');
assert(typeof vscode.window.showInformationMessage === 'function');
assert(typeof vscode.workspace.getConfiguration === 'function');
assert(typeof vscode.languages.registerCompletionItemProvider === 'function');
assert(typeof vscode.languages.registerDocumentFormattingEditProvider === 'function');
assert(typeof vscode.languages.registerCodeActionsProvider === 'function');
assert(typeof vscode.languages.format === 'function');
assert(typeof vscode.languages.codeAction === 'function');

console.log('[Test] Testing TextEdit, WorkspaceEdit, CodeAction...');
const edit1 = vscode.TextEdit.replace(range, 'replacement text');
assert(edit1.newText === 'replacement text', 'TextEdit newText mismatch');

const wsEdit = new vscode.WorkspaceEdit();
wsEdit.set(uri, [edit1]);
assert(wsEdit.has(uri), 'WorkspaceEdit should have uri');
assert(wsEdit.get(uri).length === 1, 'WorkspaceEdit get(uri) length mismatch');

const action = new vscode.CodeAction('Fix issue', vscode.CodeActionKind.QuickFix);
assert(action.title === 'Fix issue', 'CodeAction title mismatch');
assert(action.kind === 'quickfix', 'CodeAction kind mismatch');
action.edit = wsEdit;

console.log('[Test] Testing vscode.notebooks API...');
const rpc = require('./rpc-bridge');
assert(vscode.NotebookCellKind.Markup === 1, 'NotebookCellKind.Markup mismatch');
assert(vscode.NotebookCellKind.Code === 2, 'NotebookCellKind.Code mismatch');
assert(typeof vscode.notebooks.registerNotebookSerializer === 'function');
assert(typeof vscode.notebooks.createNotebookController === 'function');
assert(typeof vscode.notebooks.createNotebookCellExecution === 'function');
assert(typeof vscode.notebooks.onDidOpenNotebookDocument === 'function');

const nbCell = new vscode.NotebookCellData(vscode.NotebookCellKind.Code, 'x = 1', 'python');
const nbData = new vscode.NotebookData([nbCell]);
assert(nbData.cells.length === 1, 'NotebookData cells length mismatch');

const outItem = vscode.NotebookCellOutputItem.text('hello');
const outItemJson = outItem.toJSON();
assert(outItemJson.mime === 'text/plain', 'NotebookCellOutputItem mime mismatch');
assert(Array.isArray(outItemJson.data) && outItemJson.data.length === 5, 'NotebookCellOutputItem data JSON mismatch');
const cellOutput = new vscode.NotebookCellOutput([outItem]);
assert(cellOutput.items.length === 1, 'NotebookCellOutput items length mismatch');

const nbRange = new vscode.NotebookRange(0, 2);
assert(!nbRange.isEmpty, 'NotebookRange should not be empty');
const nbEdit = vscode.NotebookEdit.replaceCells(nbRange, [nbCell]);
assert(nbEdit.newCells.length === 1, 'NotebookEdit newCells mismatch');

const serDisp = vscode.notebooks.registerNotebookSerializer('jupyter-notebook', {
  deserializeNotebook(bytes) {
    return new vscode.NotebookData([
      new vscode.NotebookCellData(vscode.NotebookCellKind.Code, String(bytes.length), 'python'),
    ]);
  },
  serializeNotebook(data) {
    return new Uint8Array([data.cells.length]);
  },
});

let executedCount = 0;
const ctrl = vscode.notebooks.createNotebookController('pure-rust', 'jupyter-notebook', 'Pure Rust', (cells) => {
  executedCount += cells.length;
});
const exec = ctrl.createNotebookCellExecution({ index: 0, notebookUri: 'file:///demo.ipynb' });
exec.start(100);
exec.replaceOutput(cellOutput);
exec.end(true, 200);

(async () => {
  const deser = await rpc.handleIncomingRequest('notebooks.deserializeNotebook', {
    notebookType: 'jupyter-notebook',
    content: 'abc',
  });
  assert(deser.cells.length === 1 && deser.cells[0].value === '3', 'deserializeNotebook RPC failed');

  const ser = await rpc.handleIncomingRequest('notebooks.serializeNotebook', {
    notebookType: 'jupyter-notebook',
    data: deser,
  });
  assert(Array.isArray(ser) && ser[0] === 1, 'serializeNotebook RPC failed');

  await rpc.handleIncomingRequest('notebooks.executeCells', {
    controllerId: 'pure-rust',
    cells: [nbCell],
    notebook: { uri: 'file:///demo.ipynb' },
  });
  assert(executedCount === 1, 'executeCells RPC failed');

  ctrl.dispose();
  serDisp.dispose();
  console.log('All JavaScript extension host tests passed successfully!');
})();

function assert(condition, message) {
  if (!condition) {
    throw new Error('Assertion failed: ' + (message || ''));
  }
}
