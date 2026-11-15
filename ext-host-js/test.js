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

console.log('All JavaScript extension host tests passed successfully!');

function assert(condition, message) {
  if (!condition) {
    throw new Error('Assertion failed: ' + (message || ''));
  }
}
