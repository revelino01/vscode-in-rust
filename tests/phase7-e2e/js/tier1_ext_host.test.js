'use strict';

const assert = require('assert/strict');
const fsNative = require('fs');
const os = require('os');
const path = require('path');

const {
  createSuite,
  extHostDir,
  loadExtHost,
  loadExtension,
  repoRoot,
  restoreRequireHook,
} = require('./helpers');

const suite = createSuite('tier1_ext_host');

function loadNodeCompat(name) {
  const modPath = path.join(extHostDir(), 'node-compat', `${name}.js`);
  delete require.cache[require.resolve(modPath)];
  return require(modPath);
}

// ============================================================================
// F1: Live Rust <-> JS RPC Transport
// ============================================================================

suite.test('[F1] rpc.call and rpc.callSync roundtrip JSON over Deno.core.ops', async () => {
  const calls = [];
  const { rpc, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method, rawParams) => {
        calls.push({ sync: false, method, params: JSON.parse(rawParams) });
        if (method === 'env.clipboardRead') {
          return JSON.stringify('hello-from-op');
        }
        return JSON.stringify([]);
      },
      op_rpc_call_sync: (method, rawParams) => {
        calls.push({ sync: true, method, params: JSON.parse(rawParams) });
        if (method === 'extensions.getAll') {
          return JSON.stringify([]);
        }
        return JSON.stringify({ fontSize: 16 });
      },
    },
  });

  try {
    const syncRes = rpc.callSync('workspace.getConfiguration', { section: 'editor' });
    assert.deepEqual(syncRes, { fontSize: 16 });

    const asyncRes = await rpc.call('env.clipboardRead', {});
    assert.equal(asyncRes, 'hello-from-op');
    assert.ok(calls.some((c) => c.sync && c.method === 'workspace.getConfiguration'));
    assert.ok(calls.some((c) => !c.sync && c.method === 'env.clipboardRead'));
  } finally {
    restore();
  }
});

suite.test('[F1] rpc.dispatchNotification delivers payloads to subscribed event listeners', async () => {
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: () => null,
  });

  try {
    const received = [];
    const disp = vscode.workspace.onDidOpenTextDocument((doc) => received.push(doc));
    rpc.dispatchNotification('workspace.onDidOpenTextDocument', {
      uri: 'file:///workspace/lib.rs',
      languageId: 'rust',
    });
    assert.equal(received.length, 1);
    assert.equal(received[0].languageId, 'rust');

    disp.dispose();
    rpc.dispatchNotification('workspace.onDidOpenTextDocument', {
      uri: 'file:///workspace/other.rs',
    });
    assert.equal(received.length, 1);
  } finally {
    restore();
  }
});

suite.test('[F1] rpc.handleIncomingRequest routes commands.executeCommand to registered JS handler', async () => {
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: () => null,
  });

  try {
    const disp = vscode.commands.registerCommand('e2e.addNumbers', (a, b) => a + b);
    const sum = await rpc.handleIncomingRequest('commands.executeCommand', {
      command: 'e2e.addNumbers',
      args: [19, 23],
    });
    assert.equal(sum, 42);
    disp.dispose();
  } finally {
    restore();
  }
});

suite.test('[F1] rpc.handleIncomingRequest routes DocumentFormatting and Completion providers', async () => {
  const registered = {};
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'extHost.registerProvider' && params && params.type) {
        registered[params.type] = params.id;
      }
      return null;
    },
  });

  try {
    const compDisp = vscode.languages.registerCompletionItemProvider(
      'rust',
      {
        provideCompletionItems(uri, pos) {
          return [{ label: `fn_at_${pos.line}`, detail: uri }];
        },
      },
      '.'
    );
    const fmtDisp = vscode.languages.registerDocumentFormattingEditProvider('rust', {
      provideDocumentFormattingEdits(uri) {
        return [{ newText: `formatted:${uri}` }];
      },
    });

    const completions = await rpc.handleIncomingRequest('languages.provideCompletionItems', {
      providerId: registered.Completion,
      uri: 'file:///main.rs',
      position: { line: 4, character: 2 },
    });
    assert.ok(Array.isArray(completions));
    assert.equal(completions[0].label, 'fn_at_4');

    const edits = await rpc.handleIncomingRequest('languages.provideDocumentFormattingEdits', {
      providerId: registered.DocumentFormatting,
      uri: 'file:///main.rs',
      options: { tabSize: 4 },
    });
    assert.ok(Array.isArray(edits));
    assert.equal(edits[0].newText, 'formatted:file:///main.rs');

    compDisp.dispose();
    fmtDisp.dispose();
  } finally {
    restore();
  }
});

