'use strict';

const assert = require('assert');
const fsNative = require('fs');
const osNative = require('os');
const pathNative = require('path');
const {
  createSuite,
  extHostDir,
  loadExtHost,
} = require('./helpers');

const suite = createSuite('tier3_cross_feature');

suite.test('[F15+F8] node-compat/url + path + fs sandbox URI-to-path roundtrip and readdir withFileTypes', async () => {
  const hostRoot = extHostDir();
  const files = new Map();
  const { rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'fs.writeFile') {
        files.set(params.path, Buffer.isBuffer(params.data) ? params.data.toString('utf8') : String(params.data));
        return { written: true };
      }
      if (method === 'fs.readFile') {
        if (!files.has(params.path)) {
          return { error: { code: 'ENOENT', message: `ENOENT: ${params.path}` } };
        }
        return files.get(params.path);
      }
      if (method === 'fs.readdir') {
        if (params.withFileTypes || (params.options && params.options.withFileTypes)) {
          return [
            { name: 'alpha.txt', isFile: true, isDirectory: false, isSymbolicLink: false },
            { name: 'sub', isFile: false, isDirectory: true, isSymbolicLink: false },
          ];
        }
        return ['alpha.txt', 'sub'];
      }
      return null;
    },
  });

  try {
    const urlMod = require(pathNative.join(hostRoot, 'node-compat', 'url.js'));
    const pathMod = require(pathNative.join(hostRoot, 'node-compat', 'path.js'));
    const fsCompat = require(pathNative.join(hostRoot, 'node-compat', 'fs.js'));
    const samplePath = process.platform === 'win32'
      ? 'C:\\workspace\\proj\\alpha.txt'
      : '/workspace/proj/alpha.txt';
    const fileUrl = urlMod.pathToFileURL(samplePath);
    assert.ok(fileUrl.href.startsWith('file:///'), `Expected file:/// URL, got ${fileUrl.href}`);
    const roundtripPath = urlMod.fileURLToPath(fileUrl);
    const joined = pathMod.join(pathMod.dirname(roundtripPath), 'alpha.txt');

    await fsCompat.promises.writeFile(joined, 'cross-feature-content', 'utf8');
    const content = await fsCompat.promises.readFile(joined, 'utf8');
    assert.strictEqual(content, 'cross-feature-content');

    const dirents = await fsCompat.promises.readdir(pathMod.dirname(joined), { withFileTypes: true });
    assert.strictEqual(dirents.length, 2);
    assert.strictEqual(typeof dirents[0].isFile, 'function');
    assert.strictEqual(dirents[0].isFile(), true);
    assert.strictEqual(dirents[1].isDirectory(), true);
  } finally {
    void rpc;
    restore();
  }
});

suite.test('[F4+F5+F2] commands + window + rpc-bridge structured error propagation', async () => {
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'commands.registerCommand') return { registered: true, command: params.command };
      if (method === 'commands.executeCommand') {
        if (params.command === 'missing.cmd') {
          return { error: { code: 'ENOENT', message: 'Command not found: missing.cmd' } };
        }
        return { ok: true, command: params.command };
      }
      if (method === 'window.showInformationMessage') return 'Confirmed';
      return null;
    },
  });

  try {
    const disp = vscode.commands.registerCommand('e2e.greet', async (name) => {
      const choice = await vscode.window.showInformationMessage(`Hi ${name}`, 'Confirmed');
      return `greeted:${name}:${choice}`;
    });

    const out = await rpc.handleIncomingRequest('commands.executeCommand', {
      command: 'e2e.greet',
      args: ['RustCode'],
    });
    assert.strictEqual(out, 'greeted:RustCode:Confirmed');

    await assert.rejects(
      () => vscode.commands.executeCommand('missing.cmd'),
      (err) => err && err.code === 'ENOENT' && /Command not found/.test(err.message)
    );
    disp.dispose();
  } finally {
    restore();
  }
});

