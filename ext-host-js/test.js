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
const expectedSep = process.platform === 'win32' ? '\\' : '/';
const expectedDelimiter = process.platform === 'win32' ? ';' : ':';
assert(path.sep === expectedSep, 'path.sep OS mismatch');
assert(path.delimiter === expectedDelimiter, 'path.delimiter OS mismatch');
assert(path.join('a', 'b', 'c.rs') === ['a', 'b', 'c.rs'].join(path.sep), 'path.join failed');
assert(path.posix.join('a', 'b', 'c.rs') === 'a/b/c.rs', 'path.posix.join failed');
assert(path.win32.join('a', 'b', 'c.rs') === 'a\\b\\c.rs', 'path.win32.join failed');
assert(path.basename('/a/b/c.rs') === 'c.rs', 'path.basename failed');
assert(path.extname('/a/b/c.rs') === '.rs', 'path.extname failed');

console.log('[Test] Testing vscode namespace APIs...');
assert(typeof vscode.commands.registerCommand === 'function');
assert(typeof vscode.window.showInformationMessage === 'function');
assert(typeof vscode.window.createOutputChannel === 'function');
assert(typeof vscode.window.createTerminal === 'function');
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

console.log('[Test] Testing vscode.window OutputChannel and Terminal...');
const outChannel = vscode.window.createOutputChannel('RustCode Build', 'log');
assert(outChannel.name === 'RustCode Build', 'OutputChannel name mismatch');
outChannel.append('compiling ');
outChannel.appendLine('done');
outChannel.replace('replaced line\n');
outChannel.show(true);
outChannel.show(1, false);
outChannel.hide();
outChannel.clear();
outChannel.dispose();

