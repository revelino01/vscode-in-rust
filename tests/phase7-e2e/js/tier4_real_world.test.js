'use strict';

const assert = require('assert');
const pathNative = require('path');
const {
  createSuite,
  extHostDir,
  loadExtHost,
  loadExtension,
  repoRoot,
} = require('./helpers');

const suite = createSuite('tier4_real_world');

suite.test('Scenario 1 [F1+F3+F4+F5+F7+F11]: Load and run sample-extensions/hello-world end-to-end', async () => {
  const rpcCalls = [];
  const helloDir = pathNative.join(repoRoot(), 'resources', 'sample-extensions', 'hello-world');

  const loaded = loadExtension(helloDir, {
    mockRpc: (method, params) => {
      rpcCalls.push({ method, params });
      if (method === 'extensions.getAll') {
        return [
          {
            id: 'rustcode.sample-hello-world',
            name: 'sample-hello-world',
            publisher: 'rustcode',
            version: '0.0.1',
            isActive: false,
          },
        ];
      }
      if (method === 'window.showInformationMessage') {
        return 'Acknowledge';
      }
      return { ok: true };
    },
  });

  try {
    assert.strictEqual(loaded.manifest.name, 'sample-hello-world');
    const context = { subscriptions: [] };
    loaded.exports.activate(context);
    assert.strictEqual(context.subscriptions.length, 1);

    // Invoke registered command via incoming Rust -> JS RPC
    await loaded.rpc.handleIncomingRequest('commands.executeCommand', {
      command: 'sample.helloWorld',
      args: [],
    });

    const msgCall = rpcCalls.find((c) => c.method === 'window.showInformationMessage');
    assert.ok(msgCall, 'sample.helloWorld must trigger window.showInformationMessage');
    assert.strictEqual(msgCall.params.message, 'Hello World from RustCode Extension Host!');

    loaded.exports.deactivate();
  } finally {
    loaded.restore();
  }
});

suite.test('Scenario 4 [F1+F5+F12]: Extension-driven TestController discovery, run, failure reporting & rerun', async () => {
  const events = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      events.push({ method, params });
      return { ok: true };
    },
  });

  try {
    const controller = vscode.tests.createTestController('e2e-runner', 'E2E Runner');
    const suiteItem = controller.createTestItem('suite', 'Math Suite', vscode.Uri.file('/repo/tests/math.rs'));
    const passItem = controller.createTestItem('pass_1', 'addition passes', vscode.Uri.file('/repo/tests/math.rs'));
    const failItem = controller.createTestItem('fail_1', 'division fails', vscode.Uri.file('/repo/tests/math.rs'));
    suiteItem.children.replace([passItem, failItem]);
    controller.items.replace([suiteItem]);

    controller.createRunProfile('Default', vscode.TestRunProfileKind.Run, (request) => {
      const run = controller.createTestRun(request, 'Automated Run', true);
      run.started(passItem);
      run.passed(passItem, 3);
      run.started(failItem);
      run.failed(failItem, new vscode.TestMessage('expected 2, got 3'), 11);
      run.appendOutput('1 passed, 1 failed\n');
      run.end();
    }, true);

    await rpc.handleIncomingRequest('tests.runTests', {
      controllerId: 'e2e-runner',
      request: { include: ['suite'] },
    });

    assert.ok(events.some((e) => e.method === 'tests.createController' && e.params.id === 'e2e-runner'));
    assert.ok(events.some((e) => e.method === 'tests.updateTestState' && e.params.testId === 'pass_1' && e.params.state === 'passed'));
    assert.ok(events.some((e) => e.method === 'tests.updateTestState' && e.params.testId === 'fail_1' && e.params.state === 'failed'));
    assert.ok(events.some((e) => e.method === 'tests.appendRunOutput'));
    assert.ok(events.some((e) => e.method === 'tests.endTestRun'));
    controller.dispose();
  } finally {
    restore();
  }
});

