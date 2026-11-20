'use strict';

const assert = require('assert/strict');
const path = require('path');

const {
  createSuite,
  extHostDir,
  loadExtHost,
  restoreRequireHook,
} = require('./helpers');

const suite = createSuite('tier2_ext_host');

function loadNodeCompat(name) {
  const modPath = path.join(extHostDir(), 'node-compat', `${name}.js`);
  delete require.cache[require.resolve(modPath)];
  return require(modPath);
}

// ============================================================================
// F1: Live Rust <-> JS RPC Transport (Boundary / Error)
// ============================================================================

suite.test('[F1] rpc.handleIncomingRequest rejects unknown incoming method names', async () => {
  const { rpc, restore } = loadExtHost({ mockRpc: () => null });
  try {
    await assert.rejects(
      () => rpc.handleIncomingRequest('unknown.namespace.method', {}),
      /Unknown incoming method/i
    );
  } finally {
    restore();
  }
});

suite.test('[F1] rpc.handleIncomingRequest rejects missing provider and missing command handler', async () => {
  const { rpc, restore } = loadExtHost({ mockRpc: () => null });
  try {
    await assert.rejects(
      () => rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'missing.view', element: null }),
      /Provider not found/i
    );
    await assert.rejects(
      () => rpc.handleIncomingRequest('commands.executeCommand', { command: 'missing.cmd', args: [] }),
      /Command not found/i
    );
  } finally {
    restore();
  }
});

suite.test('[F1] rpc.dispatchNotification isolates listener exceptions so remaining listeners still run', () => {
  const { vscode, rpc, restore } = loadExtHost({ mockRpc: () => null });
  const origConsoleError = console.error;
  console.error = () => {};
  try {
    let secondCalled = false;
    const d1 = vscode.workspace.onDidChangeConfiguration(() => {
      throw new Error('listener-1 failure');
    });
    const d2 = vscode.workspace.onDidChangeConfiguration(() => {
      secondCalled = true;
    });

    rpc.dispatchNotification('workspace.onDidChangeConfiguration', { affectedKeys: ['editor.fontSize'] });
    assert.equal(secondCalled, true);

    d1.dispose();
    d2.dispose();
  } finally {
    console.error = origConsoleError;
    restore();
  }
});

suite.test('[F1] rpc.call propagates malformed JSON from op_rpc_call without hanging', async () => {
  const { rpc, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async () => '{bad-json',
      op_rpc_call_sync: () => JSON.stringify([]),
    },
  });

  try {
    await assert.rejects(() => rpc.call('workspace.getConfiguration', {}), SyntaxError);
  } finally {
    restore();
  }
});

suite.test('[F1] rpc.registerObjectProxy dispose removes provider and clears cached tree elements', async () => {
  const { vscode, rpc, restore } = loadExtHost({ mockRpc: () => null });
  try {
    const disp = vscode.window.registerTreeDataProvider('e2e.disposableTree', {
      getTreeItem: (el) => el,
      getChildren: () => [{ id: 'item-1', label: 'Node 1' }],
    });

    const first = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'e2e.disposableTree',
      element: null,
    });
    assert.equal(first.length, 1);

    disp.dispose();
    await assert.rejects(
      () => rpc.handleIncomingRequest('treeView.getChildren', { providerId: 'e2e.disposableTree', element: null }),
      /Provider not found/i
    );
  } finally {
    restore();
  }
});

// ============================================================================
// F2: RPC Error Propagation (Boundary / Error)
// ============================================================================

