'use strict';

const assert = require('assert');
const path = require('path');

const vscode = require(path.resolve(__dirname, '../../../ext-host-js/vscode-api/index'));
const rpc = require(path.resolve(__dirname, '../../../ext-host-js/rpc-bridge'));

function parseFilter(argv) {
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--filter' && i + 1 < argv.length) {
      return argv[i + 1];
    }
    if (argv[i].startsWith('--filter=')) {
      return argv[i].slice('--filter='.length);
    }
  }
  return null;
}

async function main() {
  const filter = parseFilter(process.argv.slice(2));
  const cases = [];

  cases.push([
    'phase10_enums_and_data_transfer',
    async () => {
      assert.strictEqual(vscode.LogLevel.Off, 0);
      assert.strictEqual(vscode.LogLevel.Trace, 1);
      assert.strictEqual(vscode.LogLevel.Debug, 2);
      assert.strictEqual(vscode.LogLevel.Info, 3);
      assert.strictEqual(vscode.LogLevel.Warning, 4);
      assert.strictEqual(vscode.LogLevel.Error, 5);

      assert.strictEqual(vscode.FileChangeType.Changed, 1);
      assert.strictEqual(vscode.FileChangeType.Created, 2);
      assert.strictEqual(vscode.FileChangeType.Deleted, 3);

      assert.strictEqual(vscode.TreeItemCheckboxState.Unchecked, 0);
      assert.strictEqual(vscode.TreeItemCheckboxState.Checked, 1);

      const dt = new vscode.DataTransfer([
        ['text/plain', new vscode.DataTransferItem('hello')],
      ]);
      assert.strictEqual(await dt.get('text/plain').asString(), 'hello');
      dt.set('application/custom', { id: 7 });
      assert.deepStrictEqual(dt.toJSON(), {
        'text/plain': 'hello',
        'application/custom': { id: 7 },
      });
    },
  ]);

  cases.push([
    'phase10_vfs_and_text_document_content_provider',
    async () => {
      const files = new Map([['/init.txt', Buffer.from('initial-data', 'utf8')]]);
      const changeEmitter = new vscode.EventEmitter();
      const disp = vscode.workspace.registerFileSystemProvider('phase10fs', {
        onDidChangeFile: changeEmitter.event,
        stat: (u) => ({
          type: vscode.FileType.File,
          ctime: 1,
          mtime: 2,
          size: (files.get(u.path) || Buffer.alloc(0)).length,
        }),
        readDirectory: () => [['init.txt', vscode.FileType.File]],
        createDirectory: () => {},
        readFile: (u) => files.get(u.path) || Buffer.alloc(0),
        writeFile: (u, b) => {
          files.set(u.path, Buffer.from(b));
        },
        delete: (u) => {
          files.delete(u.path);
        },
        rename: () => {},
      });

      const readRpc = await rpc.handleIncomingRequest('vfs.readFile', {
        uri: 'phase10fs:///init.txt',
      });
      assert.strictEqual(Buffer.from(readRpc).toString('utf8'), 'initial-data');

      const wsTarget = vscode.Uri.parse('phase10fs:///written.txt');
      await vscode.workspace.fs.writeFile(wsTarget, Buffer.from('written-by-api', 'utf8'));
      const roundtrip = await vscode.workspace.fs.readFile(wsTarget);
      assert.strictEqual(Buffer.from(roundtrip).toString('utf8'), 'written-by-api');
      await vscode.workspace.fs.delete(wsTarget);
      assert.strictEqual(files.has('/written.txt'), false);
      disp.dispose();

      const contentDisp = vscode.workspace.registerTextDocumentContentProvider('p10doc', {
        provideTextDocumentContent: (u) => `rendered:${u.path}`,
      });
      const resolved = await rpc.handleIncomingRequest('vfs.provideTextDocumentContent', {
        uri: 'p10doc:///notes.md',
      });
      assert.strictEqual(resolved, 'rendered:/notes.md');
      contentDisp.dispose();
    },
  ]);

  cases.push([
    'phase10_webview_view_and_log_output_and_tree_dnd',
    async () => {
      const inbound = [];
      const wvDisp = vscode.window.registerWebviewViewProvider('phase10.panel', {
        resolveWebviewView(wvView) {
          wvView.title = 'Phase10 Sidebar';
          wvView.description = 'Compat View';
          wvView.webview.html = '<p>Active</p>';
          wvView.webview.onDidReceiveMessage((m) => inbound.push(m));
        },
      });
      const resolved = await rpc.handleIncomingRequest('window.resolveWebviewView', {
        viewId: 'phase10.panel',
      });
      assert.strictEqual(resolved.resolved, true);
      assert.strictEqual(resolved.html, '<p>Active</p>');

      vscode.window._dispatchWebviewViewMessage({
        viewId: 'phase10.panel',
        message: { action: 'ping', count: 1 },
      });
      assert.strictEqual(inbound.length, 1);
      assert.strictEqual(inbound[0].action, 'ping');
      wvDisp.dispose();

      const log = vscode.window.createOutputChannel('Phase10CompatLog', { log: true });
      log.setLogLevel(vscode.LogLevel.Warning);
      assert.strictEqual(log.logLevel, vscode.LogLevel.Warning);
      log.warn('warn sample');
      log.error('error sample');
      log.dispose();

      let dropSummary = '';
      const tv = vscode.window.createTreeView('phase10.treeDnd', {
        showCollapseAll: true,
        canSelectMany: true,
        manageCheckboxStateManually: false,
        dragAndDropController: {
          dragMimeTypes: ['text/uri-list'],
          dropMimeTypes: ['text/uri-list'],
          handleDrag(sources, dt) {
            dt.set('text/uri-list', sources.join('|'));
          },
          handleDrop(target, dt) {
            dropSummary = `${target}:${dt.get('text/uri-list').value}`;
          },
        },
      });
      const dragRes = await rpc.handleIncomingRequest('treeView.handleDrag', {
        viewId: 'phase10.treeDnd',
        sourceElementIds: ['item1', 'item2'],
      });
      assert.strictEqual(dragRes['text/uri-list'], 'item1|item2');
      await rpc.handleIncomingRequest('treeView.handleDrop', {
        viewId: 'phase10.treeDnd',
        targetElementId: 'dirA',
        dataTransfer: dragRes,
      });
      assert.strictEqual(dropSummary, 'dirA:item1|item2');
      tv.dispose();
    },
  ]);

  cases.push([
    'phase10_shell_integration_custom_exec_and_module_hook',
    async () => {
      const term = vscode.window.createTerminal({ name: 'p10sh', cwd: '/workspace' });
      assert(term.shellIntegration && typeof term.shellIntegration.executeCommand === 'function');
      let started = false;
      let ended = false;
      const sSub = vscode.window.onDidStartTerminalShellExecution(() => { started = true; });
      const eSub = vscode.window.onDidEndTerminalShellExecution(() => { ended = true; });
      const shellExec = await term.shellIntegration.executeCommand('cargo', ['--version']);
      assert.strictEqual(shellExec.commandLine.value, 'cargo --version');
      assert(started && ended, 'Terminal shell execution events must fire');
      sSub.dispose();
      eSub.dispose();
      term.dispose();

      let opened = false;
      const task = new vscode.Task(
        { type: 'custom-p10' },
        vscode.TaskScope.Workspace,
        'CustomTask',
        'phase10',
        new vscode.CustomExecution(async () => ({
          onDidWrite: new vscode.EventEmitter().event,
          onDidClose: new vscode.EventEmitter().event,
          open() {
            opened = true;
          },
          close() {},
        }))
      );
      const taskExec = await vscode.tasks.executeTask(task);
      await new Promise((r) => setTimeout(r, 20));
      assert.strictEqual(opened, true);
      taskExec.terminate();

      const ping = await rpc.handleIncomingRequest('extHost.ping', {});
      assert.strictEqual(ping.pong, true);
      assert(typeof ping.uptimeMs === 'number');
      assert(ping.memory && typeof ping.memory.rssBytes === 'number');

      rpc.installVscodeModuleHook();
      const diag = rpc.getModuleResolutionDiagnostics();
      assert.strictEqual(diag.hookInstalled, true);
      assert.strictEqual(diag.mode, 'native-node');
      assert.strictEqual(diag.nativeBuiltinsAvailable, true);
    },
  ]);

  let ran = 0;
  for (const [name, fn] of cases) {
    if (filter && !name.includes(filter)) {
      continue;
    }
    console.log(`[Phase 10 JS Compat] ${name}...`);
    await fn();
    ran++;
  }

  console.log(`[Phase 10 JS Compat] ${ran} case(s) PASSED`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