suite.test('[F1] loadExtension activates sample hello-world extension end-to-end via RPC', async () => {
  const rpcCalls = [];
  const sampleDir = path.join(repoRoot(), 'resources', 'sample-extensions', 'hello-world');
  const loaded = loadExtension(sampleDir, {
    mockRpc: (method, params) => {
      rpcCalls.push({ method, params });
      if (method === 'extensions.getAll') {
        return [];
      }
      return { ok: true };
    },
  });

  try {
    const subscriptions = [];
    loaded.exports.activate({ subscriptions });
    assert.equal(subscriptions.length, 1);
    assert.ok(rpcCalls.some((c) => c.method === 'commands.registerCommand' && c.params.command === 'sample.helloWorld'));

    await loaded.rpc.handleIncomingRequest('commands.executeCommand', {
      command: 'sample.helloWorld',
      args: [],
    });
    assert.ok(
      rpcCalls.some(
        (c) =>
          c.method === 'window.showInformationMessage' &&
          c.params.message === 'Hello World from RustCode Extension Host!'
      )
    );

    loaded.exports.deactivate();
    subscriptions.forEach((d) => d.dispose());
  } finally {
    loaded.restore();
  }
});

// ============================================================================
// F2: RPC Error Propagation
// ============================================================================

suite.test('[F2] fs.promises.readFile rejects with Error.code ENOENT on RPC error envelope', async () => {
  loadExtHost({
    mockRpc: (method) => {
      if (method.startsWith('fs.')) {
        return { error: { code: 'ENOENT', message: 'ENOENT: No such file or directory' } };
      }
      return [];
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    await assert.rejects(
      () => compatFs.promises.readFile('/nonexistent/file.txt', 'utf8'),
      (err) => err instanceof Error && err.code === 'ENOENT' && /No such file/i.test(err.message)
    );
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F2] fs.promises.writeFile rejects with Error.code EACCES on sandbox violation envelope', async () => {
  loadExtHost({
    mockRpc: (method) => {
      if (method.startsWith('fs.')) {
        return { error: { code: 'EACCES', message: 'EACCES: Path escapes fs_root sandbox' } };
      }
      return [];
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    await assert.rejects(
      () => compatFs.promises.writeFile('../escape.txt', 'bad'),
      (err) => err instanceof Error && err.code === 'EACCES'
    );
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F2] fs callback readFile passes Error with .code as first callback argument', async () => {
  loadExtHost({
    mockRpc: (method) => {
      if (method.startsWith('fs.')) {
        return { error: { code: 'EISDIR', message: 'EISDIR: Illegal operation on a directory' } };
      }
      return [];
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    const captured = await new Promise((resolve) => {
      compatFs.readFile('/some/dir', 'utf8', (err, data) => resolve({ err, data }));
    });
    assert.ok(captured.err instanceof Error);
    assert.equal(captured.err.code, 'EISDIR');
    assert.equal(captured.data, undefined);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F2] commands.executeCommand propagates RPC error envelope from Rust host', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method === 'commands.executeCommand') {
          return JSON.stringify({
            error: { code: 'ENOENT', message: 'Command not found: unknown.cmd' },
          });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: () => JSON.stringify([]),
    },
  });

  try {
    await assert.rejects(
      () => vscode.commands.executeCommand('unknown.cmd'),
      (err) => err instanceof Error && /Command not found/i.test(err.message)
    );
  } finally {
    restore();
  }
});

suite.test('[F2] rpc.handleIncomingRequest propagates synchronous and async JS handler errors', async () => {
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: () => null,
  });

  try {
    const d1 = vscode.commands.registerCommand('e2e.syncBoom', () => {
      const e = new Error('sync boom');
      e.code = 'E_SYNC';
      throw e;
    });
    const d2 = vscode.commands.registerCommand('e2e.asyncBoom', async () => {
      throw new Error('async boom');
    });

    await assert.rejects(
      () => rpc.handleIncomingRequest('commands.executeCommand', { command: 'e2e.syncBoom' }),
      (err) => err.message === 'sync boom' && err.code === 'E_SYNC'
    );
    await assert.rejects(
      () => rpc.handleIncomingRequest('commands.executeCommand', { command: 'e2e.asyncBoom' }),
      (err) => err.message === 'async boom'
    );

    d1.dispose();
    d2.dispose();
  } finally {
    restore();
  }
});

// ============================================================================
// F5: window.* Prompts, Editors & TreeDataProvider
// ============================================================================

suite.test('[F5] window.showInformationMessage, showWarningMessage, showErrorMessage forward items and options', async () => {
  const recorded = [];
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      recorded.push({ method, params });
      return 'Retry';
    },
  });

  try {
    const r1 = await vscode.window.showInformationMessage('Saved', 'OK');
    const r2 = await vscode.window.showWarningMessage('Disk low', { modal: true, detail: '95% full' }, 'Clean');
    const r3 = await vscode.window.showErrorMessage('Failed', 'Retry', 'Ignore');
    assert.equal(r1, 'Retry');
    assert.equal(r2, 'Retry');
    assert.equal(r3, 'Retry');
    const warnCall = recorded.find((c) => c.method === 'window.showWarningMessage');
    const errCall = recorded.find((c) => c.method === 'window.showErrorMessage');
    assert.ok(warnCall);
    assert.equal(warnCall.params.items[0].modal, true);
    assert.ok(errCall);
  } finally {
    restore();
  }
});

