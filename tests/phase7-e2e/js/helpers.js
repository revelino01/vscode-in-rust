'use strict';

const fs = require('fs');
const Module = require('module');
const path = require('path');

const originalResolveFilename = Module._resolveFilename;
const hadInitialDeno = Object.prototype.hasOwnProperty.call(globalThis, 'Deno');
const initialDenoDescriptor = hadInitialDeno
  ? Object.getOwnPropertyDescriptor(globalThis, 'Deno')
  : null;
const RPC_BRIDGE_STATE_SYMBOL = Symbol.for('rustcode.rpcBridgeState');
let activeHookState = null;

function restoreInitialDeno() {
  if (hadInitialDeno && initialDenoDescriptor) {
    Object.defineProperty(globalThis, 'Deno', initialDenoDescriptor);
  } else {
    delete globalThis.Deno;
  }
}

function resetRpcBridgeState() {
  const state = globalThis[RPC_BRIDGE_STATE_SYMBOL];
  if (state && typeof state === 'object') {
    const transport = state.stdioTransport;
    if (transport && typeof transport === 'object') {
      try {
        if (typeof transport.stop === 'function') {
          transport.stop();
        } else if (typeof transport.close === 'function') {
          transport.close();
        }
      } catch (_) {
        // Best-effort transport shutdown during test teardown
      }
    }
    if (typeof state.stop === 'function') {
      try {
        state.stop();
      } catch (_) {
        // Best-effort state shutdown during test teardown
      }
    }
  }
  delete globalThis[RPC_BRIDGE_STATE_SYMBOL];
}

function repoRoot() {
  return path.resolve(__dirname, '..', '..', '..');
}

function extHostDir() {
  return path.join(repoRoot(), 'ext-host-js');
}

function normalizeForCompare(p) {
  const resolved = path.resolve(p);
  return process.platform === 'win32' ? resolved.toLowerCase() : resolved;
}

function isPathInside(targetPath, parentDir) {
  if (!targetPath || !parentDir) {
    return false;
  }
  const rel = path.relative(normalizeForCompare(parentDir), normalizeForCompare(targetPath));
  return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel));
}

function clearExtHostCache() {
  resetRpcBridgeState();
  const hostDir = extHostDir();
  for (const key of Object.keys(require.cache)) {
    if (isPathInside(key, hostDir)) {
      delete require.cache[key];
    }
  }
}

function restoreRequireHook() {
  Module._resolveFilename = originalResolveFilename;
  activeHookState = null;
  restoreInitialDeno();
  resetRpcBridgeState();
}

function resolveNodeCompatMapping(request, nodeCompat) {
  if (!nodeCompat) {
    return null;
  }
  const cleanName = request.startsWith('node:') ? request.slice('node:'.length) : request;
  if (!/^[a-zA-Z0-9_-]+$/.test(cleanName)) {
    return null;
  }

  if (nodeCompat === true) {
    const candidate = path.join(extHostDir(), 'node-compat', `${cleanName}.js`);
    return fs.existsSync(candidate) ? candidate : null;
  }

  if (Array.isArray(nodeCompat)) {
    if (!nodeCompat.includes(cleanName) && !nodeCompat.includes(request)) {
      return null;
    }
    const candidate = path.join(extHostDir(), 'node-compat', `${cleanName}.js`);
    return fs.existsSync(candidate) ? candidate : null;
  }

  if (typeof nodeCompat === 'object') {
    const entry = nodeCompat[request] !== undefined ? nodeCompat[request] : nodeCompat[cleanName];
    if (entry === true) {
      const candidate = path.join(extHostDir(), 'node-compat', `${cleanName}.js`);
      return fs.existsSync(candidate) ? candidate : null;
    }
    if (typeof entry === 'string') {
      return path.resolve(entry);
    }
  }

  return null;
}

function installRequireHook({ scopedExtDir = null, nodeCompat = null } = {}) {
  const vscodeEntry = path.join(extHostDir(), 'vscode-api', 'index.js');
  activeHookState = {
    vscodeEntry,
    scopedExtDir: scopedExtDir ? path.resolve(scopedExtDir) : null,
    nodeCompat,
  };

  Module._resolveFilename = function patchedResolveFilename(request, parent, isMain, options) {
    if (request === 'vscode') {
      return activeHookState.vscodeEntry;
    }

    if (activeHookState && activeHookState.nodeCompat) {
      const parentFile = parent && parent.filename ? parent.filename : null;
      const inScopedExt =
        activeHookState.scopedExtDir && parentFile && isPathInside(parentFile, activeHookState.scopedExtDir);
      const inExtHost = parentFile && isPathInside(parentFile, extHostDir());
      if (inScopedExt || inExtHost || !activeHookState.scopedExtDir) {
        const mapped = resolveNodeCompatMapping(request, activeHookState.nodeCompat);
        if (mapped) {
          return mapped;
        }
      }
    }

    return originalResolveFilename.call(this, request, parent, isMain, options);
  };
}