suite.test('[F15+F1] node-compat buffer + crypto + os + child_process over RPC', async () => {
  const hostRoot = extHostDir();
  const { restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'env.getOsInfo') {
        return {
          platform: process.platform,
          arch: process.arch,
          type: process.platform === 'win32' ? 'Windows_NT' : 'Linux',
          release: '6.0.0',
          homedir: osNative.homedir(),
          tmpdir: osNative.tmpdir(),
          hostname: 'rustcode-host',
          eol: process.platform === 'win32' ? '\r\n' : '\n',
        };
      }
      if (method === 'process.exec') {
        return {
          pid: 4242,
          exitCode: 0,
          signal: null,
          stdout: `exec:${params.command}${process.platform === 'win32' ? '\r\n' : '\n'}`,
          stderr: '',
        };
      }
      return null;
    },
  });

  try {
    const BufferMod = require(pathNative.join(hostRoot, 'node-compat', 'buffer.js'));
    const cryptoMod = require(pathNative.join(hostRoot, 'node-compat', 'crypto.js'));
    const osMod = require(pathNative.join(hostRoot, 'node-compat', 'os.js'));
    const cpMod = require(pathNative.join(hostRoot, 'node-compat', 'child_process.js'));
    const BufferCtor = BufferMod.Buffer || BufferMod;

    const buf = BufferCtor.from('abc', 'utf8');
    const hex = cryptoMod.createHash('sha256').update(buf).digest('hex');
    assert.strictEqual(hex, 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');

    const expectedEol = process.platform === 'win32' ? '\r\n' : '\n';
    assert.strictEqual(osMod.EOL, expectedEol);

    const execResult = await new Promise((resolve, reject) => {
      const child = cpMod.exec('echo ok', (err, stdout, stderr) => {
        if (err) reject(err);
        else resolve({ stdout, stderr, child });
      });
      assert.ok(child, 'exec must return a ChildProcess instance');
    });
    assert.ok(execResult.stdout.includes('exec:echo ok'));
  } finally {
    restore();
  }
});

suite.test('[F6+F3+F5] workspace hydration + config update + affectsConfiguration + showTextDocument', async () => {
  const rpcLog = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      rpcLog.push({ method, params });
      if (method === 'extensions.getAll') return [];
      if (method === 'workspace.getWorkspaceFolders') {
        return [{ uri: 'file:///workspace/app', name: 'app', index: 0 }];
      }
      if (method === 'workspace.getConfiguration') {
        return params && params.section === 'editor'
          ? { minimap: { enabled: true }, tabSize: 4 }
          : { editor: { minimap: { enabled: true }, tabSize: 4 } };
      }
      if (method === 'workspace.updateConfiguration') {
        return Promise.resolve({ updated: true, key: params.key });
      }
      if (method === 'workspace.openTextDocument') {
        return Promise.resolve({
          uri: 'file:///workspace/app/main.rs',
          fileName: '/workspace/app/main.rs',
          languageId: 'rust',
          version: 1,
          isDirty: false,
          isUntitled: false,
          eol: 1,
          text: 'fn main() {}\n',
          lineCount: 2,
        });
      }
      if (method === 'window.showTextDocument') {
        return Promise.resolve({
          document: {
            uri: 'file:///workspace/app/main.rs',
            fileName: '/workspace/app/main.rs',
            languageId: 'rust',
            version: 1,
            text: 'fn main() {}\n',
          },
          selection: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
          selections: [{ start: { line: 0, character: 0 }, end: { line: 0, character: 0 } }],
          viewColumn: 1,
        });
      }
      return null;
    },
  });

  try {
    assert.ok(
      Array.isArray(vscode.workspace.workspaceFolders) && vscode.workspace.workspaceFolders.length === 1,
      'workspaceFolders must hydrate from workspace.getWorkspaceFolders RPC'
    );

    const cfg = vscode.workspace.getConfiguration('editor');
    assert.strictEqual(cfg.get('tabSize'), 4);
    await cfg.update('minimap.enabled', false, vscode.ConfigurationTarget.Workspace);

    let affectsMinimap = false;
    let affectsFont = true;
    const sub = vscode.workspace.onDidChangeConfiguration((e) => {
      affectsMinimap = e.affectsConfiguration('editor.minimap.enabled');
      affectsFont = e.affectsConfiguration('editor.fontSize');
    });
    rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
      affectedKeys: ['editor.minimap.enabled'],
    });
    assert.strictEqual(affectsMinimap, true, 'affectsConfiguration must return true for changed key');
    assert.strictEqual(affectsFont, false, 'affectsConfiguration must return false for unrelated key');
    sub.dispose();

    const doc = await vscode.workspace.openTextDocument(vscode.Uri.parse('file:///workspace/app/main.rs'));
    assert.strictEqual(doc.languageId, 'rust');
    assert.strictEqual(typeof doc.offsetAt, 'function');
    assert.strictEqual(typeof doc.positionAt, 'function');

    const editor = await vscode.window.showTextDocument(doc, 1);
    assert.ok(editor && editor.document, 'window.showTextDocument must return TextEditorProxy');
    assert.ok(vscode.window.activeTextEditor, 'window.activeTextEditor must be populated');
  } finally {
    restore();
  }
});