suite.test('Scenario 5 [F4+F5+F14]: Contributed TreeView outline provider with refresh & command click', async () => {
  const calls = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      calls.push({ method, params });
      return { ok: true };
    },
  });

  try {
    let revision = 1;
    const emitter = new vscode.EventEmitter();
    const provider = {
      onDidChangeTreeData: emitter.event,
      getTreeItem: (el) => el,
      getChildren: (el) => {
        if (!el) {
          return [
            {
              id: 'root_src',
              label: `src (rev ${revision})`,
              collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
            },
          ];
        }
        return [
          {
            id: 'leaf_lib',
            label: 'lib.rs',
            collapsibleState: vscode.TreeItemCollapsibleState.None,
            command: { title: 'Refresh', command: 'projectOutline.refresh', arguments: [revision] },
          },
        ];
      },
    };

    const disp = vscode.window.registerTreeDataProvider('projectOutlineView', provider);
    const rootNodes = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'projectOutlineView',
      element: null,
    });
    assert.strictEqual(rootNodes[0].label, 'src (rev 1)');

    const childNodes = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'projectOutlineView',
      element: 'root_src',
    });
    assert.strictEqual(childNodes[0].label, 'lib.rs');

    let refreshedArg = null;
    const cmdDisp = vscode.commands.registerCommand('projectOutline.refresh', (rev) => {
      refreshedArg = rev;
      revision++;
      emitter.fire(undefined);
    });
    await rpc.handleIncomingRequest('commands.executeCommand', {
      command: childNodes[0].command.command,
      args: childNodes[0].command.arguments,
    });
    assert.strictEqual(refreshedArg, 1);
    assert.ok(calls.some((c) => c.method === 'treeView.onDidChangeTreeData'));

    const updatedRoots = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'projectOutlineView',
      element: null,
    });
    assert.strictEqual(updatedRoots[0].label, 'src (rev 2)');

    cmdDisp.dispose();
    disp.dispose();
  } finally {
    restore();
  }
});

suite.test('Scenario 6 [F1+F2+F8+F15]: Cross-platform Node polyfill sandbox workload', async () => {
  const hostRoot = extHostDir();
  const vfs = new Map();

  const { restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'fs.writeFile') {
        vfs.set(params.path, params.data);
        return { written: true };
      }
      if (method === 'fs.readFile') {
        if (!vfs.has(params.path)) {
          return { error: { code: 'ENOENT', message: `Missing: ${params.path}` } };
        }
        return vfs.get(params.path);
      }
      if (method === 'process.exec') {
        return { pid: 1001, exitCode: 0, stdout: 'node-compat-ok\n', stderr: '' };
      }
      return null;
    },
  });

  try {
    const BufferMod = require(pathNative.join(hostRoot, 'node-compat', 'buffer.js'));
    const urlMod = require(pathNative.join(hostRoot, 'node-compat', 'url.js'));
    const utilMod = require(pathNative.join(hostRoot, 'node-compat', 'util.js'));
    const osMod = require(pathNative.join(hostRoot, 'node-compat', 'os.js'));
    const cryptoMod = require(pathNative.join(hostRoot, 'node-compat', 'crypto.js'));
    const cpMod = require(pathNative.join(hostRoot, 'node-compat', 'child_process.js'));
    const pathMod = require(pathNative.join(hostRoot, 'node-compat', 'path.js'));
    const fsCompat = require(pathNative.join(hostRoot, 'node-compat', 'fs.js'));
    const BufferCtor = BufferMod.Buffer || BufferMod;
    const payload = BufferCtor.from('RustCode Phase 7', 'utf8');
    const hmac = cryptoMod.createHmac('sha256', 'secret-key').update(payload).digest('hex');
    assert.strictEqual(hmac.length, 64);

    const targetUrl = urlMod.pathToFileURL(
      process.platform === 'win32' ? 'C:\\sandbox\\artifact.json' : '/sandbox/artifact.json'
    );
    const fsPath = urlMod.fileURLToPath(targetUrl);
    const outPath = pathMod.join(pathMod.dirname(fsPath), 'artifact.json');

    await fsCompat.promises.writeFile(outPath, JSON.stringify({ hmac, os: osMod.platform() }), 'utf8');
    const saved = await fsCompat.promises.readFile(outPath, 'utf8');
    assert.ok(saved.includes(hmac));

    const out = await utilMod.promisify(cpMod.exec)('echo node-compat-ok');
    assert.ok(String(out.stdout || out).includes('node-compat-ok'));

    await assert.rejects(
      () => fsCompat.promises.readFile('/sandbox/does-not-exist.json', 'utf8'),
      (err) => err && err.code === 'ENOENT'
    );
  } finally {
    restore();
  }
});