suite.test('[F5] window.showQuickPick maps string items back to string and supports canPickMany', async () => {
  const { vscode, restore } = loadExtHost({
    mockRpc: (_method, params) => {
      if (params.options && params.options.canPickMany) {
        return [{ label: 'alpha' }, { label: 'gamma' }];
      }
      return { label: 'beta' };
    },
  });

  try {
    const single = await vscode.window.showQuickPick(['alpha', 'beta', 'gamma']);
    assert.equal(single, 'beta');

    const multi = await vscode.window.showQuickPick(['alpha', 'beta', 'gamma'], {
      canPickMany: true,
    });
    assert.deepEqual(multi, ['alpha', 'gamma']);
  } finally {
    restore();
  }
});

suite.test('[F5] window.showInputBox preserves empty string response vs undefined cancellation', async () => {
  let reply = '';
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      if (method === 'window.showInputBox') {
        return reply;
      }
      return null;
    },
  });

  try {
    reply = '';
    const emptyAccepted = await vscode.window.showInputBox({ prompt: 'Enter suffix' });
    assert.strictEqual(emptyAccepted, '');

    reply = null;
    const cancelled = await vscode.window.showInputBox({ prompt: 'Enter suffix' });
    assert.strictEqual(cancelled, undefined);
  } finally {
    restore();
  }
});

suite.test('[F5] window.registerTreeDataProvider and createTreeView normalize TreeItems and fire onDidChangeTreeData', async () => {
  const rpcLog = [];
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: (method, params) => {
      rpcLog.push({ method, params });
      return null;
    },
  });

  try {
    const emitter = new vscode.EventEmitter();
    let items = [
      {
        id: 'node-1',
        label: 'src',
        collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
        resourceUri: vscode.Uri.file('C:\\repo\\src'),
        command: { title: 'Open', command: 'ext.openNode', arguments: ['node-1'] },
      },
    ];

    const tv = vscode.window.createTreeView('e2e.outline', {
      treeDataProvider: {
        onDidChangeTreeData: emitter.event,
        getTreeItem: (el) => el,
        getChildren: (el) => (el ? [] : items),
      },
    });

    const roots = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'e2e.outline',
      element: null,
    });
    assert.equal(roots.length, 1);
    assert.equal(roots[0].id, 'node-1');
    assert.equal(roots[0].collapsibleState, 1);
    assert.equal(roots[0].command.command, 'ext.openNode');

    emitter.fire(undefined);
    assert.ok(
      rpcLog.some((c) => c.method === 'treeView.onDidChangeTreeData'),
      'expected TreeDataProvider.onDidChangeTreeData to forward treeView.onDidChangeTreeData RPC'
    );

    tv.dispose();
  } finally {
    restore();
  }
});

suite.test('[F5] window.activeTextEditor, visibleTextEditors, and showTextDocument manage live editor proxies', async () => {
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'window.showTextDocument') {
        return {
          document: {
            uri: params.uri || 'file:///workspace/app.rs',
            fileName: '/workspace/app.rs',
            languageId: 'rust',
            version: 1,
            text: 'fn main() {}\n',
            lineCount: 2,
          },
          selection: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
          selections: [{ start: { line: 0, character: 0 }, end: { line: 0, character: 0 } }],
          viewColumn: 1,
        };
      }
      return null;
    },
  });

  try {
    assert.equal(typeof vscode.window.showTextDocument, 'function');
    assert.ok('activeTextEditor' in vscode.window);
    assert.ok(Array.isArray(vscode.window.visibleTextEditors));

    const editor = await vscode.window.showTextDocument(vscode.Uri.file('/workspace/app.rs'));
    assert.ok(editor && editor.document);
    assert.equal(editor.document.languageId, 'rust');
  } finally {
    restore();
  }
});