suite.test('[F2] fs.promises.readdir, stat, mkdir, unlink all convert RPC { error: { code, message } } into Error.code', async () => {
  const codes = ['ENOENT', 'ENOTDIR', 'EEXIST', 'EPERM'];
  let idx = 0;
  loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method.startsWith('fs.')) {
          const code = codes[idx++ % codes.length];
          return JSON.stringify({ error: { code, message: `${code}: synthetic error` } });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: () => JSON.stringify([]),
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    await assert.rejects(() => compatFs.promises.readdir('/bad'), (e) => e.code === 'ENOENT');
    await assert.rejects(() => compatFs.promises.stat('/bad'), (e) => e.code === 'ENOTDIR');
    await assert.rejects(() => compatFs.promises.mkdir('/bad'), (e) => e.code === 'EEXIST');
    await assert.rejects(() => compatFs.promises.unlink('/bad'), (e) => e.code === 'EPERM');
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F2] fs callback writeFile passes Error with .code on failure and null on success', async () => {
  let shouldFail = true;
  loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method === 'fs.writeFile') {
          return JSON.stringify(
            shouldFail
              ? { error: { code: 'EINVAL', message: 'EINVAL: Invalid empty or NUL path' } }
              : { written: true }
          );
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: () => JSON.stringify([]),
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    const errRes = await new Promise((resolve) => {
      compatFs.writeFile('\0bad', 'data', (err) => resolve(err));
    });
    assert.ok(errRes instanceof Error);
    assert.equal(errRes.code, 'EINVAL');

    shouldFail = false;
    const okRes = await new Promise((resolve) => {
      compatFs.writeFile('good.txt', 'data', (err) => resolve(err));
    });
    assert.equal(okRes, null);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F2] window.showQuickPick rejects when RPC returns { error } envelope', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method === 'window.showQuickPick') {
          return JSON.stringify({ error: { code: 'EINVAL', message: 'Invalid quickPick items' } });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: () => JSON.stringify([]),
    },
  });

  try {
    await assert.rejects(
      () => vscode.window.showQuickPick(['a', 'b']),
      (err) => err instanceof Error && err.code === 'EINVAL'
    );
  } finally {
    restore();
  }
});

suite.test('[F2] workspace.updateConfiguration rejects when RPC returns { error } envelope', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method === 'workspace.updateConfiguration') {
          return JSON.stringify({ error: { code: 'EACCES', message: 'Read-only configuration target' } });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: (method) => {
        if (method === 'workspace.getConfiguration') {
          return JSON.stringify({});
        }
        return JSON.stringify([]);
      },
    },
  });

  try {
    const cfg = vscode.workspace.getConfiguration('editor');
    await assert.rejects(
      () => cfg.update('fontSize', 999, 1),
      (err) => err instanceof Error && err.code === 'EACCES'
    );
  } finally {
    restore();
  }
});

suite.test('[F2] env.openExternal rejects or propagates error on RPC { error } envelope', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method === 'env.openExternal') {
          return JSON.stringify({ error: { code: 'EIO', message: 'Launcher failed' } });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: () => JSON.stringify([]),
    },
  });

  try {
    await assert.rejects(
      () => vscode.env.openExternal('https://example.com'),
      (err) => err instanceof Error && err.code === 'EIO'
    );
  } finally {
    restore();
  }
});

// ============================================================================
// F5: window.* Prompts, Editors & TreeDataProvider (Boundary / Error)
// ============================================================================

suite.test('[F5] window.showQuickPick and showInputBox immediately return undefined when CancellationToken is cancelled', async () => {
  let rpcInvoked = false;
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      if (method.startsWith('window.')) {
        rpcInvoked = true;
      }
      return [];
    },
  });

  try {
    const cts = new vscode.CancellationTokenSource();
    cts.cancel();
    assert.strictEqual(await vscode.window.showQuickPick(['a', 'b'], {}, cts.token), undefined);
    assert.strictEqual(await vscode.window.showInputBox({ prompt: 'Name' }, cts.token), undefined);
    assert.equal(rpcInvoked, false);
  } finally {
    restore();
  }
});

suite.test('[F5] window.showInputBox runs validateInput callback and prevents invalid submission', async () => {
  const prompts = [];
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'window.showInputBox') {
        prompts.push(params);
        return params.options && params.options.value === 'invalid' ? 'invalid' : 'valid-id';
      }
      return [];
    },
  });

  try {
    let seenValidation = null;
    const res = await vscode.window.showInputBox({
      value: 'valid-id',
      validateInput: (val) => {
        seenValidation = val === 'valid-id' ? null : 'Must not be empty';
        return seenValidation;
      },
    });
    assert.equal(res, 'valid-id');
    assert.equal(seenValidation, null);

    // Invalid value validated locally should reject/block or re-prompt without accepting invalid string blindly
    const badResult = await vscode.window.showInputBox({
      value: 'invalid',
      validateInput: (val) => (val === 'invalid' ? 'Invalid identifier' : null),
    });
    assert.notEqual(badResult, 'invalid');
  } finally {
    restore();
  }
});