suite.test('Scenario 7 [F3+F4+F5+F6+F11]: Workspace settings live update + QuickPick + InputBox prompt flow', async () => {
  const settings = { editor: { fontSize: 14, minimap: { enabled: true } } };
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'workspace.getConfiguration') {
        return params && params.section === 'editor' ? settings.editor : settings;
      }
      if (method === 'workspace.updateConfiguration') {
        if (params.key === 'editor.fontSize') settings.editor.fontSize = params.value;
        return { updated: true };
      }
      if (method === 'window.showQuickPick') return 'Dark+';
      if (method === 'window.showInputBox') {
        return params.options && params.options.value !== undefined ? params.options.value : '16';
      }
      if (method === 'commands.executeCommand') return { executed: params.command };
      return null;
    },
  });

  try {
    let configChanged = false;
    const disp = vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('editor.fontSize')) configChanged = true;
    });

    const themePick = await vscode.window.showQuickPick(['Dark+', 'Light+'], { placeHolder: 'Choose theme' });
    assert.strictEqual(themePick, 'Dark+');

    const fontInput = await vscode.window.showInputBox({
      prompt: 'Enter editor font size',
      value: '16',
      validateInput: (v) => (/^\d+$/.test(v) ? null : 'Must be numeric'),
    });
    assert.strictEqual(fontInput, '16');

    await vscode.workspace.getConfiguration('editor').update('fontSize', Number(fontInput), vscode.ConfigurationTarget.Workspace);
    rpc.dispatchNotification('workspace.onDidChangeConfiguration', { affectedKeys: ['editor.fontSize'] });
    assert.strictEqual(configChanged, true);
    assert.strictEqual(vscode.workspace.getConfiguration('editor').get('fontSize'), 16);

    const toggleRes = await vscode.commands.executeCommand('workbench.action.togglePanel');
    assert.deepStrictEqual(toggleRes, { executed: 'workbench.action.togglePanel' });
    disp.dispose();
  } finally {
    restore();
  }
});

suite.test('Scenario 8 [F1+F2+F5+F8]: Error recovery across bad FS, unknown commands, and cancelled tokens', async () => {
  let rpcCalledWhenCancelled = false;
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'window.showQuickPick' || method === 'window.showInputBox') {
        rpcCalledWhenCancelled = true;
      }
      if (method === 'fs.readFile') {
        return { error: { code: 'EACCES', message: 'Sandbox traversal denied' } };
      }
      if (method === 'commands.executeCommand') {
        return { error: { code: 'ERR_UNKNOWN_COMMAND', message: 'Unknown command' } };
      }
      return null;
    },
  });

  try {
    const fsCompat = require(pathNative.join(extHostDir(), 'node-compat', 'fs.js'));

    // Pre-cancelled CancellationToken must short-circuit without calling RPC
    const cts = new vscode.CancellationTokenSource();
    cts.cancel();
    const qpRes = await vscode.window.showQuickPick(['a', 'b'], {}, cts.token);
    const ibRes = await vscode.window.showInputBox({}, cts.token);
    assert.strictEqual(qpRes, undefined);
    assert.strictEqual(ibRes, undefined);
    assert.strictEqual(rpcCalledWhenCancelled, false, 'Cancelled token must not invoke prompt RPC');

    // Sandbox traversal error preserved
    await assert.rejects(
      () => fsCompat.promises.readFile('../../secret.txt', 'utf8'),
      (err) => err && err.code === 'EACCES'
    );

    // Unknown command error preserved
    await assert.rejects(
      () => vscode.commands.executeCommand('does.not.exist'),
      (err) => err && err.code === 'ERR_UNKNOWN_COMMAND'
    );
  } finally {
    restore();
  }
});

if (require.main === module) {
  suite.run().catch((err) => {
    process.stderr.write(`${err && err.stack ? err.stack : err}\n`);
    process.exit(1);
  });
}

module.exports = suite;