// ============================================================================
// F6: workspace.* Hydration, Config & Events
// ============================================================================

suite.test('[F6] workspace.workspaceFolders and rootPath hydrate from workspace.getWorkspaceFolders RPC', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async () => JSON.stringify(null),
      op_rpc_call_sync: (method) => {
        if (method === 'workspace.getWorkspaceFolders') {
          return JSON.stringify([
            { uri: 'file:///workspace/app', name: 'app', index: 0 },
          ]);
        }
        return JSON.stringify([]);
      },
    },
  });

  try {
    assert.ok(Array.isArray(vscode.workspace.workspaceFolders));
    assert.equal(vscode.workspace.workspaceFolders.length, 1);
    assert.equal(vscode.workspace.workspaceFolders[0].name, 'app');
    assert.ok(typeof vscode.workspace.rootPath === 'string' && vscode.workspace.rootPath.endsWith('app'));
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.getWorkspaceFolder and asRelativePath resolve Windows and POSIX workspace URIs', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async () => JSON.stringify(null),
      op_rpc_call_sync: (method) => {
        if (method === 'workspace.getWorkspaceFolders') {
          return JSON.stringify([
            { uri: 'file:///C:/repo/proj', name: 'proj', index: 0 },
          ]);
        }
        return JSON.stringify([]);
      },
    },
  });

  try {
    assert.equal(typeof vscode.workspace.getWorkspaceFolder, 'function');
    assert.equal(typeof vscode.workspace.asRelativePath, 'function');

    const targetUri = vscode.Uri.file('C:\\repo\\proj\\src\\main.rs');
    const folder = vscode.workspace.getWorkspaceFolder(targetUri);
    assert.ok(folder);
    assert.equal(folder.name, 'proj');

    const rel = vscode.workspace.asRelativePath(targetUri);
    assert.equal(rel.replace(/\\/g, '/'), 'src/main.rs');
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.getConfiguration supports get, has, inspect, and update with ConfigurationTarget', async () => {
  const updates = [];
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'workspace.getConfiguration') {
        return { fontSize: 16, minimap: { enabled: false } };
      }
      if (method === 'workspace.inspectConfiguration') {
        return {
          key: params.key,
          defaultValue: 14,
          globalValue: 16,
          workspaceValue: undefined,
        };
      }
      if (method === 'workspace.updateConfiguration') {
        updates.push(params);
        return { updated: true };
      }
      return [];
    },
  });

  try {
    const cfg = vscode.workspace.getConfiguration('editor');
    assert.equal(cfg.get('fontSize'), 16);
    assert.equal(cfg.has('minimap.enabled'), true);
    assert.equal(cfg.get('minimap.enabled'), false);

    const inspected = cfg.inspect('fontSize');
    assert.equal(inspected.defaultValue, 14);
    assert.equal(inspected.globalValue, 16);

    await cfg.update('fontSize', 18, vscode.ConfigurationTarget.Workspace);
    assert.equal(updates.length, 1);
    assert.equal(updates[0].key, 'editor.fontSize');
    assert.equal(updates[0].value, 18);
    assert.equal(updates[0].target, vscode.ConfigurationTarget.Workspace);
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.onDidChangeConfiguration delivers ConfigurationChangeEvent with affectsConfiguration', async () => {
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: () => [],
  });

  try {
    let seenEvent = null;
    const disp = vscode.workspace.onDidChangeConfiguration((e) => {
      seenEvent = e;
    });

    rpc.dispatchNotification('workspace.onDidChangeConfiguration', {
      affectedKeys: ['editor.fontSize', 'workbench.colorTheme'],
    });

    assert.ok(seenEvent);
    assert.equal(typeof seenEvent.affectsConfiguration, 'function');
    assert.equal(seenEvent.affectsConfiguration('editor.fontSize'), true);
    assert.equal(seenEvent.affectsConfiguration('editor'), true);
    assert.equal(seenEvent.affectsConfiguration('terminal.integrated'), false);
    disp.dispose();
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.openTextDocument supports disk URI, Windows path, and TextDocumentProxy methods', async () => {
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'workspace.openTextDocument') {
        return {
          uri: params.uri,
          fileName: 'C:\\proj\\main.rs',
          languageId: 'rust',
          version: 1,
          isDirty: false,
          isUntitled: false,
          eol: 2,
          text: 'fn a() {}\r\nfn b() {}',
          lineCount: 2,
        };
      }
      if (method === 'workspace.saveTextDocument') {
        return true;
      }
      return [];
    },
  });

  try {
    const doc = await vscode.workspace.openTextDocument('C:\\proj\\main.rs');
    assert.equal(doc.uri.scheme, 'file');
    assert.equal(doc.eol, vscode.EndOfLine.CRLF);
    assert.equal(doc.lineCount, 2);
    assert.equal(doc.lineAt(0).text, 'fn a() {}');

    assert.equal(typeof doc.offsetAt, 'function');
    assert.equal(typeof doc.positionAt, 'function');
    assert.equal(typeof doc.validateRange, 'function');
    assert.equal(typeof doc.save, 'function');

    const pos = doc.positionAt(11);
    assert.equal(pos.line, 1);
    assert.equal(pos.character, 0);
    assert.equal(doc.offsetAt(new vscode.Position(1, 3)), 14);
    assert.equal(await doc.save(), true);
  } finally {
    restore();
  }
});