suite.test('[F5] window.registerCustomEditorProvider rejects empty viewType and non-object provider', () => {
  const { vscode, restore } = loadExtHost({ mockRpc: () => [] });
  try {
    assert.throws(() => vscode.window.registerCustomEditorProvider('', {}), /non-empty viewType/);
    assert.throws(() => vscode.window.registerCustomEditorProvider('e2e.hex', null), /provider object/);
    assert.throws(() => vscode.window.registerCustomEditorProvider('e2e.hex', []), /provider object/);
  } finally {
    restore();
  }
});

suite.test('[F5] TreeDataProvider normalizes missing id, Windows resourceUri, and non-array getChildren', async () => {
  const { vscode, rpc, restore } = loadExtHost({ mockRpc: () => [] });
  try {
    const d = vscode.window.registerTreeDataProvider('e2e.edgeTree', {
      getTreeItem: (el) => el,
      getChildren: (el) => {
        if (!el) {
          return [{ resourceUri: vscode.Uri.file('C:\\work\\Cargo.toml'), collapsibleState: 99 }];
        }
        return null;
      },
    });

    const roots = await rpc.handleIncomingRequest('TreeData.getChildren', {
      providerId: 'e2e.edgeTree',
      element: null,
    });
    assert.equal(roots.length, 1);
    assert.equal(roots[0].label, 'Cargo.toml');
    assert.equal(roots[0].collapsibleState, 2);
    assert.ok(typeof roots[0].id === 'string' && roots[0].id.length > 0, 'expected fallback ID when id omitted');

    const childRes = await rpc.handleIncomingRequest('treeView.getChildren', {
      providerId: 'e2e.edgeTree',
      element: roots[0].id,
    });
    assert.deepEqual(childRes, []);
    d.dispose();
  } finally {
    restore();
  }
});

suite.test('[F5] OutputChannelProxy ignores operations after dispose()', () => {
  const calls = [];
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      calls.push(method);
      return [];
    },
  });

  try {
    const ch = vscode.window.createOutputChannel('E2E Log');
    ch.appendLine('before');
    ch.dispose();
    const countAfterDispose = calls.length;
    ch.append('after');
    ch.replace('replaced');
    ch.clear();
    ch.show();
    ch.hide();
    ch.dispose();
    assert.equal(calls.length, countAfterDispose);
  } finally {
    restore();
  }
});

// ============================================================================
// F6: workspace.* Hydration, Config & Events (Boundary / Error)
// ============================================================================

suite.test('[F6] workspace.openTextDocument does not misparse C:\\... Windows drive path as URI scheme', async () => {
  let seenPayload = null;
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'workspace.openTextDocument') {
        seenPayload = params;
        return {
          uri: params.uri,
          fileName: 'C:\\Users\\dev\\hello world.rs',
          languageId: 'rust',
          version: 1,
          text: 'fn hi() {}',
          lineCount: 1,
        };
      }
      return [];
    },
  });

  try {
    const doc = await vscode.workspace.openTextDocument('C:\\Users\\dev\\hello world.rs');
    assert.ok(seenPayload && seenPayload.uri.startsWith('file:///C:/'));
    assert.equal(doc.uri.scheme, 'file');
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.getConfiguration handles mixed dotted keys, missing keys, and null RPC fallback', () => {
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      if (method === 'workspace.getConfiguration') {
        return {
          'a.b': 1,
          nested: { 'c.d': 2, e: 3 },
        };
      }
      return [];
    },
  });

  try {
    const cfg = vscode.workspace.getConfiguration('myExt');
    assert.equal(cfg.get('a.b'), 1);
    assert.equal(cfg.get('nested.c.d'), 2);
    assert.equal(cfg.get('nested.e'), 3);
    assert.equal(cfg.has('missing.key'), false);
    assert.equal(cfg.get('missing.key', 'fallback'), 'fallback');
  } finally {
    restore();
  }
});

