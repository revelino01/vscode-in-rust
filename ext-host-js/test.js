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

  // node-compat/path Windows path support (while keeping POSIX behavior)
  assert(path.basename('C:\\workspace\\src\\lib.rs') === 'lib.rs', 'path.basename Windows failed');
  assert(path.dirname('C:\\workspace\\src\\lib.rs') === 'C:/workspace/src', 'path.dirname Windows failed');
  assert(path.dirname('C:\\lib.rs') === 'C:/', 'path.dirname Windows root failed');
  assert(path.extname('C:\\workspace\\src\\lib.rs') === '.rs', 'path.extname Windows failed');
  assert(path.isAbsolute('C:\\workspace\\src\\lib.rs') === true, 'path.isAbsolute Windows failed');
  assert(path.normalize('C:\\workspace\\src\\..\\lib.rs') === 'C:/workspace/lib.rs', 'path.normalize Windows failed');
  assert(path.resolve('C:\\workspace\\src', '../lib.rs') === 'C:/workspace/lib.rs', 'path.resolve Windows failed');

  console.log('All JavaScript extension host tests passed successfully!');
})();

function assert(condition, message) {
  if (!condition) {
    throw new Error('Assertion failed: ' + (message || ''));
  }
}