// ============================================================================
// F7: env.* & extensions.* Real Backends
// ============================================================================

suite.test('[F7] env.clipboard.writeText and readText roundtrip text via RPC', async () => {
  let clip = '';
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'env.clipboardWrite') {
        clip = params.value;
        return { written: true };
      }
      if (method === 'env.clipboardRead') {
        return clip;
      }
      return [];
    },
  });

  try {
    await vscode.env.clipboard.writeText('rustcode-clipboard-test');
    const actual = await vscode.env.clipboard.readText();
    assert.equal(actual, 'rustcode-clipboard-test');
  } finally {
    restore();
  }
});

suite.test('[F7] env.openExternal sends valid Uri and string targets to env.openExternal RPC', async () => {
  const opened = [];
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'env.openExternal') {
        opened.push(params.uri);
        return true;
      }
      return [];
    },
  });

  try {
    const ok1 = await vscode.env.openExternal(vscode.Uri.parse('https://rustcode.dev/docs'));
    const ok2 = await vscode.env.openExternal('mailto:maintainers@rustcode.dev');
    assert.equal(ok1, true);
    assert.equal(ok2, true);
    assert.deepEqual(opened, ['https://rustcode.dev/docs', 'mailto:maintainers@rustcode.dev']);
  } finally {
    restore();
  }
});

suite.test('[F7] env hydrates cross-platform appRoot, shell, and uriScheme without hardcoded /rustcode', async () => {
  const expectedAppRoot = process.platform === 'win32' ? 'C:\\Program Files\\RustCode' : '/opt/rustcode';
  const expectedShell = process.platform === 'win32' ? 'cmd.exe' : '/bin/bash';
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async () => JSON.stringify(null),
      op_rpc_call_sync: (method) => {
        if (method === 'env.getEnvironment') {
          return JSON.stringify({
            appRoot: expectedAppRoot,
            shell: expectedShell,
            uriScheme: 'rustcode',
            language: 'en-US',
          });
        }
        return JSON.stringify([]);
      },
    },
  });

  try {
    assert.equal(vscode.env.appRoot, expectedAppRoot);
    assert.notEqual(vscode.env.appRoot, '/rustcode');
    assert.equal(vscode.env.shell, expectedShell);
    assert.equal(vscode.env.uriScheme, 'rustcode');
  } finally {
    restore();
  }
});

suite.test('[F7] extensions.all, getExtension, and activate reflect ExtensionLoader snapshots and update isActive', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method, rawParams) => {
        const params = JSON.parse(rawParams);
        if (method === 'extensions.activate') {
          return JSON.stringify({ activated: true, id: params.id, exports: { ready: true } });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: (method) => {
        if (method === 'extensions.getAll') {
          return JSON.stringify([
            {
              id: 'rustcode.sample-hello-world',
              extensionPath: '/ext/hello-world',
              isActive: false,
              packageJSON: { name: 'sample-hello-world', publisher: 'rustcode', version: '0.0.1' },
            },
          ]);
        }
        return JSON.stringify(null);
      },
    },
  });

  try {
    assert.equal(vscode.extensions.all.length, 1);
    const ext = vscode.extensions.getExtension('RUSTCODE.SAMPLE-HELLO-WORLD');
    assert.ok(ext);
    assert.equal(ext.isActive, false);

    const exported = await ext.activate();
    assert.equal(ext.isActive, true);
    assert.deepEqual(exported, { ready: true });
  } finally {
    restore();
  }
});

suite.test('[F7] extensions.onDidChange updates extensions.all when new extension list is pushed', async () => {
  const { vscode, rpc, restore } = loadExtHost({
    mockRpc: () => [],
  });

  try {
    let eventCount = 0;
    const disp = vscode.extensions.onDidChange(() => {
      eventCount++;
    });

    rpc.dispatchNotification('extensions.onDidChange', [
      {
        id: 'acme.theme',
        extensionPath: '/ext/acme-theme',
        isActive: true,
        packageJSON: { name: 'theme', publisher: 'acme' },
      },
    ]);

    assert.equal(eventCount, 1);
    assert.equal(vscode.extensions.all.length, 1);
    assert.equal(vscode.extensions.getExtension('acme.theme').isActive, true);
    disp.dispose();
  } finally {
    restore();
  }
});