suite.test('[F6] TextDocumentProxy clamps out-of-range lines, negative positions, and empty documents', async () => {
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      if (method === 'workspace.openTextDocument') {
        return {
          uri: 'untitled:Untitled-1',
          fileName: 'Untitled-1',
          languageId: 'plaintext',
          version: 1,
          isUntitled: true,
          eol: 1,
          text: '',
          lineCount: 1,
        };
      }
      return [];
    },
  });

  try {
    const doc = await vscode.workspace.openTextDocument({ language: 'plaintext', content: '' });
    assert.equal(doc.lineAt(-5).lineNumber, 0);
    assert.equal(doc.lineAt(999).text, '');
    const clampedPos = doc.validatePosition(new vscode.Position(50, 50));
    assert.equal(clampedPos.line, 0);
    assert.equal(clampedPos.character, 0);
    const clampedRange = doc.validateRange(
      new vscode.Range(new vscode.Position(-1, -1), new vscode.Position(5, 5))
    );
    assert.equal(clampedRange.start.line, 0);
    assert.equal(clampedRange.end.character, 0);
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.asRelativePath returns original path when outside all workspace folders', () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async () => JSON.stringify(null),
      op_rpc_call_sync: (method) => {
        if (method === 'workspace.getWorkspaceFolders') {
          return JSON.stringify([{ uri: 'file:///workspace/proj', name: 'proj', index: 0 }]);
        }
        return JSON.stringify([]);
      },
    },
  });

  try {
    assert.equal(vscode.workspace.getWorkspaceFolder(vscode.Uri.file('/outside/file.txt')), undefined);
    const rel = vscode.workspace.asRelativePath('/outside/file.txt');
    assert.equal(rel.replace(/\\/g, '/'), '/outside/file.txt');
  } finally {
    restore();
  }
});

suite.test('[F6] workspace.onDidChangeConfiguration affectsConfiguration handles undefined/empty scope safely', () => {
  const { vscode, rpc, restore } = loadExtHost({ mockRpc: () => [] });
  try {
    let evt = null;
    const d = vscode.workspace.onDidChangeConfiguration((e) => {
      evt = e;
    });
    rpc.dispatchNotification('workspace.onDidChangeConfiguration', { affectedKeys: ['files.autoSave'] });
    assert.ok(evt);
    assert.equal(evt.affectsConfiguration(''), false);
    assert.equal(evt.affectsConfiguration('files', undefined), true);
    d.dispose();
  } finally {
    restore();
  }
});

// ============================================================================
// F7: env.* & extensions.* (Boundary / Error)
// ============================================================================

suite.test('[F7] env.openExternal returns false for empty/null/undefined without invoking RPC', async () => {
  let called = false;
  const { vscode, restore } = loadExtHost({
    mockRpc: (method) => {
      if (method === 'env.openExternal') {
        called = true;
      }
      return [];
    },
  });

  try {
    assert.equal(await vscode.env.openExternal(''), false);
    assert.equal(await vscode.env.openExternal(null), false);
    assert.equal(await vscode.env.openExternal(undefined), false);
    assert.equal(called, false);
  } finally {
    restore();
  }
});

suite.test('[F7] env.clipboard.writeText normalizes null/undefined to empty string', async () => {
  const writes = [];
  const { vscode, restore } = loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'env.clipboardWrite') {
        writes.push(params.value);
        return { written: true };
      }
      return [];
    },
  });

  try {
    await vscode.env.clipboard.writeText(null);
    await vscode.env.clipboard.writeText(undefined);
    assert.deepEqual(writes, ['', '']);
  } finally {
    restore();
  }
});

suite.test('[F7] extensions.getExtension returns undefined for empty, non-string, or unknown IDs', () => {
  const { vscode, restore } = loadExtHost({ mockRpc: () => [] });
  try {
    assert.strictEqual(vscode.extensions.getExtension(''), undefined);
    assert.strictEqual(vscode.extensions.getExtension(null), undefined);
    assert.strictEqual(vscode.extensions.getExtension('does.not.exist'), undefined);
  } finally {
    restore();
  }
});

suite.test('[F7] extensions.activate keeps isActive false when RPC returns activated=false', async () => {
  const { vscode, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async (method) => {
        if (method === 'extensions.activate') {
          return JSON.stringify({ activated: false });
        }
        return JSON.stringify(null);
      },
      op_rpc_call_sync: (method) => {
        if (method === 'extensions.getAll') {
          return JSON.stringify([
            { id: 'broken.ext', extensionPath: '/ext/broken', isActive: false, packageJSON: {} },
          ]);
        }
        return JSON.stringify(null);
      },
    },
  });

  try {
    const ext = vscode.extensions.getExtension('broken.ext');
    assert.ok(ext);
    await ext.activate();
    assert.equal(ext.isActive, false);
  } finally {
    restore();
  }
});