suite.test('[F7+F1+F5] env clipboard + openExternal + extensions.activate + onDidChange', async () => {
  let clipStore = '';
  const opened = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') {
        return [{ id: 'rustcode.demo', name: 'demo', publisher: 'rustcode', version: '1.0.0', isActive: false }];
      }
      if (method === 'env.getEnvironment') {
        return {
          appName: 'RustCode',
          appRoot: process.platform === 'win32' ? 'C:\\RustCode' : '/opt/rustcode',
          shell: process.platform === 'win32' ? 'cmd.exe' : '/bin/bash',
          uriScheme: 'rustcode',
          language: 'en',
        };
      }
      if (method === 'env.clipboardWrite') {
        clipStore = params.value;
        return Promise.resolve({ written: true });
      }
      if (method === 'env.clipboardRead') {
        return Promise.resolve(clipStore);
      }
      if (method === 'env.openExternal') {
        opened.push(params.uri);
        return Promise.resolve(true);
      }
      if (method === 'extensions.activate') {
        return Promise.resolve({ activated: true, id: params.id, exports: { ready: true } });
      }
      return null;
    },
  });

  try {
    await vscode.env.clipboard.writeText('copied-from-ext');
    const read = await vscode.env.clipboard.readText();
    assert.strictEqual(read, 'copied-from-ext');
    assert.strictEqual(await vscode.env.openExternal(vscode.Uri.parse('https://example.org')), true);
    assert.strictEqual(await vscode.env.openExternal(''), false);
    assert.deepStrictEqual(opened, ['https://example.org/']);

    assert.strictEqual(vscode.env.uriScheme, 'rustcode');
    assert.notStrictEqual(vscode.env.appRoot, '/rustcode');

    let changedCount = 0;
    vscode.extensions.onDidChange(() => {
      changedCount++;
    });
    const ext = vscode.extensions.getExtension('rustcode.demo');
    assert.ok(ext, 'getExtension must find hydrated extension');
    await ext.activate();
    rpc.dispatchNotification('extensions.onDidChange', {});
    assert.ok(changedCount >= 1);
  } finally {
    restore();
  }
});