// ============================================================================
// F8: node-compat/fs.js, path.js & M1 Cross-Platform Fixes
// ============================================================================

suite.test('[F8] node-compat/fs.js promises.readFile, writeFile (with encoding), stat, and unlink roundtrip', async () => {
  const store = new Map();
  loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'fs.writeFile') {
        store.set(params.path, params.data);
        return { written: true, path: params.path };
      }
      if (method === 'fs.readFile') {
        return store.get(params.path);
      }
      if (method === 'fs.stat') {
        return { size: 5, mtimeMs: 1234, isFile: true, isDirectory: false, isSymbolicLink: false };
      }
      if (method === 'fs.unlink') {
        store.delete(params.path);
        return { unlinked: true };
      }
      return null;
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    await compatFs.promises.writeFile('hello.txt', 'hello', { encoding: 'utf8' });
    const text = await compatFs.promises.readFile('hello.txt', 'utf8');
    assert.equal(text, 'hello');

    const st = await compatFs.promises.stat('hello.txt');
    assert.equal(st.isFile(), true);
    assert.equal(st.isDirectory(), false);
    assert.equal(st.size, 5);

    const unlinkRes = await compatFs.promises.unlink('hello.txt');
    assert.equal(unlinkRes, undefined);
    assert.equal(store.has('hello.txt'), false);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F8] node-compat/fs.js promises.readdir supports withFileTypes Dirent methods', async () => {
  loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'fs.readdir') {
        assert.equal(params.withFileTypes, true);
        return [
          { name: 'src', isFile: false, isDirectory: true, isSymbolicLink: false },
          { name: 'Cargo.toml', isFile: true, isDirectory: false, isSymbolicLink: false },
        ];
      }
      return null;
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    const entries = await compatFs.promises.readdir('.', { withFileTypes: true });
    assert.equal(entries.length, 2);
    assert.equal(entries[0].name, 'src');
    assert.equal(entries[0].isDirectory(), true);
    assert.equal(entries[0].isFile(), false);
    assert.equal(entries[1].name, 'Cargo.toml');
    assert.equal(entries[1].isFile(), true);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F8] node-compat/fs.js promises.mkdir returns undefined or first created path and supports access/rm/rename', async () => {
  const rpcCalls = [];
  loadExtHost({
    mockRpc: (method, params) => {
      rpcCalls.push({ method, params });
      if (method === 'fs.mkdir') {
        return {
          created: true,
          path: params.path,
          firstCreated: params.options && params.options.recursive ? '/ws/a' : null,
        };
      }
      return null;
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    const nonRec = await compatFs.promises.mkdir('/ws/dir');
    assert.strictEqual(nonRec, undefined);

    const rec = await compatFs.promises.mkdir('/ws/a/b', { recursive: true });
    assert.equal(rec, '/ws/a');

    assert.equal(typeof compatFs.promises.access, 'function');
    assert.equal(typeof compatFs.promises.rename, 'function');
    assert.equal(typeof compatFs.promises.rm, 'function');
    assert.equal(typeof compatFs.readFileSync, 'function');
    assert.equal(typeof compatFs.writeFileSync, 'function');
    assert.equal(typeof compatFs.existsSync, 'function');

    await compatFs.promises.access('/ws/a');
    await compatFs.promises.rename('/ws/a', '/ws/b');
    await compatFs.promises.rm('/ws/b', { recursive: true });
    assert.ok(rpcCalls.some((c) => c.method === 'fs.access'));
    assert.ok(rpcCalls.some((c) => c.method === 'fs.rename'));
    assert.ok(rpcCalls.some((c) => c.method === 'fs.rm'));
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F8] node-compat/path.js exposes OS-aware sep and delimiter matching host platform', () => {
  const compatPath = loadNodeCompat('path');
  const expectedSep = process.platform === 'win32' ? '\\' : '/';
  const expectedDelimiter = process.platform === 'win32' ? ';' : ':';
  assert.equal(compatPath.sep, expectedSep);
  assert.equal(compatPath.delimiter, expectedDelimiter);
});