function loadExtHost({ denoOps, mockRpc } = {}) {
  clearExtHostCache();
  restoreInitialDeno();

  if (denoOps === null) {
    delete globalThis.Deno;
  } else if (denoOps !== undefined) {
    globalThis.Deno = { core: { ops: denoOps } };
  } else if (typeof mockRpc === 'function') {
    globalThis.Deno = {
      core: {
        ops: {
          op_rpc_call: async (method, rawParams) => {
            const params = rawParams ? JSON.parse(rawParams) : {};
            const res = await mockRpc(method, params);
            return JSON.stringify(res === undefined ? null : res);
          },
          op_rpc_call_sync: (method, rawParams) => {
            const params = rawParams ? JSON.parse(rawParams) : {};
            const res = mockRpc(method, params);
            if (res && typeof res.then === 'function') {
              throw new Error(
                `mockRpc returned a Promise for synchronous RPC call '${method}'. Remove 'async' from mockRpc or return a synchronous value.`
              );
            }
            return JSON.stringify(res === undefined ? null : res);
          },
        },
      },
    };
  }

  try {
    installRequireHook();

    const rpc = require(path.join(extHostDir(), 'rpc-bridge.js'));

    if (mockRpc && typeof mockRpc === 'object') {
      if (typeof mockRpc.call === 'function') {
        rpc.call = mockRpc.call;
      }
      if (typeof mockRpc.callSync === 'function') {
        rpc.callSync = mockRpc.callSync;
      }
    }

    const vscode = require(path.join(extHostDir(), 'vscode-api', 'index.js'));

    return {
      vscode,
      rpc,
      restore: restoreRequireHook,
    };
  } catch (err) {
    restoreRequireHook();
    throw err;
  }
}

function loadExtension(extDir, { nodeCompat = null, denoOps, mockRpc, fresh = true } = {}) {
  const resolvedDir = path.resolve(extDir);
  const manifestPath = path.join(resolvedDir, 'package.json');
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const mainRel = manifest.main || './extension.js';
  const entryPath = path.resolve(resolvedDir, mainRel);

  try {
    let host = null;
    if (denoOps !== undefined || mockRpc !== undefined || fresh) {
      host = loadExtHost({ denoOps, mockRpc });
    }

    installRequireHook({ scopedExtDir: resolvedDir, nodeCompat });

    if (fresh) {
      for (const key of Object.keys(require.cache)) {
        if (isPathInside(key, resolvedDir)) {
          delete require.cache[key];
        }
      }
    }

    const exports = require(entryPath);
    return {
      manifest,
      exports,
      extensionPath: resolvedDir,
      entryPath,
      vscode: host ? host.vscode : require(path.join(extHostDir(), 'vscode-api', 'index.js')),
      rpc: host ? host.rpc : require(path.join(extHostDir(), 'rpc-bridge.js')),
      restore: restoreRequireHook,
    };
  } catch (err) {
    restoreRequireHook();
    throw err;
  }
}

function parseFilterArg(argv) {
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--filter' && i + 1 < argv.length) {
      return argv[i + 1];
    }
    if (argv[i].startsWith('--filter=')) {
      return argv[i].slice('--filter='.length);
    }
  }
  return process.env.E2E_FILTER || null;
}

function createSuite(suiteName) {
  const tests = [];

  return {
    test(name, fn) {
      tests.push({ name, fn });
    },

    async run() {
      const filter = parseFilterArg(process.argv.slice(2));
      const selected = filter ? tests.filter((t) => t.name.includes(filter)) : tests;

      process.stdout.write(`TAP version 13\n# Suite: ${suiteName}\n1..${selected.length}\n`);

      let passed = 0;
      let failed = 0;
      const results = [];

      for (let i = 0; i < selected.length; i++) {
        const t = selected[i];
        const idx = i + 1;
        const startedAt = Date.now();
        try {
          await t.fn();
          const durationMs = Date.now() - startedAt;
          passed++;
          results.push({ name: t.name, ok: true, durationMs });
          process.stdout.write(`ok ${idx} - ${t.name} (${durationMs}ms)\n`);
        } catch (err) {
          const durationMs = Date.now() - startedAt;
          failed++;
          const msg = err && err.stack ? err.stack : String(err);
          results.push({ name: t.name, ok: false, durationMs, error: msg });
          process.stdout.write(`not ok ${idx} - ${t.name} (${durationMs}ms)\n`);
          for (const line of msg.split(/\r?\n/)) {
            process.stdout.write(`  # ${line}\n`);
          }
        } finally {
          restoreRequireHook();
        }
      }

      const skipped = tests.length - selected.length;
      process.stdout.write(
        `# Summary (${suiteName}): ${passed}/${selected.length} passed, ${failed} failed, ${skipped} filtered\n`
      );

      if (failed > 0) {
        process.exitCode = 1;
      }

      return {
        suite: suiteName,
        total: selected.length,
        passed,
        failed,
        skipped,
        results,
      };
    },
  };
}

module.exports = {
  repoRoot,
  extHostDir,
  clearExtHostCache,
  restoreRequireHook,
  loadExtHost,
  loadExtension,
  createSuite,
};