suite.test('[F5+F14+F4] TreeDataProvider + onDidChangeTreeData + TreeItem.command dispatch', async () => {
  const notifications = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      notifications.push({ method, params });
      if (method === 'commands.executeCommand') return { ran: params.command };
      return { ok: true };
    },
  });

  try {
    const emitter = new vscode.EventEmitter();
    const provider = {
      onDidChangeTreeData: emitter.event,
      getTreeItem: (el) => el,
      getChildren: (el) => {
        if (!el) {
          return [
            {
              label: 'src',
              collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
              command: { title: 'Open', command: 'outline.select', arguments: ['src'] },
            },
          ];
        }
        return [{ label: 'main.rs', collapsibleState: vscode.TreeItemCollapsibleState.None }];
      },
    };

    const disp = vscode.window.registerTreeDataProvider('projectOutlineView', provider);
    const items = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'projectOutlineView',
      element: null,
    });
    assert.strictEqual(items.length, 1);
    assert.ok(items[0].id, 'Normalized TreeItem must have stable fallback id');
    assert.strictEqual(items[0].command.command, 'outline.select');

    const children = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'projectOutlineView',
      element: items[0].id,
    });
    assert.strictEqual(children.length, 1);
    assert.strictEqual(children[0].label, 'main.rs');

    emitter.fire(undefined);
    assert.ok(
      notifications.some((n) => n.method === 'treeView.onDidChangeTreeData'),
      'Firing onDidChangeTreeData must notify Rust host'
    );

    const cmdRes = await vscode.commands.executeCommand(items[0].command.command, ...items[0].command.arguments);
    assert.deepStrictEqual(cmdRes, { ran: 'outline.select' });
    disp.dispose();
  } finally {
    restore();
  }
});

suite.test('[F12+F1+F5] tests.createTestController + runTests + failure message + window warning', async () => {
  const calls = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extensions.getAll') return [];
      calls.push({ method, params });
      return { ok: true };
    },
  });

  try {
    const ctrl = vscode.tests.createTestController('js-suite', 'JS Suite');
    const item = ctrl.createTestItem('t_fail', 'failing case', vscode.Uri.file('/tmp/spec.test.js'));
    ctrl.items.replace([item]);

    ctrl.createRunProfile('Run', vscode.TestRunProfileKind.Run, async (request) => {
      const run = ctrl.createTestRun(request, 'JS Run', true);
      run.failed(item, new vscode.TestMessage('Assertion failed: 1 !== 2'), 12);
      run.appendOutput('FAIL t_fail\r\n');
      run.end();
      await vscode.window.showWarningMessage('1 test failed', 'Rerun');
    }, true);

    await rpc.handleIncomingRequest('tests.runTests', {
      controllerId: 'js-suite',
      request: { include: ['t_fail'] },
    });

    assert.ok(calls.some((c) => c.method === 'tests.updateTestState' && c.params.state === 'failed'));
    assert.ok(calls.some((c) => c.method === 'tests.endTestRun'));
    assert.ok(calls.some((c) => c.method === 'window.showWarningMessage'));
    ctrl.dispose();
  } finally {
    restore();
  }
});

suite.test('[F15+F2+F8] util.promisify + fs callback ENOENT + path OS-aware sep/delimiter', async () => {
  const hostRoot = extHostDir();
  const { restore } = loadExtHost({
    mockRpc: (method) => {
      if (method === 'extensions.getAll') return [];
      if (method === 'fs.readFile') {
        return { error: { code: 'ENOENT', message: 'No such file' } };
      }
      return null;
    },
  });

  try {
    const utilMod = require(pathNative.join(hostRoot, 'node-compat', 'util.js'));
    const pathMod = require(pathNative.join(hostRoot, 'node-compat', 'path.js'));
    const fsCompat = require(pathNative.join(hostRoot, 'node-compat', 'fs.js'));

    const expectedSep = process.platform === 'win32' ? '\\' : '/';
    const expectedDelim = process.platform === 'win32' ? ';' : ':';
    assert.strictEqual(pathMod.sep, expectedSep, 'node-compat/path.sep must match host OS');
    assert.strictEqual(pathMod.delimiter, expectedDelim, 'node-compat/path.delimiter must match host OS');

    const readFileAsync = utilMod.promisify(fsCompat.readFile);
    await assert.rejects(
      () => readFileAsync('missing.txt', 'utf8'),
      (err) => err && err.code === 'ENOENT'
    );

    const circular = { name: 'node' };
    circular.self = circular;
    const formatted = utilMod.format('obj=%j', circular);
    assert.ok(typeof formatted === 'string' && formatted.length > 0);
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