suite.test('[F8] node-compat/path.js join, resolve, dirname, basename, extname handle Windows and POSIX paths', () => {
  const compatPath = loadNodeCompat('path');
  assert.equal(compatPath.win32.basename('C:\\repo\\src\\main.rs'), 'main.rs');
  assert.equal(compatPath.posix.basename('/repo/src/main.rs', '.rs'), 'main');
  assert.equal(compatPath.extname('archive.tar.gz'), '.gz');
  assert.equal(compatPath.win32.isAbsolute('C:\\Users\\dev'), true);
  assert.equal(compatPath.posix.isAbsolute('/usr/local/bin'), true);
  assert.equal(compatPath.posix.isAbsolute('relative/path.txt'), false);

  const joined = compatPath.join('src', 'sub', '..', 'lib.rs').replace(/\\/g, '/');
  assert.equal(joined, 'src/lib.rs');

  const dir = compatPath.posix.dirname('/a/b/c.txt');
  assert.equal(dir, '/a/b');
});

// ============================================================================
// F15: Expanded node-compat Polyfills
// ============================================================================

suite.test('[F15] node-compat/buffer.js supports from, alloc, concat, encodings, and BE/LE reads', () => {
  const { Buffer } = loadNodeCompat('buffer');
  const utf8 = Buffer.from('café 🚀', 'utf8');
  assert.equal(Buffer.isBuffer(utf8), true);
  assert.equal(utf8.toString('utf8'), 'café 🚀');

  const hexBuf = Buffer.from('deadbeef', 'hex');
  assert.equal(hexBuf.readUInt32BE(0), 0xdeadbeef);
  assert.equal(hexBuf.toString('base64'), Buffer.from([0xde, 0xad, 0xbe, 0xef]).toString('base64'));

  const a = Buffer.alloc(2, 0x41);
  const b = Buffer.from('BC');
  const combined = Buffer.concat([a, b]);
  assert.equal(combined.toString('utf8'), 'AABC');
  assert.equal(Buffer.isEncoding('base64url'), true);
});

suite.test('[F15] node-compat/url.js supports URL, URLSearchParams, parse, format, resolve, fileURLToPath, and pathToFileURL', () => {
  const compatUrl = loadNodeCompat('url');
  const u = new compatUrl.URL('https://example.com:8443/docs/index.html?q=rust#top');
  assert.equal(u.hostname, 'example.com');
  assert.equal(u.port, '8443');
  assert.equal(u.searchParams.get('q'), 'rust');

  const parsed = compatUrl.parse('https://example.com/a?x=1&y=2', true);
  assert.equal(parsed.query.x, '1');
  assert.equal(compatUrl.resolve('https://example.com/a/b', '../c'), 'https://example.com/c');

  const crossPlatformPath = path.join(repoRoot(), 'ext-host-js', 'rpc-bridge.js');
  const crossPlatformUrl = compatUrl.pathToFileURL(crossPlatformPath);
  assert.ok(crossPlatformUrl.href.startsWith('file:///'));
  assert.ok(crossPlatformUrl.href.endsWith('/ext-host-js/rpc-bridge.js'));
  const roundtripPath = path.resolve(compatUrl.fileURLToPath(crossPlatformUrl));
  const expectedPath = path.resolve(crossPlatformPath);
  assert.equal(
    process.platform === 'win32' ? roundtripPath.toLowerCase() : roundtripPath,
    process.platform === 'win32' ? expectedPath.toLowerCase() : expectedPath
  );

  if (process.platform === 'win32') {
    const winFileUrl = compatUrl.pathToFileURL('C:\\repo\\main.rs');
    assert.equal(winFileUrl.href, 'file:///C:/repo/main.rs');
    assert.equal(
      compatUrl.fileURLToPath('file:///C:/repo/main.rs').replace(/\\/g, '/'),
      'C:/repo/main.rs'
    );
  } else {
    const posixFileUrl = compatUrl.pathToFileURL('/repo/main.rs');
    assert.equal(posixFileUrl.href, 'file:///repo/main.rs');
    assert.equal(
      compatUrl.fileURLToPath('file:///usr/bin/node').replace(/\\/g, '/'),
      '/usr/bin/node'
    );
  }
});

suite.test('[F15] node-compat/util.js supports format, inspect, promisify, callbackify, inherits, types, TextEncoder/TextDecoder', async () => {
  const compatUtil = loadNodeCompat('util');
  assert.equal(compatUtil.format('%s=%d %%', 'port', 8080), 'port=8080 %');
  assert.match(compatUtil.inspect({ a: 1, b: [2, 3] }), /a:\s*1/);

  const asyncFn = compatUtil.promisify((x, cb) => cb(null, x * 2));
  assert.equal(await asyncFn(21), 42);

  const cbFn = compatUtil.callbackify(async (x) => x + 1);
  const cbVal = await new Promise((resolve, reject) => {
    cbFn(9, (err, v) => (err ? reject(err) : resolve(v)));
  });
  assert.equal(cbVal, 10);

  assert.equal(compatUtil.types.isUint8Array(new Uint8Array(2)), true);
  const encoded = new compatUtil.TextEncoder().encode('rustcode');
  assert.equal(new compatUtil.TextDecoder().decode(encoded), 'rustcode');
});