suite.test('[F7] extensions.onDidChange ignores non-array payloads without corrupting extensions.all', () => {
  const { vscode, rpc, restore } = loadExtHost({
    denoOps: {
      op_rpc_call: async () => JSON.stringify(null),
      op_rpc_call_sync: (method) => {
        if (method === 'extensions.getAll') {
          return JSON.stringify([
            { id: 'keep.me', extensionPath: '/ext/keep', isActive: true, packageJSON: {} },
          ]);
        }
        return JSON.stringify(null);
      },
    },
  });

  try {
    assert.equal(vscode.extensions.all.length, 1);
    rpc.dispatchNotification('extensions.onDidChange', { unexpected: true });
    rpc.dispatchNotification('extensions.onDidChange', null);
    assert.equal(vscode.extensions.all.length, 1);
    assert.equal(vscode.extensions.all[0].id, 'keep.me');
  } finally {
    restore();
  }
});

// ============================================================================
// F8: node-compat/fs.js & path.js (Boundary / Error)
// ============================================================================

suite.test('[F8] node-compat/fs.js readFile with null encoding converts byte array to Uint8Array', async () => {
  loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'fs.readFile') {
        assert.strictEqual(params.encoding, null);
        return [0x00, 0xff, 0x41];
      }
      return null;
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    const bytes = await compatFs.promises.readFile('blob.bin', null);
    assert.ok(bytes instanceof Uint8Array);
    assert.deepEqual(Array.from(bytes), [0x00, 0xff, 0x41]);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F8] node-compat/fs.js writeFile encodes string/hex options and handles Uint8Array payloads', async () => {
  const sent = [];
  loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'fs.writeFile') {
        sent.push(params);
        return { written: true };
      }
      return null;
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    await compatFs.promises.writeFile('enc.txt', 'data', { encoding: 'utf8' });
    await compatFs.promises.writeFile('hex.bin', 'deadbeef', { encoding: 'hex' });
    await compatFs.promises.writeFile('bin.dat', Uint8Array.from([1, 2, 3]));
    assert.equal(sent[0].data, 'data');
    assert.deepEqual(sent[1].data, [0xde, 0xad, 0xbe, 0xef]);
    assert.deepEqual(sent[2].data, [1, 2, 3]);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F8] node-compat/fs.js rejects invalid path inputs cleanly on RPC error', async () => {
  loadExtHost({
    mockRpc: (method) => {
      if (method.startsWith('fs.')) {
        return { error: { code: 'EINVAL', message: 'EINVAL: Invalid empty or NUL path' } };
      }
      return [];
    },
  });

  try {
    const compatFs = loadNodeCompat('fs');
    await assert.rejects(() => compatFs.promises.readFile(''), (e) => e.code === 'EINVAL');
    await assert.rejects(() => compatFs.promises.stat('bad\0path'), (e) => e.code === 'EINVAL');
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F8] node-compat/path.js handles empty, non-string, and root clamping boundaries', () => {
  const compatPath = loadNodeCompat('path');
  assert.equal(compatPath.normalize(''), '.');
  assert.equal(compatPath.normalize(null), '.');
  assert.equal(compatPath.posix.dirname('/'), '/');
  assert.equal(compatPath.dirname('file.txt'), '.');
  assert.equal(compatPath.win32.basename('C:\\'), '');
  assert.equal(compatPath.extname('.gitignore'), '');
  assert.equal(compatPath.extname('noext'), '');
});

suite.test('[F8] node-compat/path.js normalizes UNC paths and mixed slashes without dropping share root', () => {
  const compatPath = loadNodeCompat('path');
  const norm = compatPath.win32.normalize('\\\\fileserver\\share\\dir\\..\\file.txt').replace(/\\/g, '/');
  assert.equal(norm, '//fileserver/share/file.txt');
  assert.equal(compatPath.win32.isAbsolute('\\\\fileserver\\share\\file.txt'), true);
});

// ============================================================================
// F15: Expanded node-compat Polyfills (Boundary / Error)
// ============================================================================