const termHandle = vscode.window.createTerminal('Rust Shell');
assert(termHandle.name === 'Rust Shell', 'Terminal name mismatch');
termHandle.show(true);
termHandle.sendText('echo ok');
termHandle.dispose();

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

  console.log('[Test] Testing vscode.EventEmitter...');
  const vsEmitter = new vscode.EventEmitter();
  const vsReceived = [];
  const vsDisposables = [];
  const vsSub = vsEmitter.event(function (val) {
    vsReceived.push(this.prefix + ':' + val);
  }, { prefix: 'ok' }, vsDisposables);
  assert(vsDisposables.length === 1, 'EventEmitter should push to disposables');
  vsEmitter.fire('one');
  vsSub.dispose();
  vsEmitter.fire('two');
  assert(vsReceived.length === 1 && vsReceived[0] === 'ok:one', 'vscode.EventEmitter failed');
  vsEmitter.dispose();

  const siblingEmitter = new vscode.EventEmitter();
  const siblingLog = [];
  let secondSub = null;
  siblingEmitter.event(function (val) {
    siblingLog.push(this + ':' + val);
    if (secondSub) secondSub.dispose();
  }, 0);
  secondSub = siblingEmitter.event((val) => {
    siblingLog.push('second:' + val);
  });
  siblingEmitter.fire('x');
  assert(siblingLog.length === 1 && siblingLog[0] === '0:x', 'EventEmitter falsy thisArgs or sibling dispose failed');
  siblingEmitter.dispose();

  console.log('[Test] Testing vscode.scm API...');
  const sc = vscode.scm.createSourceControl('git', 'Git', vscode.Uri.file('/workspace'));
  sc.inputBox.value = 'feat: test';
  sc.quickDiffProvider = {
    provideOriginalResource(docUri) {
      return vscode.Uri.parse('git:' + docUri);
    },
  };
  const scGroup = sc.createResourceGroup('workingTree', 'Changes');
  scGroup.resourceStates = [{ resourceUri: vscode.Uri.file('/workspace/main.rs') }];
  assert(vscode.scm.inputBox.value === 'feat: test', 'scm.inputBox mismatch');
  const origRes = await rpc.handleIncomingRequest('scm.provideOriginalResource', {
    providerId: 'git',
    uri: 'file:///workspace/main.rs',
  });
  assert(origRes && origRes.scheme === 'git', 'scm.provideOriginalResource RPC failed');
  const acceptNullRes = await rpc.handleIncomingRequest('scm.acceptInput', {
    providerId: 'git',
    value: null,
  });
  assert(sc.inputBox.value === 'feat: test' && acceptNullRes && acceptNullRes.value === 'feat: test', 'scm.acceptInput with null must preserve inputBox.value');
  scGroup.dispose();
  sc.dispose();

  console.log('[Test] Testing vscode.debug API...');
  const dbgCfgDisp = vscode.debug.registerDebugConfigurationProvider('lldb', {
    provideDebugConfigurations() {
      return [{ type: 'lldb', name: 'Launch', request: 'launch' }];
    },
    resolveDebugConfiguration(_folder, config) {
      return { ...config, stopOnEntry: true };
    },
  });
  const dbgFactoryDisp = vscode.debug.registerDebugAdapterDescriptorFactory('lldb', {
    createDebugAdapterDescriptor() {
      return new vscode.DebugAdapterExecutable('lldb-vscode', ['--port', '0']);
    },
  });
  const sbp = new vscode.SourceBreakpoint({
    uri: vscode.Uri.file('/workspace/main.rs'),
    range: new vscode.Range(10, 0, 10, 0),
  });
  const fbp = new vscode.FunctionBreakpoint('main');
  vscode.debug.addBreakpoints([sbp, fbp]);
  assert(vscode.debug.breakpoints.length === 2, 'debug.breakpoints length mismatch');
  const resolvedCfg = await rpc.handleIncomingRequest('debug.resolveDebugConfiguration', {
    debugType: 'lldb',
    debugConfiguration: { type: 'lldb', name: 'Launch', request: 'launch' },
  });
  assert(resolvedCfg && resolvedCfg.stopOnEntry === true, 'debug.resolveDebugConfiguration failed');
  const adapterDesc = await rpc.handleIncomingRequest('debug.createDebugAdapterDescriptor', {
    debugType: 'lldb',
    session: { id: 'dbg_1', type: 'lldb', name: 'Launch' },
  });
  assert(adapterDesc && adapterDesc.command === 'lldb-vscode', 'debug.createDebugAdapterDescriptor failed');
  vscode.debug.removeBreakpoints([sbp, fbp]);
  assert(vscode.debug.breakpoints.length === 0, 'debug.removeBreakpoints failed');
  dbgFactoryDisp.dispose();
  dbgCfgDisp.dispose();

  console.log('[Test] Testing vscode.tasks API...');
  const cargoTask = new vscode.Task(
    { type: 'cargo', command: 'test' },
    vscode.TaskScope.Workspace,
    'cargo test',
    'cargo',
    new vscode.ShellExecution('cargo test')
  );
  cargoTask.group = vscode.TaskGroup.Build;
  const taskDisp = vscode.tasks.registerTaskProvider('cargo', {
    provideTasks() {
      return [cargoTask];
    },
    resolveTask(t) {
      return t;
    },
  });
  const providedTasks = await rpc.handleIncomingRequest('tasks.provideTasks', { taskType: 'cargo' });
  assert(Array.isArray(providedTasks) && providedTasks.length === 1, 'tasks.provideTasks failed');
  const resolvedTask = await rpc.handleIncomingRequest('tasks.resolveTask', {
    taskType: 'cargo',
    task: cargoTask,
  });
  assert(resolvedTask && resolvedTask.name === 'cargo test', 'tasks.resolveTask failed');
  taskDisp.dispose();

  console.log('[Test] Testing vscode.tests API...');
  const testCtrl = vscode.tests.createTestController('rust-tests', 'Rust Tests');
  const testItem = testCtrl.createTestItem('unit_1', 'test_one', vscode.Uri.file('/workspace/lib.rs'));
  testCtrl.items.replace([testItem]);
  assert(testItem.toJSON().parent === undefined, 'TestItem.toJSON must strip parent');
  let testsRan = 0;
  const runProfile = testCtrl.createRunProfile(
    'Run',
    vscode.TestRunProfileKind.Run,
    (request) => {
      const run = testCtrl.createTestRun(request, 'Quick Run', true);
      run.started(testItem);
      run.passed(testItem, 12);
      run.end();
      testsRan += 1;
    },
    true
  );
  await rpc.handleIncomingRequest('tests.runTests', {
    controllerId: 'rust-tests',
    request: { profileId: runProfile.id },
  });
  assert(testsRan === 1, 'tests.runTests failed');
  runProfile.dispose();
  testCtrl.dispose();

  console.log('[Test] Testing vscode.authentication API...');
  const authSessions = [{ id: 'sess_1', accessToken: 'tok_123', account: { id: 'u1', label: 'dev' }, scopes: ['read'] }];
  const authDisp = vscode.authentication.registerAuthenticationProvider('github', 'GitHub', {
    getSessions() {
      return authSessions;
    },
    createSession(scopes) {
      return { id: 'sess_2', accessToken: 'tok_new', account: { id: 'u1', label: 'dev' }, scopes };
    },
    removeSession() {
      return true;
    },
  });
  const gotSessions = await rpc.handleIncomingRequest('authentication.getSessions', {
    providerId: 'github',
    scopes: ['read'],
  });
  assert(Array.isArray(gotSessions) && gotSessions.length === 1, 'authentication.getSessions failed');
  const createdSession = await rpc.handleIncomingRequest('authentication.createSession', {
    providerId: 'github',
    scopes: ['repo'],
  });
  assert(createdSession && createdSession.id === 'sess_2', 'authentication.createSession failed');
  const removedSession = await rpc.handleIncomingRequest('authentication.removeSession', {
    providerId: 'github',
    sessionId: 'sess_2',
  });
  assert(removedSession === true, 'authentication.removeSession failed');
  let crossTypeFailed = false;
  try {
    await rpc.invokeProvider('SourceControl', 'github', 'provideOriginalResource');
  } catch (err) {
    crossTypeFailed = String(err && err.message ? err.message : err).indexOf('Provider not found: SourceControl:github') !== -1;
  }
  assert(crossTypeFailed, 'invokeProvider must not collide across provider types');
  authDisp.dispose();

  console.log('[Test] Testing vscode.window Webview & Custom Editors...');
  assert(typeof vscode.window.createWebviewPanel === 'function', 'createWebviewPanel missing');
  assert(typeof vscode.window.registerCustomEditorProvider === 'function', 'registerCustomEditorProvider missing');
  const wvPanel = vscode.window.createWebviewPanel('catCoding', 'Cat Coding', 1, { enableScripts: true });
  wvPanel.webview.html = '<h1>Meow</h1>';
  assert(wvPanel.webview.html === '<h1>Meow</h1>', 'webview html mismatch');
  wvPanel.webview.postMessage({ type: 'ping' });
  wvPanel.reveal(1);
  wvPanel.dispose();

  const textCustomDisp = vscode.window.registerCustomEditorProvider(
    'catEdit.catScratch',
    {
      resolveCustomTextEditor(doc, webviewPanel) {
        webviewPanel.webview.html = '<p>' + doc.getText() + '</p>';
      },
    },
    { webviewOptions: { enableScripts: true } }
  );
  const resolvedTextCustom = await rpc.handleIncomingRequest('window.resolveCustomEditor', {
    viewType: 'catEdit.catScratch',
    uri: 'file:///demo.cscratch',
    panelId: 'ce_1',
    text: 'hello scratch',
  });
  assert(resolvedTextCustom && resolvedTextCustom.resolved === true, 'resolveCustomTextEditor failed');
  assert(resolvedTextCustom.html === '<p>hello scratch</p>', 'resolveCustomTextEditor html mismatch');
  textCustomDisp.dispose();

  let customDocSaved = false;
  let customDocSavedAs = false;
  let customDocReverted = false;
  let customDocBackedUp = false;
  const binCustomDisp = vscode.window.registerCustomEditorProvider(
    'catEdit.hexViewer',
    {
      openCustomDocument(docUri) {
        return { uri: docUri, dispose() {} };
      },
      resolveCustomEditor(doc, webviewPanel) {
        webviewPanel.webview.html = '<div>' + doc.uri.toString() + '</div>';
      },
      saveCustomDocument() {
        customDocSaved = true;
      },
      saveCustomDocumentAs() {
        customDocSavedAs = true;
      },
      revertCustomDocument() {
        customDocReverted = true;
      },
      backupCustomDocument() {
        customDocBackedUp = true;
        return { id: 'backup_1', delete() {} };
      },
    },
    { supportsMultipleEditorsPerDocument: true }
  );
  await rpc.handleIncomingRequest('window.openCustomDocument', {
    viewType: 'catEdit.hexViewer',
    uri: 'file:///demo.bin',
  });
  const resolvedBinCustom = await rpc.handleIncomingRequest('window.resolveCustomEditor', {
    viewType: 'catEdit.hexViewer',
    uri: 'file:///demo.bin',
    panelId: 'ce_2',
  });
  assert(resolvedBinCustom && resolvedBinCustom.resolved === true, 'resolveCustomEditor failed');
  await rpc.handleIncomingRequest('window.saveCustomDocument', {
    viewType: 'catEdit.hexViewer',
    uri: 'file:///demo.bin',
  });
  await rpc.handleIncomingRequest('window.saveCustomDocumentAs', {
    viewType: 'catEdit.hexViewer',
    uri: 'file:///demo.bin',
    destination: 'file:///demo_copy.bin',
  });
  await rpc.handleIncomingRequest('window.revertCustomDocument', {
    viewType: 'catEdit.hexViewer',
    uri: 'file:///demo.bin',
  });
  const backupRes = await rpc.handleIncomingRequest('window.backupCustomDocument', {
    viewType: 'catEdit.hexViewer',
    uri: 'file:///demo.bin',
    context: { destination: 'file:///backup.bin' },
  });
  assert(customDocSaved && customDocSavedAs && customDocReverted && customDocBackedUp, 'CustomEditor lifecycle hooks failed');
  assert(backupRes && backupRes.id === 'backup_1', 'backupCustomDocument failed');
  binCustomDisp.dispose();

  console.log('[Test] Testing M1 TreeDataProvider, Window, Workspace, Commands, Env, Extensions, FS...');
  assert(vscode.TreeItemCollapsibleState.None === 0, 'TreeItemCollapsibleState.None mismatch');
  assert(vscode.TreeItemCollapsibleState.Collapsed === 1, 'TreeItemCollapsibleState.Collapsed mismatch');
  assert(vscode.TreeItemCollapsibleState.Expanded === 2, 'TreeItemCollapsibleState.Expanded mismatch');
  assert(vscode.ConfigurationTarget.Global === 1, 'ConfigurationTarget.Global mismatch');
  assert(vscode.ConfigurationTarget.Workspace === 2, 'ConfigurationTarget.Workspace mismatch');
  assert(vscode.ConfigurationTarget.WorkspaceFolder === 3, 'ConfigurationTarget.WorkspaceFolder mismatch');
  assert(vscode.EndOfLine.LF === 1 && vscode.EndOfLine.CRLF === 2, 'EndOfLine mismatch');

  const sampleTreeItem = new vscode.TreeItem('src', vscode.TreeItemCollapsibleState.Collapsed);
  sampleTreeItem.id = 'node_src';
  sampleTreeItem.contextValue = 'folder';
  assert(sampleTreeItem.label === 'src' && sampleTreeItem.collapsibleState === 1, 'TreeItem constructor mismatch');

  const treeDisp = vscode.window.registerTreeDataProvider('m1.treeView', {
    getTreeItem(el) {
      return el;
    },
    getChildren(el) {
      if (!el) {
        return [sampleTreeItem];
      }
      const child = new vscode.TreeItem(vscode.Uri.file('/workspace/src/main.rs'));
      child.id = 'node_main';
      return [child];
    },
  });
  const rootTreeItems = await rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'm1.treeView' });
  assert(Array.isArray(rootTreeItems) && rootTreeItems.length === 1 && rootTreeItems[0].label === 'src', 'treeView.getChildren root failed');
  const childTreeItems = await rpc.handleIncomingRequest('TreeDataProvider.getChildren', {
    providerId: 'm1.treeView',
    element: 'node_src',
  });
  assert(Array.isArray(childTreeItems) && childTreeItems.length === 1 && childTreeItems[0].label === 'main.rs', 'TreeDataProvider.getChildren child failed');
  const tvHandle = vscode.window.createTreeView('m1.treeViewHandle', {
    treeDataProvider: { getTreeItem: (e) => e, getChildren: () => [] },
  });
  assert(tvHandle.viewId === 'm1.treeViewHandle', 'createTreeView viewId mismatch');
  tvHandle.dispose();
  treeDisp.dispose();

  const cancelSrc = new vscode.CancellationTokenSource();
  cancelSrc.cancel();
  const qpCancelled = await vscode.window.showQuickPick(['a', 'b'], {}, cancelSrc.token);
  assert(qpCancelled === undefined, 'showQuickPick with cancelled token must return undefined');
  const ibCancelled = await vscode.window.showInputBox({ prompt: 'name' }, cancelSrc.token);
  assert(ibCancelled === undefined, 'showInputBox with cancelled token must return undefined');
  await vscode.window.showInformationMessage('M1 info', 'OK');

  const wsCfg = vscode.workspace.getConfiguration('editor');
  wsCfg._data = { fontSize: 14, minimap: { enabled: true }, 'files.exclude': { '**/.git': true } };
  assert(wsCfg.get('minimap.enabled') === true, 'WorkspaceConfiguration nested get failed');
  assert(wsCfg.has('minimap.enabled') === true, 'WorkspaceConfiguration nested has failed');
  assert(wsCfg.get('missing.key', 42) === 42, 'WorkspaceConfiguration defaultValue failed');
  assert(wsCfg.inspect('fontSize').key === 'editor.fontSize', 'WorkspaceConfiguration inspect key mismatch');
  await vscode.workspace.openTextDocument('/workspace/main.rs');

  let cmdRanWith = null;
  const cmdDisp = vscode.commands.registerCommand('m1.greet', (who) => {
    cmdRanWith = who;
    return 'hello ' + who;
  });
  const cmdOut = await rpc.handleIncomingRequest('commands.executeCommand', {
    command: 'm1.greet',
    args: ['rustcode'],
  });
  assert(cmdRanWith === 'rustcode' && cmdOut === 'hello rustcode', 'commands.executeCommand incoming failed');
  const cmdList = await vscode.commands.getCommands(true);
  assert(Array.isArray(cmdList) && cmdList.includes('m1.greet'), 'commands.getCommands headless fallback failed');
  cmdDisp.dispose();

  await vscode.env.clipboard.writeText('clip-m1');
  await vscode.env.clipboard.readText();
  await vscode.env.openExternal(vscode.Uri.parse('https://rustcode.dev'));
  await vscode.env.openExternal('https://rustcode.dev/docs');
  const invalidExtOpen = await vscode.env.openExternal(null);
  assert(invalidExtOpen === false, 'env.openExternal(null) must resolve false');

  vscode.extensions._setAll([
    {
      id: 'RustCode.sample',
      extensionPath: '/ext/sample',
      isActive: false,
      packageJSON: { name: 'sample', publisher: 'RustCode' },
      extensionKind: 1,
    },
  ]);
  const foundExt = vscode.extensions.getExtension('rustcode.sample');
  assert(foundExt && foundExt.id === 'RustCode.sample' && foundExt.extensionUri.fsPath === '/ext/sample', 'extensions.getExtension case-insensitive failed');

  const nodeFs = require('./node-compat/fs');
  assert(typeof nodeFs.promises.readFile === 'function' && typeof nodeFs.promises.stat === 'function', 'node-compat/fs missing methods');
  await nodeFs.promises.stat('/tmp/nonexistent');

  const winUri = vscode.Uri.file('C:\\workspace\\src\\lib.rs');
  assert(winUri.path === '/C:/workspace/src/lib.rs', 'Uri.file Windows path normalization failed: ' + winUri.path);
  assert(winUri.toString() === 'file:///C:/workspace/src/lib.rs', 'Uri.file Windows toString failed: ' + winUri.toString());
  assert(winUri.fsPath === 'C:\\workspace\\src\\lib.rs', 'Uri.file Windows fsPath failed: ' + winUri.fsPath);

  const parsedWinUri = vscode.Uri.parse('file:///C:/workspace/src/lib.rs');
  assert(parsedWinUri.fsPath === 'C:\\workspace\\src\\lib.rs', 'Uri.parse Windows fsPath failed: ' + parsedWinUri.fsPath);

  const uncUri = vscode.Uri.file('\\\\server\\share\\dir\\app.rs');
  assert(uncUri.toString() === 'file://server/share/dir/app.rs', 'Uri.file UNC toString failed: ' + uncUri.toString());
  assert(uncUri.fsPath === '\\\\server\\share\\dir\\app.rs', 'Uri.file UNC fsPath failed: ' + uncUri.fsPath);

  const winTreeItem = new vscode.TreeItem(winUri);
  assert(winTreeItem.label === 'lib.rs', 'TreeItem Windows path basename extraction failed');

  // rpc-bridge normalizeTreeItem with plain object resourceUri (no TreeItem constructor)
  const winTreeDisp = vscode.window.registerTreeDataProvider('m1.winTree', {
    getTreeItem: (el) => el,
    getChildren: () => [{ resourceUri: { fsPath: 'C:\\workspace\\src\\main.rs' } }],
  });
  const winItems = await rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'm1.winTree' });
  assert(Array.isArray(winItems) && winItems.length === 1 && winItems[0].label === 'main.rs', 'rpc-bridge normalizeTreeItem Windows backslash basename failed');
  winTreeDisp.dispose();

  // workspace.openTextDocument must treat C:\... as a file path, NOT a `c:` URI scheme
  let capturedOpenPayload = null;
  const origRpcCall = rpc.call;
  rpc.call = async (method, params) => {
    if (method === 'workspace.openTextDocument') {
      capturedOpenPayload = params;
      return null;
    }
    return origRpcCall.call(rpc, method, params);
  };
  try {
    await vscode.workspace.openTextDocument('C:\\workspace\\src\\main.rs');
    assert(
      capturedOpenPayload && capturedOpenPayload.uri === 'file:///C:/workspace/src/main.rs',
      'workspace.openTextDocument misparsed Windows drive path: ' + JSON.stringify(capturedOpenPayload)
    );
    await vscode.workspace.openTextDocument('untitled:Untitled-1');
    assert(
      capturedOpenPayload && capturedOpenPayload.uri === 'untitled://Untitled-1' || capturedOpenPayload.uri === 'untitled:Untitled-1',
      'workspace.openTextDocument broke untitled URI: ' + JSON.stringify(capturedOpenPayload)
    );
  } finally {
    rpc.call = origRpcCall;
  }

  // node-compat/path Windows & POSIX flavor support
  assert(path.win32.sep === '\\' && path.win32.delimiter === ';', 'path.win32 sep/delimiter failed');
  assert(path.posix.sep === '/' && path.posix.delimiter === ':', 'path.posix sep/delimiter failed');
  assert(path.win32.basename('C:\\workspace\\src\\lib.rs') === 'lib.rs', 'path.win32.basename failed');
  assert(path.win32.dirname('C:\\workspace\\src\\lib.rs') === 'C:\\workspace\\src', 'path.win32.dirname failed');
  assert(path.win32.dirname('C:\\lib.rs') === 'C:\\', 'path.win32.dirname root failed');
  assert(path.win32.extname('C:\\workspace\\src\\lib.rs') === '.rs', 'path.win32.extname failed');
  assert(path.win32.isAbsolute('C:\\workspace\\src\\lib.rs') === true, 'path.win32.isAbsolute failed');
  assert(path.win32.isAbsolute('C:relative.rs') === false, 'path.win32.isAbsolute drive-relative failed');
  assert(path.win32.normalize('C:\\workspace\\src\\..\\lib.rs') === 'C:\\workspace\\lib.rs', 'path.win32.normalize failed');
  assert(path.win32.resolve('C:\\workspace\\src', '../lib.rs') === 'C:\\workspace\\lib.rs', 'path.win32.resolve failed');
  assert(path.win32.relative('C:\\workspace\\src', 'C:\\workspace\\tests\\a.rs') === '..\\tests\\a.rs', 'path.win32.relative failed');
  assert(path.posix.dirname('/a/b/c.rs') === '/a/b', 'path.posix.dirname failed');
  assert(path.posix.isAbsolute('C:\\a') === false, 'path.posix.isAbsolute Windows path must be false');
  assert(path.posix.relative('/a/b', '/a/c/d.rs') === '../c/d.rs', 'path.posix.relative failed');
  const parsedPosix = path.posix.parse('/a/b/file.txt');
  assert(parsedPosix.dir === '/a/b' && parsedPosix.name === 'file' && parsedPosix.ext === '.txt', 'path.posix.parse failed');
  assert(path.posix.format(parsedPosix) === '/a/b/file.txt', 'path.posix.format failed');
  const parsedWin = path.win32.parse('C:\\a\\b\\file.txt');
  assert(parsedWin.root === 'C:\\' && parsedWin.dir === 'C:\\a\\b' && parsedWin.base === 'file.txt', 'path.win32.parse failed');
  assert(path.win32.format(parsedWin) === 'C:\\a\\b\\file.txt', 'path.win32.format failed');

  // Differential parity tests against Node.js built-in require('path')
  const nodeBuiltinPath = require('path');
  const diffCases = [
    ['win32', 'dirname', ['C:/']],
    ['win32', 'parse', ['C:/']],
    ['win32', 'format', [path.win32.parse('C:/')]],
    ['win32', 'extname', ['C:..']],
    ['win32', 'parse', ['C:..']],
    ['win32', 'extname', ['.']],
    ['win32', 'extname', ['..']],
    ['win32', 'parse', ['.']],
    ['win32', 'parse', ['..']],
    ['win32', 'parse', ['.gitignore']],
    ['win32', 'format', [path.win32.parse('.gitignore')]],
    ['win32', 'parse', ['./.gitignore']],
    ['win32', 'dirname', ['\\\\server\\share']],
    ['win32', 'basename', ['\\\\server\\share']],
    ['win32', 'basename', ['\\\\server\\share\\']],
    ['win32', 'parse', ['\\\\server\\share']],
    ['win32', 'format', [path.win32.parse('\\\\server\\share')]],
    ['win32', 'dirname', ['//server/share/dir/file.txt']],
    ['win32', 'parse', ['//server/share/dir/file.txt']],
    ['win32', 'format', [path.win32.parse('//server/share/dir/file.txt')]],
    ['win32', 'relative', ['\\\\server\\share1\\a', '\\\\server\\share2\\b']],
    ['win32', 'relative', ['\\\\server1\\share\\a', '\\\\server2\\share\\b']],
    ['posix', 'extname', ['.']],
    ['posix', 'extname', ['..']],
    ['posix', 'parse', ['.']],
    ['posix', 'parse', ['..']],
    ['posix', 'parse', ['.gitignore']],
    ['posix', 'format', [path.posix.parse('.gitignore')]],
    ['posix', 'parse', ['./.gitignore']],
    ['posix', 'parse', ['./']],
    ['posix', 'dirname', ['/a/../b/']],
    ['posix', 'parse', ['/a/../b/']],
    ['posix', 'format', [path.posix.parse('/a/../b/')]],
    ['posix', 'dirname', ['a//b/']],
  ];
  if (process.platform === 'win32') {
    diffCases.push(['win32', 'resolve', ['C:foo']]);
    diffCases.push(['win32', 'resolve', ['c:foo']]);
    diffCases.push(['win32', 'resolve', ['D:../foo']]);
  }
  for (const [flavor, method, args] of diffCases) {
    const actual = JSON.stringify(path[flavor][method](...args));
    const expected = JSON.stringify(nodeBuiltinPath[flavor][method](...args));
    assert(
      actual === expected,
      `path.${flavor}.${method}(${JSON.stringify(args)}) mismatch: actual=${actual} expected=${expected}`
    );
  }

  console.log('[Test] Testing node-compat/fs APIs (mocked RPC + sync + callbacks + encodings)...');
  const savedCall = rpc.call;
  const fsRpcLog = [];
  rpc.call = async (method, params) => {
    fsRpcLog.push({ method, params });
    if (method === 'fs.readdir' && params.withFileTypes) {
      return [
        { name: 'dirA', isFile: false, isDirectory: true, isSymbolicLink: false },
        { name: 'fileB.txt', isFile: true, isDirectory: false, isSymbolicLink: false },
        { name: 'linkC', isFile: false, isDirectory: false, isSymbolicLink: true },
      ];
    }
    if (method === 'fs.mkdir') {
      return { created: true, path: params.path, firstCreated: params.options && params.options.recursive ? '/tmp/first' : null };
    }
    if (method === 'fs.readFile') {
      if (params.path === '/missing') {
        return { error: { code: 'ENOENT', message: 'no such file' } };
      }
      if (params.encoding === null) {
        return [104, 101, 108, 108, 111]; // "hello"
      }
      return 'hello';
    }
    if (method === 'fs.writeFile' || method === 'fs.access' || method === 'fs.rm' || method === 'fs.rename' || method === 'fs.copyFile' || method === 'fs.unlink') {
      return { ok: true };
    }
    return null;
  };
  try {
    const dirents = await nodeFs.promises.readdir('/demo', { withFileTypes: true });
    assert(dirents.length === 3 && dirents[0] instanceof nodeFs.Dirent, 'readdir withFileTypes must return Dirent instances');
    assert(dirents[0].name === 'dirA' && dirents[0].isDirectory() && !dirents[0].isFile(), 'Dirent dir check failed');
    assert(dirents[1].isFile() && !dirents[1].isDirectory(), 'Dirent file check failed');
    assert(dirents[2].isSymbolicLink() && !dirents[2].isFile() && !dirents[2].isDirectory(), 'Dirent symlink check failed');

    const mkRec = await nodeFs.promises.mkdir('/tmp/first/second', { recursive: true });
    assert(mkRec === '/tmp/first', 'mkdir recursive must return firstCreated');
    const mkNonRec = await nodeFs.promises.mkdir('/tmp/single');
    assert(mkNonRec === undefined, 'mkdir non-recursive must return undefined');

    await nodeFs.promises.writeFile('/enc/b64.txt', Buffer.from('hello').toString('base64'), 'base64');
    const lastWrite = fsRpcLog[fsRpcLog.length - 1];
    assert(Array.isArray(lastWrite.params.data) && lastWrite.params.data.join(',') === '104,101,108,108,111', 'writeFile base64 encoding failed');

    const b64Read = await nodeFs.promises.readFile('/enc/b64.txt', 'base64');
    assert(b64Read === Buffer.from('hello').toString('base64'), 'readFile base64 decoding failed');
    const hexRead = await nodeFs.promises.readFile('/enc/b64.txt', { encoding: 'hex' });
    assert(hexRead === '68656c6c6f', 'readFile hex decoding failed');
    const bufRead = await nodeFs.promises.readFile('/enc/b64.txt', null);
    assert(bufRead instanceof Uint8Array && bufRead.length === 5, 'readFile binary failed');

    let caughtCode = null;
    try {
      await nodeFs.promises.readFile('/missing');
    } catch (e) {
      caughtCode = e.code;
    }
    assert(caughtCode === 'ENOENT', 'unwrapResult error code propagation failed');

    assert((await nodeFs.promises.access('/a', nodeFs.constants.R_OK)) === undefined, 'access must resolve undefined');
    assert((await nodeFs.promises.rm('/a', { recursive: true, force: true })) === undefined, 'rm must resolve undefined');
    assert((await nodeFs.promises.rename('/a', '/b')) === undefined, 'rename must resolve undefined');
    assert((await nodeFs.promises.copyFile('/a', '/b', nodeFs.constants.COPYFILE_EXCL)) === undefined, 'copyFile must resolve undefined');

    await new Promise((resolve, reject) => {
      nodeFs.writeFile('/cb/test.txt', '6869', 'hex', (err) => {
        if (err) return reject(err);
        const w = fsRpcLog[fsRpcLog.length - 1];
        assert(Array.isArray(w.params.data) && w.params.data[0] === 0x68 && w.params.data[1] === 0x69, 'callback writeFile options dropped');
        resolve();
      });
    });

    rpc.setSyncHandler((method, params) => {
      if (method === 'fs.stat') {
        return params.path === '/exists' ? { size: 4, isFile: true, isDirectory: false, isSymbolicLink: false } : { error: { code: 'ENOENT', message: 'missing' } };
      }
      if (method === 'fs.readFile') {
        return 'sync-ok';
      }
      if (method === 'fs.mkdir') {
        return { created: true, path: params.path, firstCreated: '/sync-first' };
      }
      if (method === 'fs.readdir') {
        return [{ name: 's.txt', isFile: true, isDirectory: false, isSymbolicLink: false }];
      }
      return { ok: true };
    });
    assert(nodeFs.existsSync('/exists') === true, 'existsSync true failed');
    assert(nodeFs.existsSync('/nope') === false, 'existsSync false failed');
    assert(nodeFs.readFileSync('/exists', 'utf8') === 'sync-ok', 'readFileSync failed');
    assert(nodeFs.mkdirSync('/sync-first/a', { recursive: true }) === '/sync-first', 'mkdirSync failed');
    assert(nodeFs.readdirSync('/d', { withFileTypes: true })[0].isFile() === true, 'readdirSync withFileTypes failed');
    assert(nodeFs.statSync('/exists').size === 4, 'statSync failed');
    nodeFs.writeFileSync('/w', 'abc', 'utf8');
    nodeFs.accessSync('/exists', nodeFs.constants.F_OK);
    nodeFs.copyFileSync('/a', '/b', nodeFs.constants.COPYFILE_EXCL);
    nodeFs.renameSync('/a', '/b');
    nodeFs.rmSync('/b', { recursive: true });
    nodeFs.unlinkSync('/c');

    // Callback safety: omitted/non-function callbacks on readFile/writeFile must not throw
    nodeFs.readFile('/exists', 'utf8');
    nodeFs.readFile('/missing', 'utf8', null);
    nodeFs.writeFile('/w', 'abc', 'utf8');
    nodeFs.writeFile('/w', 'abc', undefined, 'not-a-fn');
  } finally {
    rpc.setSyncHandler(null);
    rpc.call = savedCall;
  }

  console.log('[Test] Testing rpc-bridge live stdio transport (PassThrough streams)...');
  const { PassThrough } = require('stream');
  for (const wireFormat of ['content-length', 'newline-json']) {
    const clientIn = new PassThrough();
    const clientOut = new PassThrough();
    const hostChunks = [];
    clientOut.on('data', (chunk) => hostChunks.push(chunk));

    rpc.startStdioTransport({ input: clientIn, output: clientOut, format: wireFormat, redirectConsole: false });
    try {
      assert(rpc.isLiveStdioMode() === true, 'isLiveStdioMode should be true');
      assert(nodeFs.existsSync('/any') === false, 'existsSync in live stdio without sync op must return false');
      let syncErrCode = null;
      try {
        nodeFs.readFileSync('/any');
      } catch (e) {
        syncErrCode = e.code;
      }
      assert(syncErrCode === 'ENOSYS', 'readFileSync in live stdio without sync op must throw ENOSYS');

      // Send an incoming ping request split into arbitrary 3-byte chunks (testing UTF-8 framing)
      const pingMsg = JSON.stringify({
        type: 'request',
        id: 77,
        method: 'extHost.ping',
        params: { greeting: 'こんにちは 🚀' },
      });
      const frameBuf = wireFormat === 'newline-json'
        ? Buffer.from(pingMsg + '\n', 'utf8')
        : Buffer.concat([
          Buffer.from('Content-Length: ' + Buffer.byteLength(pingMsg, 'utf8') + '\r\n\r\n', 'ascii'),
          Buffer.from(pingMsg, 'utf8'),
        ]);
      for (let off = 0; off < frameBuf.length; off += 3) {
        clientIn.write(frameBuf.subarray(off, off + 3));
      }

      // Wait briefly for async response frame
      await new Promise((r) => setTimeout(r, 20));
      const rawOut = Buffer.concat(hostChunks).toString('utf8');
      assert(rawOut.includes('"pong":true') && rawOut.includes('こんにちは 🚀'), 'Stdio ping response mismatch: ' + rawOut);

      // Test outbound rpc.call + structured error rejection
      const callPromise = rpc.call('fs.readFile', { path: 'missing.txt' });
      await new Promise((r) => setTimeout(r, 10));
      const allOut = Buffer.concat(hostChunks).toString('utf8');
      const idxReq = allOut.indexOf('"fs.readFile"');
      assert(idxReq !== -1, 'Outbound request not written to stdio');
      const reqMatch = allOut.slice(Math.max(0, idxReq - 40), idxReq + 40).match(/"id":(\d+)/);
      assert(reqMatch, 'Missing id in outbound request');
      const outReqId = Number(reqMatch[1]);
      const errRespJson = JSON.stringify({
        type: 'response',
        id: outReqId,
        result: null,
        error: { code: 'ENOENT', message: 'missing file over stdio' },
      });
      const errFrame = wireFormat === 'newline-json'
        ? Buffer.from(errRespJson + '\n', 'utf8')
        : Buffer.concat([
          Buffer.from('Content-Length: ' + Buffer.byteLength(errRespJson, 'utf8') + '\r\n\r\n', 'ascii'),
          Buffer.from(errRespJson, 'utf8'),
        ]);
      clientIn.write(errFrame);
      let rejectedCode = null;
      try {
        await callPromise;
      } catch (e) {
        rejectedCode = e.code;
      }
      assert(rejectedCode === 'ENOENT', 'rpc.call structured error rejection failed');
    } finally {
      rpc.stopStdioTransport();
      assert(rpc.isLiveStdioMode() === false, 'isLiveStdioMode should be false after stop');
    }
  }

  console.log('[Test] Testing rpc-bridge globalThis singleton sharing across module reloads / drive-case...');
  {
    const bridgeResolved = require.resolve('./rpc-bridge.js');
    const clientIn = new PassThrough();
    const clientOut = new PassThrough();
    rpc.startStdioTransport({ input: clientIn, output: clientOut, format: 'content-length', redirectConsole: false });
    try {
      const savedCacheEntry = require.cache[bridgeResolved];
      delete require.cache[bridgeResolved];
      let rpcAlt = require('./rpc-bridge.js');
      try {
        assert(rpcAlt.isLiveStdioMode() === true, 'Re-evaluated rpc-bridge must share active stdioTransport');
        rpc.registerCommandHandler('singleton.cmd', (v) => v + 1);
        assert(rpcAlt.hasCommandHandler('singleton.cmd') === true, 'Re-evaluated rpc-bridge must share commandHandlers');

        if (process.platform === 'win32') {
          const lowerDrivePath = bridgeResolved.replace(/^[A-Za-z]:/, (m) => m.toLowerCase());
          const upperDrivePath = bridgeResolved.replace(/^[A-Za-z]:/, (m) => m.toUpperCase());
          const rpcLower = require(lowerDrivePath);
          const rpcUpper = require(upperDrivePath);
          assert(rpcLower.isLiveStdioMode() === true && rpcUpper.isLiveStdioMode() === true, 'Windows drive-case require must share stdioTransport');
          assert(rpcLower.hasCommandHandler('singleton.cmd') && rpcUpper.hasCommandHandler('singleton.cmd'), 'Windows drive-case require must share commandHandlers');
          delete require.cache[lowerDrivePath];
          delete require.cache[upperDrivePath];
        }
      } finally {
        rpc.unregisterCommandHandler('singleton.cmd');
        require.cache[bridgeResolved] = savedCacheEntry;
      }
    } finally {
      rpc.stopStdioTransport();
    }
  }

  console.log('[Test] Testing M1.2 F4-core fireAndForget safety & commands parity...');
  {
    let unhandledCount = 0;
    const onUnhandled = () => { unhandledCount++; };
    process.on('unhandledRejection', onUnhandled);
    const clientIn = new PassThrough();
    const clientOut = new PassThrough();
    rpc.startStdioTransport({ input: clientIn, output: clientOut, format: 'content-length', redirectConsole: false });
    try {
      const cmdDisp = vscode.commands.registerCommand('m12.localOnly', (x) => `local:${x}`);
      assert(typeof rpc.getCommandHandler('m12.localOnly') === 'function', 'getCommandHandler missing');
      const cmds = await Promise.race([
        vscode.commands.getCommands(true),
        new Promise((r) => setTimeout(() => r(null), 50)),
      ]);
      // Even before Rust responds or when mocked null, localCommands should be merged if resolved
      const savedCall2 = rpc.call;
      rpc.call = async () => ['rust.builtin', '_internal'];
      try {
        const merged = await vscode.commands.getCommands(true);
        assert(merged.includes('m12.localOnly'), 'getCommands must merge localCommands');
        assert(merged.includes('rust.builtin'), 'getCommands must keep Rust commands');
        assert(!merged.includes('_internal'), 'getCommands(true) must filter _internal');
      } finally {
        rpc.call = savedCall2;
      }

      // Exercise fire-and-forget APIs across namespaces right before stopStdioTransport()
      const sb = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 1);
      sb.text = 'M1.2';
      sb.show();
      sb.hide();
      sb.dispose();
      const ch = vscode.window.createOutputChannel('M12Chan');
      ch.appendLine('line');
      ch.clear();
      ch.dispose();
      const term = vscode.window.createTerminal('M12Term');
      term.sendText('echo hi');
      term.dispose();
      const diags = vscode.languages.createDiagnosticCollection('m12');
      diags.set(vscode.Uri.file('/a.rs'), []);
      diags.dispose();
      const sc = vscode.scm.createSourceControl('m12scm', 'M12');
      const grp = sc.createResourceGroup('g1', 'G1');
      grp.resourceStates = [];
      sc.dispose();
      const tc = vscode.tests.createTestController('m12test', 'M12T');
      const run = tc.createTestRun(undefined, 'r1');
      run.end();
      tc.dispose();
      const dbgDisp = vscode.debug.registerDebugConfigurationProvider('node', {});
      dbgDisp.dispose();
      const taskDisp = vscode.tasks.registerTaskProvider('shell', {});
      taskDisp.dispose();
      const serDisp = vscode.notebooks.registerNotebookSerializer('nb', {});
      serDisp.dispose();
      const authDisp = vscode.authentication.registerAuthenticationProvider('gh', 'GH', {});
      authDisp.dispose();
      cmdDisp.dispose();
    } finally {
      rpc.stopStdioTransport();
    }
    await new Promise((r) => setTimeout(r, 20));
    process.off('unhandledRejection', onUnhandled);
    assert(unhandledCount === 0, `Expected 0 unhandledRejections, got ${unhandledCount}`);

    let notFoundErrCode = null;
    try {
      await rpc.handleIncomingRequest('commands.executeCommand', { command: 'no.such.cmd', args: [] });
    } catch (e) {
      notFoundErrCode = e.code;
    }
    assert(notFoundErrCode === 'COMMAND_NOT_FOUND', `Expected COMMAND_NOT_FOUND, got ${notFoundErrCode}`);
  }

  console.log('[Test] Testing M1.2 F5 window prompts, editors, and TreeDataProvider synthetic handles...');
  {
    const savedCall = rpc.call;
    try {
      // null -> undefined for show*Message, showQuickPick, showInputBox; "" preserved for showInputBox
      rpc.call = async () => null;
      assert((await vscode.window.showInformationMessage('hi', 'OK')) === undefined, 'showInformationMessage null->undefined');
      assert((await vscode.window.showWarningMessage('warn')) === undefined, 'showWarningMessage null->undefined');
      assert((await vscode.window.showErrorMessage('err')) === undefined, 'showErrorMessage null->undefined');
      assert((await vscode.window.showQuickPick(['a', 'b'])) === undefined, 'showQuickPick null->undefined');
      assert((await vscode.window.showInputBox({ prompt: 'p' })) === undefined, 'showInputBox null->undefined');

      rpc.call = async () => '';
      assert((await vscode.window.showInputBox({ prompt: 'empty' })) === '', 'showInputBox empty string must be preserved');

      // Mid-flight CancellationToken + validateInput + onDidSelectItem
      const cts = new vscode.CancellationTokenSource();
      let cancelledClientId = null;
      let validateRes = null;
      let selectedItemObserved = null;
      rpc.call = async (method, params) => {
        if (method === 'window.showInputBox') {
          assert(params.hasValidateInput === true, 'hasValidateInput must be true');
          validateRes = await rpc.handleIncomingRequest('window.validateInput', {
            clientId: params.clientId,
            value: 'bad',
          });
          setTimeout(() => cts.cancel(), 5);
          return await new Promise((resolve) => setTimeout(() => resolve('late'), 50));
        }
        if (method === 'window.showQuickPick') {
          assert(params.hasOnDidSelectItem === true, 'hasOnDidSelectItem must be true');
          rpc.dispatchNotification('window.quickPickDidSelectItem', {
            clientId: params.clientId,
            item: { label: 'Beta' },
          });
          return { label: 'Beta' };
        }
        if (method === 'window.cancelPrompt') {
          cancelledClientId = params.clientId;
          return { cancelled: true };
        }
        return null;
      };
      const midCancelRes = await vscode.window.showInputBox({
        validateInput: (v) => (v === 'bad' ? 'Too bad' : null),
      }, cts.token);
      assert(midCancelRes === undefined, 'mid-flight cancelled showInputBox must resolve undefined');
      assert(validateRes === 'Too bad', 'validateInput callback failed');
      assert(typeof cancelledClientId === 'string' && cancelledClientId.length > 0, 'window.cancelPrompt not sent');

      const qpPick = await vscode.window.showQuickPick(['Alpha', 'Beta'], {
        onDidSelectItem: (item) => { selectedItemObserved = item; },
      });
      assert(qpPick === 'Beta', 'showQuickPick string label resolution failed');
      assert(selectedItemObserved === 'Beta', 'onDidSelectItem must unwrap string label');

      // TextEditorProxy, showTextDocument, activeTextEditor, visibleTextEditors
      const activeChangedEvents = [];
      const activeSub = vscode.window.onDidChangeActiveTextEditor((ed) => activeChangedEvents.push(ed));
      rpc.call = async (method, params) => {
        if (method === 'window.showTextDocument') {
          return {
            id: 'ed_1',
            document: {
              uri: params.uri,
              fileName: '/ws/demo.rs',
              languageId: 'rust',
              version: 1,
              text: 'fn a() {}\n',
              isDirty: false,
              isUntitled: false,
              eol: 1,
              lineCount: 2,
            },
            selection: { anchor: { line: 0, character: 0 }, active: { line: 0, character: 2 }, start: { line: 0, character: 0 }, end: { line: 0, character: 2 }, isReversed: false },
            viewColumn: vscode.ViewColumn.Two,
          };
        }
        return null;
      };
      const editor = await vscode.window.showTextDocument(vscode.Uri.file('/ws/demo.rs'), vscode.ViewColumn.Two);
      assert(editor && editor.document.languageId === 'rust', 'showTextDocument must return TextEditorProxy');
      assert(editor.viewColumn === vscode.ViewColumn.Two, 'editor.viewColumn mismatch');
      assert(vscode.window.activeTextEditor && vscode.window.activeTextEditor.id === 'ed_1', 'activeTextEditor not updated');
      assert(vscode.window.visibleTextEditors.length >= 1, 'visibleTextEditors not updated');
      rpc.dispatchNotification('window.onDidChangeActiveTextEditor', {
        id: 'ed_2',
        document: { uri: 'file:///ws/b.rs', languageId: 'rust', text: 'x' },
      });
      assert(activeChangedEvents.length === 1 && activeChangedEvents[0].document.languageId === 'rust', 'onDidChangeActiveTextEditor wrapper failed');
      activeSub.dispose();

      // TreeDataProvider synthetic handles without .id + onDidChangeTreeData + dispose()
      const treeCalls = [];
      rpc.call = async (method, params) => {
        treeCalls.push({ method, params });
        return { ok: true };
      };
      const emitter = new vscode.EventEmitter();
      const objA = { name: 'FolderA', children: [{ name: 'Child1' }] };
      const treeView = vscode.window.createTreeView('m12.tree', {
        treeDataProvider: {
          onDidChangeTreeData: emitter.event,
          getTreeItem(el) {
            const ti = new vscode.TreeItem(el.name, el.children ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
            ti.resourceUri = vscode.Uri.file('C:\\proj\\src\\file.rs');
            return ti;
          },
          getChildren(el) {
            return el ? (el.children || []) : [objA];
          },
          resolveTreeItem(item) {
            item.tooltip = 'Resolved Tooltip';
            return item;
          },
        },
      });
      const rootItems = await rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'm12.tree' });
      assert(rootItems.length === 1 && typeof rootItems[0].id === 'string' && rootItems[0].id.startsWith('__rustcode_tree_m12.tree_'), 'synthetic tree handle missing');
      assert(typeof rootItems[0].resourceUri === 'string' && rootItems[0].resourceUri.includes('file.rs'), 'resourceUri missing on normalized TreeItem');
      const childItems = await rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'm12.tree', element: rootItems[0].id });
      assert(childItems.length === 1 && childItems[0].label === 'Child1', 'synthetic tree handle child expansion failed');
      const resolvedItem = await rpc.handleIncomingRequest('treeView.resolveTreeItem', { providerId: 'm12.tree', element: rootItems[0].id });
      assert(resolvedItem.tooltip === 'Resolved Tooltip', 'resolveTreeItem failed');

      emitter.fire(objA);
      const changeCall = treeCalls.find((c) => c.method === 'treeView.onDidChangeTreeData');
      assert(changeCall && changeCall.params.elementIds[0] === rootItems[0].id, 'onDidChangeTreeData failed to map element to synthetic handle');
      treeView.dispose();
      const callsBefore = treeCalls.length;
      emitter.fire();
      assert(treeCalls.length === callsBefore, 'treeView.dispose() must unsubscribe onDidChangeTreeData');
    } finally {
      rpc.call = savedCall;
    }
  }

  console.log('[Test] Testing M1.2 F6 workspace folders, config cache, CRLF TextDocumentProxy, Uri, fs...');
  {
    // Uri.parse Windows drive path & percent-decoding fsPath
    const winUri = vscode.Uri.parse('C:\\Users\\Alice\\my file.rs');
    assert(winUri.scheme === 'file', 'Uri.parse must treat C:\\... as file scheme, got ' + winUri.scheme);
    assert(winUri.fsPath === 'C:\\Users\\Alice\\my file.rs', 'Uri.parse fsPath mismatch: ' + winUri.fsPath);
    const pctWin = vscode.Uri.parse('file:///C:/my%20dir/100%25.rs');
    assert(pctWin.fsPath === 'C:\\my dir\\100%.rs', 'Uri.fsPath percent-decode mismatch: ' + pctWin.fsPath);
    const pctPosix = vscode.Uri.parse('file:///tmp/my%20file.rs');
    assert(pctPosix.fsPath === '/tmp/my file.rs', 'POSIX Uri.fsPath percent-decode mismatch: ' + pctPosix.fsPath);

    // Workspace folders + getWorkspaceFolder + asRelativePath + onDidChangeWorkspaceFolders
    const wsEvents = [];
    const wsSub = vscode.workspace.onDidChangeWorkspaceFolders((e) => wsEvents.push(e));
    rpc.dispatchNotification('workspace.onDidChangeWorkspaceFolders', {
      folders: [
        { uri: 'file:///C:/Work/App', name: 'App', path: 'C:\\Work\\App', index: 0 },
        { uri: 'file:///C:/Work/Lib', name: 'Lib', path: 'C:\\Work\\Lib', index: 1 },
      ],
    });
    assert(vscode.workspace.workspaceFolders.length === 2, 'workspaceFolders hydration failed');
    assert(vscode.workspace.rootPath === 'C:\\Work\\App', 'rootPath mismatch: ' + vscode.workspace.rootPath);
    const foundFolder = vscode.workspace.getWorkspaceFolder(vscode.Uri.parse('file:///c:/work/lib/src/index.ts'));
    assert(foundFolder && foundFolder.name === 'Lib', 'getWorkspaceFolder case-insensitive Windows match failed');
    assert(vscode.workspace.asRelativePath('C:\\Work\\Lib\\src\\index.ts') === 'Lib/src/index.ts', 'asRelativePath multi-root failed');
    assert(vscode.workspace.asRelativePath('C:\\Work\\Lib\\src\\index.ts', false) === 'src/index.ts', 'asRelativePath includeWorkspaceFolder=false failed');
    assert(wsEvents.length === 1 && wsEvents[0].added.length === 2, 'onDidChangeWorkspaceFolders event failed');
    wsSub.dispose();

    // onDidChangeConfiguration + affectsConfiguration + WorkspaceConfiguration.update() refresh
    const cfgEvents = [];
    const cfgSub = vscode.workspace.onDidChangeConfiguration((e) => cfgEvents.push(e));
    rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
      affectedKeys: ['editor.tabSize'],
      config: { editor: { tabSize: 2, fontSize: 16 } },
    });
    assert(cfgEvents.length === 1 && cfgEvents[0].affectsConfiguration('editor.tabSize') === true, 'affectsConfiguration exact match failed');
    assert(cfgEvents[0].affectsConfiguration('editor') === true, 'affectsConfiguration parent section failed');
    assert(cfgEvents[0].affectsConfiguration('files.autoSave') === false, 'affectsConfiguration unrelated section should be false');
    assert(vscode.workspace.getConfiguration('editor').get('tabSize') === 2, 'cached getConfiguration failed');
    cfgSub.dispose();

    const savedCall = rpc.call;
    try {
      rpc.call = async (method) => {
        if (method === 'workspace.updateConfiguration') return { updated: true, key: 'editor.tabSize' };
        if (method === 'workspace.getConfiguration') return { tabSize: 8, fontSize: 16 };
        if (method === 'workspace.saveTextDocument') return true;
        if (method === 'workspace.findFiles') return ['file:///C:/Work/App/src/main.rs'];
        if (method === 'fs.stat') return { size: 12, isFile: true, isDirectory: false, isSymbolicLink: false, mtimeMs: 100 };
        if (method === 'fs.readdir') return [{ name: 'a.rs', isFile: true, isDirectory: false, isSymbolicLink: false }];
        return null;
      };
      const edCfg = vscode.workspace.getConfiguration('editor');
      await edCfg.update('tabSize', 8, vscode.ConfigurationTarget.Workspace);
      assert(edCfg.get('tabSize') === 8, 'WorkspaceConfiguration.update must refresh local _data');

      // TextDocumentProxy CRLF getText / offsetAt / positionAt / validatePosition / validateRange / save
      const crlfDoc = new vscode.workspace._TextDocumentProxy({
        uri: 'file:///C:/Work/App/crlf.txt',
        text: 'alpha\r\nbeta\r\ngamma',
        eol: vscode.EndOfLine.CRLF,
        isDirty: true,
      });
      assert(crlfDoc.lineCount === 3, 'CRLF lineCount mismatch');
      assert(crlfDoc.lineAt(0).text === 'alpha', 'lineAt(0) must strip \\r');
      assert(crlfDoc.getText(new vscode.Range(0, 0, 0, 99)) === 'alpha', 'CRLF single-line getText leaked \\r: ' + JSON.stringify(crlfDoc.getText(new vscode.Range(0, 0, 0, 99))));
      assert(crlfDoc.getText(new vscode.Range(0, 2, 1, 2)) === 'pha\r\nbe', 'CRLF multi-line getText mismatch');
      assert(crlfDoc.offsetAt(new vscode.Position(1, 0)) === 7, 'CRLF offsetAt line 1 mismatch');
      assert(crlfDoc.positionAt(7).line === 1 && crlfDoc.positionAt(7).character === 0, 'CRLF positionAt(7) mismatch');
      assert(crlfDoc.positionAt(6).line === 0 && crlfDoc.positionAt(6).character === 5, 'CRLF positionAt mid-CRLF clamp mismatch');
      assert((await crlfDoc.save()) === true && crlfDoc.isDirty === false, 'TextDocumentProxy.save failed');

      // findFiles + workspace.fs
      const foundUris = await vscode.workspace.findFiles('**/*.rs');
      assert(foundUris.length === 1 && foundUris[0] instanceof vscode.Uri, 'findFiles must return Uri[]');
      const st = await vscode.workspace.fs.stat(vscode.Uri.file('/a.rs'));
      assert((st.type & vscode.FileType.File) !== 0 && st.size === 12, 'workspace.fs.stat failed');
      const dirEntries = await vscode.workspace.fs.readDirectory(vscode.Uri.file('/'));
      assert(dirEntries[0][0] === 'a.rs' && (dirEntries[0][1] & vscode.FileType.File) !== 0, 'workspace.fs.readDirectory failed');
    } finally {
      rpc.call = savedCall;
    }
  }

  console.log('[Test] Testing M1.2 F7 env.refresh, UIKind, extensions.activate, require("vscode")...');
  {
    const fs = require('fs');
    const os = require('os');
    const nodePath = require('path');
    assert(vscode.UIKind.Desktop === 1 && vscode.UIKind.Web === 2, 'UIKind enum mismatch');

    const savedCall = rpc.call;
    const tempExtDir = fs.mkdtempSync(nodePath.join(os.tmpdir(), 'rustcode_ext_js_'));
    try {
      fs.writeFileSync(
        nodePath.join(tempExtDir, 'ext.js'),
        `const vs = require('vscode');\nexports.activate = (ctx) => ({ ok: true, extId: ctx.extension.id, hasUiKind: vs.UIKind.Desktop === 1 });\n`
      );

      rpc.call = async (method) => {
        if (method === 'env.getEnvironment') {
          return {
            appName: 'RustCode',
            appRoot: tempExtDir,
            appHost: 'desktop',
            language: 'en',
            machineId: 'm1',
            sessionId: 's1',
            shell: '/bin/bash',
            uriScheme: 'rustcode',
            uiKind: 1,
          };
        }
        if (method === 'extensions.activate') {
          return { activated: true, id: 'RustCode.temp-js-ext' };
        }
        return null;
      };

      await vscode.env.refresh();
      assert(vscode.env.appRoot === tempExtDir, 'env.refresh did not hydrate appRoot');
      assert(vscode.env.shell === '/bin/bash' && vscode.env.uriScheme === 'rustcode', 'env.refresh fields mismatch');

      vscode.extensions._setAll([
        {
          id: 'RustCode.temp-js-ext',
          extensionPath: tempExtDir,
          isActive: false,
          packageJSON: { name: 'temp-js-ext', publisher: 'RustCode', main: './ext.js' },
          extensionKind: 1,
        },
      ]);
      // Case-insensitive lookup by both canonical ID and short manifest name
      assert(vscode.extensions.getExtension('rustcode.temp-js-ext') !== undefined, 'getExtension by id failed');
      const byShort = vscode.extensions.getExtension('TEMP-JS-EXT');
      assert(byShort !== undefined && byShort.id === 'RustCode.temp-js-ext', 'getExtension by short name failed');

      const exportsObj = await byShort.activate();
      assert(exportsObj && exportsObj.ok === true && exportsObj.hasUiKind === true, 'ext.activate() did not run module activate with require("vscode")');
      assert(byShort.isActive === true, 'ext.isActive must be true after activate()');

      // _setAll during onDidChange must preserve exports
      rpc.dispatchNotification('extensions.onDidChange', [
        {
          id: 'RustCode.temp-js-ext',
          extensionPath: tempExtDir,
          isActive: true,
          packageJSON: { name: 'temp-js-ext', publisher: 'RustCode', main: './ext.js' },
          extensionKind: 1,
        },
      ]);
      assert(vscode.extensions.getExtension('rustcode.temp-js-ext').exports.ok === true, 'onDidChange wiped ext.exports');

      // Test handleIncomingRequest('extensions.activate') on unmodified sample hello-world extension
      const helloPath = nodePath.resolve(__dirname, '..', 'resources', 'sample-extensions', 'hello-world');
      const helloPkg = JSON.parse(fs.readFileSync(nodePath.join(helloPath, 'package.json'), 'utf8'));
      const actHostRes = await rpc.handleIncomingRequest('extensions.activate', {
        id: 'rustcode.sample-hello-world',
        extensionPath: helloPath,
        packageJSON: helloPkg,
      });
      assert(actHostRes && actHostRes.activated === true, 'extensions.activate host request failed for hello-world');
      assert(rpc.hasCommandHandler('sample.helloWorld'), 'hello-world sample.helloWorld command was not registered');
    } finally {
      rpc.call = savedCall;
      fs.rmSync(tempExtDir, { recursive: true, force: true });
    }
  }

  console.log('[Test] Testing M1.2 Iteration 2 fixes (notebooks/tasks shutdown, token commands, TreeItem.id, Uri %, scope, activeTextEditor, openTextDocument)...');
  {
    const { PassThrough } = require('stream');

    // 1. Fire-and-forget rejection storm on stopStdioTransport() in notebooks.js & tasks.js
    const unhandled = [];
    const onUnhandled = (reason) => unhandled.push(reason);
    process.on('unhandledRejection', onUnhandled);
    const savedCallStorm = rpc.call;
    try {
      const clientIn = new PassThrough();
      const clientOut = new PassThrough();
      rpc.startStdioTransport({ input: clientIn, output: clientOut, format: 'content-length', redirectConsole: false });

      const nbCtrl = vscode.notebooks.createNotebookController('storm-ctrl', 'jupyter-notebook', 'Storm');
      const nbExec = nbCtrl.createNotebookCellExecution({ index: 0, notebookUri: 'file:///storm.ipynb' });
      const nbDoc = { uri: vscode.Uri.parse('file:///storm.ipynb') };
      const out = new vscode.NotebookCellOutput([vscode.NotebookCellOutputItem.text('hi')]);

      // Call executeTask while transport responds with task ack, then call all 7 methods unawaited before stopStdioTransport()
      const taskPromise = vscode.tasks.executeTask(new vscode.Task({ type: 'shell' }, vscode.TaskScope.Workspace, 't', 's'));
      // Respond to tasks.executeTask request id=1
      await new Promise((r) => setImmediate(r));
      const ackMsg = JSON.stringify({ type: 'response', id: 1, result: { started: true } });
      const ackFrame = `Content-Length: ${Buffer.byteLength(ackMsg, 'utf8')}\r\n\r\n${ackMsg}`;
      clientIn.write(ackFrame);
      const taskExec = await taskPromise;

      nbExec.start();
      nbExec.end(true);
      nbExec.clearOutput();
      nbExec.replaceOutput(out);
      nbExec.appendOutput(out);
      nbCtrl.updateNotebookAffinity(nbDoc, 1);
      taskExec.terminate();

      rpc.stopStdioTransport();
      await new Promise((r) => setTimeout(r, 30));
      assert(unhandled.length === 0, 'Expected 0 unhandledRejections on stopStdioTransport, got ' + unhandled.length);
      nbCtrl.dispose();
    } finally {
      process.off('unhandledRejection', onUnhandled);
      rpc.stopStdioTransport();
      rpc.call = savedCallStorm;
    }

    // 2. commands.registerCommand out-of-order disposal with same function ref & split module instance
    {
      const savedCallCmd = rpc.call;
      const savedNotify = rpc.notify;
      const sentUnreg = [];
      rpc.call = async (method, params) => {
        if (method === 'commands.unregisterCommand') sentUnreg.push(params.command);
        return null;
      };
      try {
        const sameFn = () => 'same-ok';
        const d1 = vscode.commands.registerCommand('iter2.sameFn', sameFn);
        const d2 = vscode.commands.registerCommand('iter2.sameFn', sameFn);
        d1.dispose();
        assert(sentUnreg.length === 0, 'Disposing old registration with same fn ref must not unregister newer one');
        assert((await vscode.commands.executeCommand('iter2.sameFn')) === 'same-ok', 'Newer handler must still execute');
        d2.dispose();
        assert(sentUnreg.length === 1 && sentUnreg[0] === 'iter2.sameFn', 'Disposing active registration must unregister');

        sentUnreg.length = 0;
        const commandsPath = require.resolve('./vscode-api/commands');
        const dOld = vscode.commands.registerCommand('iter2.splitMod', () => 'old');
        delete require.cache[commandsPath];
        const commandsReloaded = require('./vscode-api/commands');
        const dNew = commandsReloaded.registerCommand('iter2.splitMod', () => 'new');
        dOld.dispose();
        assert(sentUnreg.length === 0, 'Split module old disposable must not unregister newer handler');
        assert((await vscode.commands.executeCommand('iter2.splitMod')) === 'new', 'executeCommand must invoke latest shared handler');
        dNew.dispose();
        assert(sentUnreg.length === 1, 'Active split module disposable must unregister');
      } finally {
        rpc.call = savedCallCmd;
        rpc.notify = savedNotify;
      }
    }

    // 3. TreeDataProvider onDidChangeTreeData(el) when .id is on TreeItem + TreeItem(undefined) resourceUri fallback
    {
      const savedCallTree = rpc.call;
      const treeEvents = [];
      rpc.call = async (method, params) => {
        treeEvents.push({ method, params });
        return null;
      };
      try {
        const emitter = new vscode.EventEmitter();
        const hybridEl = { name: 'Hybrid' };
        const fallbackEl = { name: 'Fallback' };
        const disp = vscode.window.registerTreeDataProvider('iter2.tree', {
          onDidChangeTreeData: emitter.event,
          getChildren(el) {
            return el ? [] : [hybridEl, fallbackEl];
          },
          getTreeItem(el) {
            if (el === hybridEl) {
              const ti = new vscode.TreeItem('Hybrid');
              ti.id = 'hybrid_item_id';
              return ti;
            }
            const ti = new vscode.TreeItem(undefined);
            ti.resourceUri = vscode.Uri.file('C:\\proj\\src\\fallback.rs');
            return ti;
          },
        });
        const items = await rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'iter2.tree' });
        assert(items.length === 2, 'Expected 2 tree items');
        assert(items[0].id === 'hybrid_item_id', 'Expected explicit TreeItem.id');
        assert(items[1].label === 'fallback.rs', 'TreeItem(undefined) + resourceUri fallback failed: ' + items[1].label);

        emitter.fire(hybridEl);
        const notif = treeEvents.find((e) => e.method === 'treeView.onDidChangeTreeData');
        assert(notif && notif.params.elementIds[0] === 'hybrid_item_id', 'onDidChangeTreeData failed to resolve TreeItem.id');
        disp.dispose();
      } finally {
        rpc.call = savedCallTree;
      }
    }

    // 4. Uri.toString() query/fragment & literal % roundtrip
    {
      const qfUri = vscode.Uri.parse('https://example.com/docs?q=1&b=2#sec');
      assert(qfUri.query === 'q=1&b=2', 'Uri.query mismatch: ' + qfUri.query);
      assert(qfUri.fragment === 'sec', 'Uri.fragment mismatch: ' + qfUri.fragment);
      assert(qfUri.toString() === 'https://example.com/docs?q=1&b=2#sec', 'Uri.toString dropped query/fragment: ' + qfUri.toString());

      const roundtrip = vscode.Uri.parse(vscode.Uri.parse('C:\\my file%.rs').toString()).fsPath;
      assert(roundtrip === 'C:\\my file%.rs', 'Uri literal % roundtrip failed: ' + roundtrip);
      assert(vscode.Uri.parse('file:///C:/my%20file%25.rs').fsPath === 'C:\\my file%.rs', 'Encoded %25 fsPath failed');
      assert(vscode.Uri.parse('file:///c%3A/my%20file%25.rs').fsPath === 'c:\\my file%.rs', 'Encoded drive %3A fsPath failed');
      assert(vscode.Uri.parse('file:///C:/my%20file%.rs').fsPath === 'C:\\my file%.rs', 'Mixed %20 and literal % decode failed');
    }

    // 5. ConfigurationChangeEvent.affectsConfiguration(section, scope)
    {
      const cfgEvents = [];
      const sub = vscode.workspace.onDidChangeConfiguration((e) => cfgEvents.push(e));
      rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
        affectedKeys: ['editor.tabSize'],
        affectedFolderUris: ['file:///C:/Workspaces/RootA'],
      });
      rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
        affectedKeys: ['editor.tabSize'],
        affectedFolderUris: [],
      });
      assert(cfgEvents.length === 2, 'Expected 2 config events');
      assert(cfgEvents[0].affectsConfiguration('editor.tabSize', vscode.Uri.file('C:\\Workspaces\\RootA\\src\\a.rs')) === true, 'Folder A file should be affected');
      assert(cfgEvents[0].affectsConfiguration('editor.tabSize', { uri: vscode.Uri.file('C:\\Workspaces\\RootA') }) === true, 'Folder A workspaceFolder should be affected');
      assert(cfgEvents[0].affectsConfiguration('editor.tabSize', vscode.Uri.file('D:\\Projects\\App\\b.rs')) === false, 'Unrelated folder B file must not be affected');
      assert(cfgEvents[0].affectsConfiguration('editor.tabSize', vscode.Uri.file('C:\\Workspaces\\RootA2\\x.rs')) === false, 'Sibling prefix RootA2 must not be affected');
      assert(cfgEvents[0].affectsConfiguration('editor.tabSize') === true, 'Unscoped affectsConfiguration should be true');
      assert(cfgEvents[1].affectsConfiguration('editor.tabSize', vscode.Uri.file('D:\\Projects\\App\\b.rs')) === true, 'Global change must affect all scopes');
      sub.dispose();
    }

    // 6. window.activeTextEditor sync null clearing & 0-arg workspace.openTextDocument()
    {
      let syncEditor = { id: 'ed_sync', document: { uri: 'file:///a.rs', languageId: 'rust', text: 'a' } };
      const savedCallMisc = rpc.call;
      const openPayloads = [];
      rpc.call = async (method, params) => {
        if (method === 'window.getActiveTextEditor') return syncEditor;
        if (method === 'workspace.openTextDocument') {
          openPayloads.push(params);
          return { uri: 'untitled:Untitled-1', fileName: 'Untitled-1', languageId: 'plaintext', text: '', isUntitled: true };
        }
        return null;
      };
      rpc.setSyncHandler((method) => (method === 'window.getActiveTextEditor' ? syncEditor : undefined));
      try {
        assert(vscode.window.activeTextEditor && vscode.window.activeTextEditor.id === 'ed_sync', 'Expected ed_sync');
        syncEditor = null;
        assert(vscode.window.activeTextEditor === undefined, 'Expected activeTextEditor to clear to undefined when sync returns null');

        const untitled = await vscode.workspace.openTextDocument();
        assert(openPayloads.length === 1 && !('uri' in openPayloads[0]), '0-arg openTextDocument must omit uri');
        assert(untitled && untitled.isUntitled === true, '0-arg openTextDocument must return untitled proxy');
      } finally {
        rpc.setSyncHandler(null);
        rpc.call = savedCallMisc;
      }
    }
  }

  console.log('[Test] Testing M1.2 Iteration 3 fixes (Uri.toString non-authority, commands.js reverse split, webview.postMessage close safety, affectsConfiguration untitled)...');
  {
    const { PassThrough } = require('stream');

    // 1. Uri.toString() non-authority schemes (untitled:, mailto:) & canonicalDocKey roundtrip
    {
      const untitledUri = vscode.Uri.parse('untitled:Untitled-1');
      assert(untitledUri.toString() === 'untitled:Untitled-1', 'untitled Uri.toString mismatch: ' + untitledUri.toString());
      const reparsedUntitled = vscode.Uri.parse(untitledUri.toString());
      assert(reparsedUntitled.scheme === 'untitled' && reparsedUntitled.authority === '' && reparsedUntitled.path === 'Untitled-1', 'untitled roundtrip mismatch');

      const mailtoUri = vscode.Uri.parse('mailto:user@example.com');
      assert(mailtoUri.toString() === 'mailto:user@example.com', 'mailto Uri.toString mismatch: ' + mailtoUri.toString());
      const mailtoQueryUri = vscode.Uri.parse('mailto:user@example.com?subject=hi#frag');
      assert(mailtoQueryUri.toString() === 'mailto:user@example.com?subject=hi#frag', 'mailto query/fragment mismatch: ' + mailtoQueryUri.toString());

      assert(vscode.Uri.parse('file:///C:/a').toString() === 'file:///C:/a', 'file Uri.toString mismatch');
      assert(vscode.Uri.parse('http://example.com/a').toString() === 'http://example.com/a', 'http Uri.toString mismatch');
    }

    // 2. commands.js reverse split disposal (d2.dispose() must NOT fall back to Module 1 localCommands)
    {
      const savedCallCmd = rpc.call;
      const execRoutedToRust = [];
      rpc.call = async (method, params) => {
        if (method === 'commands.getCommands') return ['rust.only'];
        if (method === 'commands.executeCommand') {
          execRoutedToRust.push(params.command);
          const err = new Error(`Command not found: ${params.command}`);
          err.code = 'COMMAND_NOT_FOUND';
          throw err;
        }
        return null;
      };
      try {
        const commandsPath = require.resolve('./vscode-api/commands');
        const lowerCmds = require('./vscode-api/commands');
        const d1 = lowerCmds.registerCommand('iter3.reverseSplit', () => 'stale_lower');
        delete require.cache[commandsPath];
        const upperCmds = require('./vscode-api/commands');
        const d2 = upperCmds.registerCommand('iter3.reverseSplit', () => 'active_upper');
        d2.dispose();

        const listed = await lowerCmds.getCommands(false);
        assert(!listed.includes('iter3.reverseSplit'), 'Deleted command must not be returned via stale localCommands in getCommands');
        let notFoundCode = null;
        try {
          await lowerCmds.executeCommand('iter3.reverseSplit');
        } catch (err) {
          notFoundCode = err && err.code;
        }
        assert(notFoundCode === 'COMMAND_NOT_FOUND', 'executeCommand must route to Rust and reject, not run stale d1');
        assert(execRoutedToRust.includes('iter3.reverseSplit'), 'executeCommand should have routed to Rust');
        d1.dispose();
      } finally {
        rpc.call = savedCallCmd;
      }
    }

    // 3. webview.postMessage() unawaited before stopStdioTransport() resolves false with 0 unhandledRejections
    {
      const unhandled = [];
      const onUnhandled = (reason) => unhandled.push(reason);
      process.on('unhandledRejection', onUnhandled);
      const clientIn = new PassThrough();
      const clientOut = new PassThrough();
      try {
        let pauseCalled = false;
        const origPause = clientIn.pause.bind(clientIn);
        clientIn.pause = () => {
          pauseCalled = true;
          return origPause();
        };
        rpc.startStdioTransport({ input: clientIn, output: clientOut, format: 'content-length', redirectConsole: false });
        assert(clientIn.isPaused() === false, 'startStdioTransport must resume input stream');
        const panel = vscode.window.createWebviewPanel('iter3.wv', 'Iter3 Webview', vscode.ViewColumn.One);
        const postPromise = panel.webview.postMessage({ ping: 1 });
        rpc.stopStdioTransport();
        assert(pauseCalled === true && clientIn.isPaused() === true, 'stopStdioTransport must pause input stream');
        const postResult = await postPromise;
        await new Promise((r) => setTimeout(r, 20));
        assert(postResult === false, 'webview.postMessage on transport close must resolve false, got ' + postResult);
        assert(unhandled.length === 0, 'Expected 0 unhandledRejections from webview.postMessage, got ' + unhandled.length);
        panel.dispose();

        // Active transport RPC error must still reject
        const savedCallWv = rpc.call;
        rpc.call = async () => {
          const err = new Error('active webview failure');
          err.code = 'INTERNAL_ERROR';
          throw err;
        };
        try {
          const panel2 = vscode.window.createWebviewPanel('iter3.wv2', 'Iter3 Webview 2', vscode.ViewColumn.One);
          let caughtErr = null;
          try {
            await panel2.webview.postMessage({ ping: 2 });
          } catch (e) {
            caughtErr = e;
          }
          assert(caughtErr && caughtErr.code === 'INTERNAL_ERROR', 'Active transport error must still reject');
          panel2.dispose();
        } finally {
          rpc.call = savedCallWv;
        }
      } finally {
        process.off('unhandledRejection', onUnhandled);
        rpc.stopStdioTransport();
      }
    }

    // 4. ConfigurationChangeEvent.affectsConfiguration(section, nonFileUri)
    {
      const cfgEvents = [];
      const sub = vscode.workspace.onDidChangeConfiguration((e) => cfgEvents.push(e));
      rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
        affectedKeys: ['editor.tabSize'],
        affectedFolderUris: ['file:///C:/Workspaces/RootA'],
      });
      rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
        affectedKeys: ['editor.tabSize'],
        affectedFolderUris: [],
      });
      assert(cfgEvents.length === 2, 'Expected 2 config events');
      assert(
        cfgEvents[0].affectsConfiguration('editor.tabSize', vscode.Uri.parse('untitled:Untitled-1')) === false,
        'Folder-scoped config change must NOT affect untitled: URI'
      );
      assert(
        cfgEvents[0].affectsConfiguration('editor.tabSize', 'untitled:Untitled-1') === false,
        'Folder-scoped config change must NOT affect untitled: string'
      );
      assert(
        cfgEvents[0].affectsConfiguration('editor.tabSize', { uri: vscode.Uri.parse('https://example.com/doc') }) === false,
        'Folder-scoped config change must NOT affect https: URI'
      );
      assert(
        cfgEvents[1].affectsConfiguration('editor.tabSize', vscode.Uri.parse('untitled:Untitled-1')) === true,
        'Global config change must affect untitled: URI'
      );
      sub.dispose();
    }
  }

  console.log('All JavaScript extension host tests passed successfully!');
})();

function assert(condition, message) {
  if (!condition) {
    throw new Error('Assertion failed: ' + (message || ''));
  }
}