suite.test('[F15] node-compat/os.js returns platform, arch, type, homedir, tmpdir, cpus, and OS-aware EOL', () => {
  loadExtHost({
    denoOps: {
      op_rpc_call: async () => JSON.stringify(null),
      op_rpc_call_sync: (method) => {
        if (method === 'env.getOsInfo') {
          return JSON.stringify({
            platform: process.platform,
            arch: process.arch,
            type: process.platform === 'win32' ? 'Windows_NT' : 'Linux',
            release: '6.0.0',
            homedir: os.homedir(),
            tmpdir: os.tmpdir(),
            hostname: 'rustcode-host',
            eol: process.platform === 'win32' ? '\r\n' : '\n',
          });
        }
        return JSON.stringify([]);
      },
    },
  });

  try {
    const compatOs = loadNodeCompat('os');
    assert.equal(compatOs.platform(), process.platform);
    assert.equal(compatOs.arch(), process.arch);
    assert.ok(compatOs.homedir().length > 0);
    assert.ok(compatOs.tmpdir().length > 0);
    assert.ok(Array.isArray(compatOs.cpus()) && compatOs.cpus().length >= 1);
    assert.equal(compatOs.EOL, process.platform === 'win32' ? '\r\n' : '\n');
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F15] node-compat/crypto.js matches NIST test vectors for sha256/sha512/sha1/md5, hmac, randomUUID, and timingSafeEqual', () => {
  const compatCrypto = loadNodeCompat('crypto');
  assert.equal(
    compatCrypto.createHash('sha256').update('abc').digest('hex'),
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
  );
  assert.equal(
    compatCrypto.createHash('sha1').update('abc').digest('hex'),
    'a9993e364706816aba3e25717850c26c9cd0d89d'
  );
  assert.equal(
    compatCrypto.createHash('md5').update('abc').digest('hex'),
    '900150983cd24fb0d6963f7d28e17f72'
  );
  assert.equal(
    compatCrypto.createHash('sha512').update('abc').digest('hex'),
    'ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f'
  );
  assert.equal(
    compatCrypto.createHmac('sha256', 'key').update('The quick brown fox jumps over the lazy dog').digest('hex'),
    'f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8'
  );
  assert.match(
    compatCrypto.randomUUID(),
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
  );
  assert.equal(compatCrypto.randomBytes(16).length, 16);
  assert.equal(
    compatCrypto.timingSafeEqual(Uint8Array.from([1, 2, 3]), Uint8Array.from([1, 2, 3])),
    true
  );
});

suite.test('[F15] node-compat/child_process.js executes spawn, spawnSync, and exec via RPC and emits events', async () => {
  loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'process.exec') {
        return { exitCode: 0, stdout: `exec:${params.command}`, stderr: '' };
      }
      if (method === 'process.spawn') {
        return { pid: 1001, exitCode: 0, signal: null, stdout: 'spawn-out\n', stderr: '' };
      }
      if (method === 'process.spawnSync') {
        return { pid: 1002, status: 0, signal: null, stdout: 'sync-out', stderr: '' };
      }
      return [];
    },
  });

  try {
    const cp = loadNodeCompat('child_process');
    const syncRes = cp.spawnSync('node', ['-v']);
    assert.equal(syncRes.status, 0);
    assert.equal(syncRes.stdout, 'sync-out');

    const execRes = await new Promise((resolve, reject) => {
      cp.exec('echo ok', (err, stdout, stderr) => {
        if (err) reject(err);
        else resolve({ stdout, stderr });
      });
    });
    assert.equal(execRes.stdout, 'exec:echo ok');

    const child = cp.spawn('node', ['-e', 'console.log(1)']);
    const chunks = [];
    child.stdout.on('data', (d) => chunks.push(String(d)));
    const closeCode = await new Promise((resolve) => {
      child.on('close', (code) => resolve(code));
    });
    assert.equal(closeCode, 0);
    assert.equal(chunks.join(''), 'spawn-out\n');
  } finally {
    restoreRequireHook();
  }
});

if (require.main === module) {
  suite.run().catch((err) => {
    process.stderr.write(`${err && err.stack ? err.stack : err}\n`);
    process.exit(1);
  });
}

module.exports = suite;