suite.test('[F15] node-compat/buffer.js throws RangeError on negative alloc and out-of-bounds read', () => {
  const { Buffer } = loadNodeCompat('buffer');
  assert.throws(() => Buffer.alloc(-1), RangeError);
  const b = Buffer.from([1, 2]);
  assert.throws(() => b.readUInt32BE(0), RangeError);
});

suite.test('[F15] node-compat/url.js throws TypeError on non-file URL in fileURLToPath and malformed URL', () => {
  const compatUrl = loadNodeCompat('url');
  assert.throws(() => new compatUrl.URL('not a valid url'), TypeError);
  assert.throws(() => compatUrl.fileURLToPath('https://example.com/a'), TypeError);
  if (process.platform === 'win32') {
    assert.equal(
      compatUrl.fileURLToPath('file:///C:/tmp/hello%20world.txt').replace(/\\/g, '/'),
      'C:/tmp/hello world.txt'
    );
  } else {
    assert.equal(
      compatUrl.fileURLToPath('file:///tmp/hello%20world.txt').replace(/\\/g, '/'),
      '/tmp/hello world.txt'
    );
  }
});

suite.test('[F15] node-compat/util.js handles circular references without stack overflow and rejects promisified errors', async () => {
  const compatUtil = loadNodeCompat('util');
  const cyclic = { name: 'root' };
  cyclic.self = cyclic;

  const inspected = compatUtil.inspect(cyclic);
  assert.match(inspected, /\[Circular/);
  const formatted = compatUtil.format('%j', cyclic);
  assert.ok(typeof formatted === 'string' && formatted.length > 0);

  const failFn = compatUtil.promisify((cb) => cb(new Error('boom')));
  await assert.rejects(() => failFn(), /boom/);
});

suite.test('[F15] node-compat/crypto.js rejects unsupported hash algorithms, length mismatch in timingSafeEqual, and bad bounds', () => {
  const compatCrypto = loadNodeCompat('crypto');
  assert.throws(() => compatCrypto.createHash('md2-unsupported'), /not supported/i);
  assert.throws(() => compatCrypto.randomBytes(-4), RangeError);
  assert.throws(() => compatCrypto.randomInt(10, 10), RangeError);
  assert.throws(
    () => compatCrypto.timingSafeEqual(Uint8Array.from([1, 2]), Uint8Array.from([1])),
    RangeError
  );
  assert.equal(
    compatCrypto.timingSafeEqual(Uint8Array.from([1, 2]), Uint8Array.from([1, 3])),
    false
  );
});

suite.test('[F15] node-compat/child_process.js propagates non-zero exit code and ENOENT errors', async () => {
  loadExtHost({
    mockRpc: (method, params) => {
      if (method === 'process.exec') {
        return { exitCode: 2, stdout: '', stderr: 'bad flag' };
      }
      if (method === 'process.spawn') {
        return { error: { code: 'ENOENT', message: `ENOENT: ${params.command} not found` } };
      }
      if (method === 'process.spawnSync') {
        return { pid: 0, status: 1, signal: null, stdout: '', stderr: 'failed' };
      }
      return [];
    },
  });

  try {
    const cp = loadNodeCompat('child_process');
    const execErr = await new Promise((resolve) => {
      cp.exec('false', (err) => resolve(err));
    });
    assert.ok(execErr instanceof Error);
    assert.equal(execErr.code, 2);

    const spawnErr = await new Promise((resolve) => {
      const child = cp.spawn('missing-binary-e2e');
      child.on('error', (err) => resolve(err));
    });
    assert.ok(spawnErr instanceof Error);
    assert.equal(spawnErr.code, 'ENOENT');

    const syncRes = cp.spawnSync('false');
    assert.equal(syncRes.status, 1);
  } finally {
    restoreRequireHook();
  }
});

suite.test('[F15] node-compat/os.js returns non-empty homedir and tmpdir even when RPC returns null', () => {
  loadExtHost({ mockRpc: () => null });
  try {
    const compatOs = loadNodeCompat('os');
    assert.ok(typeof compatOs.homedir() === 'string' && compatOs.homedir().length > 0);
    assert.ok(typeof compatOs.tmpdir() === 'string' && compatOs.tmpdir().length > 0);
    assert.ok(['LE', 'BE'].includes(compatOs.endianness()));
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
