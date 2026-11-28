// vscode.env and vscode.extensions

const path = require('path');
const rpc = require('../rpc-bridge');
const { Uri, UIKind, EventEmitter } = require('./types');

const BRIDGE_STATE_SYMBOL = Symbol.for('rustcode.rpcBridgeState');
const rootGlobal = typeof globalThis !== 'undefined'
  ? globalThis
  : (typeof global !== 'undefined' ? global : {});
const sharedState = rootGlobal[BRIDGE_STATE_SYMBOL] || {};
if (!sharedState.activatedExtensions) {
  sharedState.activatedExtensions = new Map();
}
const globalDeno = typeof Deno !== 'undefined' ? Deno : null;
const hasDenoOps = Boolean(globalDeno && globalDeno.core && globalDeno.core.ops && globalDeno.core.ops.op_rpc_call);

function defaultShell() {
  if (typeof process === 'undefined') {
    return '/bin/sh';
  }
  if (process.platform === 'win32') {
    return (process.env && process.env.COMSPEC) || 'cmd.exe';
  }
  return (process.env && process.env.SHELL) || '/bin/bash';
}

function defaultAppRoot() {
  if (typeof process !== 'undefined' && typeof process.cwd === 'function') {
    return process.cwd();
  }
  return '.';
}

const pidSuffix = typeof process !== 'undefined' && process.pid ? process.pid : 1;

const env = {
  appName: 'RustCode',
  appRoot: defaultAppRoot(),
  appHost: 'desktop',
  language: 'en',
  machineId: `rustcode-machine-${pidSuffix}`,
  sessionId: `rustcode-session-${pidSuffix}`,
  shell: defaultShell(),
  uriScheme: 'rustcode',
  uiKind: UIKind.Desktop,
  _hydrated: false,

  _hydrate(info) {
    if (!info || typeof info !== 'object') {
      return env;
    }
    if (typeof info.appName === 'string' && info.appName) env.appName = info.appName;
    if (typeof info.appRoot === 'string' && info.appRoot) env.appRoot = info.appRoot;
    if (typeof info.appHost === 'string' && info.appHost) env.appHost = info.appHost;
    if (typeof info.language === 'string' && info.language) env.language = info.language;
    if (typeof info.machineId === 'string' && info.machineId) env.machineId = info.machineId;
    if (typeof info.sessionId === 'string' && info.sessionId) env.sessionId = info.sessionId;
    if (typeof info.shell === 'string' && info.shell) env.shell = info.shell;
    if (typeof info.uriScheme === 'string' && info.uriScheme) env.uriScheme = info.uriScheme;
    if (typeof info.uiKind === 'number') env.uiKind = info.uiKind;
    env._hydrated = true;
    return env;
  },

  async refresh() {
    const info = await rpc.call('env.getEnvironment');
    if (info && typeof info === 'object') {
      env._hydrate(info);
    }
    return env;
  },

  clipboard: {
    readText() {
      return rpc.call('env.clipboardRead');
    },
    writeText(value) {
      const text = value === undefined || value === null ? '' : String(value);
      return rpc.call('env.clipboardWrite', { value: text });
    }
  },
  openExternal(uri) {
    if (!uri) {
      return Promise.resolve(false);
    }
    const uriStr = typeof uri === 'string'
      ? uri
      : (typeof uri.toString === 'function' ? uri.toString() : String(uri));
    if (!uriStr) {
      return Promise.resolve(false);
    }
    return rpc.call('env.openExternal', { uri: uriStr });
  }
};

function createMemento() {
  const store = new Map();
  return {
    keys() {
      return Array.from(store.keys());
    },
    get(key, defaultValue) {
      return store.has(key) ? store.get(key) : defaultValue;
    },
    async update(key, value) {
      if (value === undefined) {
        store.delete(key);
      } else {
        store.set(key, value);
      }
    },
    setKeysForSync() {},
  };
}

function createEnvironmentVariableCollection() {
  const map = new Map();
  return {
    persistent: true,
    description: undefined,
    replace(variable, value) { map.set(variable, { type: 1, value }); },
    append(variable, value) { map.set(variable, { type: 2, value }); },
    prepend(variable, value) { map.set(variable, { type: 3, value }); },
    get(variable) { return map.get(variable); },
    forEach(cb, thisArg) { map.forEach((v, k) => cb.call(thisArg, k, v, this)); },
    delete(variable) { map.delete(variable); },
    clear() { map.clear(); },
  };
}

function createSecretStorage(extensionId) {
  const extId = String(extensionId || 'default-extension');
  const localCache = new Map();
  const emitter = new EventEmitter();
  const hasTransport = () => rpc.isLiveStdioMode() || rpc.hasSyncTransport() || hasDenoOps;

  return {
    onDidChange: emitter.event,
    async get(key) {
      if (hasTransport()) {
        try {
          const res = await rpc.call('secrets.get', { extensionId: extId, key: String(key) });
          return res && res.value !== undefined && res.value !== null ? res.value : undefined;
        } catch (_) {}
      }
      return localCache.has(key) ? localCache.get(key) : undefined;
    },
    async store(key, value) {
      const k = String(key);
      const v = String(value);
      localCache.set(k, v);
      if (hasTransport()) {
        await rpc.call('secrets.store', { extensionId: extId, key: k, value: v });
      }
      emitter.fire({ key: k });
    },
    async delete(key) {
      const k = String(key);
      localCache.delete(k);
      if (hasTransport()) {
        await rpc.call('secrets.delete', { extensionId: extId, key: k });
      }
      emitter.fire({ key: k });
    },
  };
}

function createExtensionContext(ext) {
  const extPath = ext.extensionPath || '';
  const extUri = ext.extensionUri || Uri.file(extPath);
  const secrets = createSecretStorage(ext.id || (ext.packageJSON && ext.packageJSON.name) || 'ext');
  return {
    subscriptions: [],
    extensionPath: extPath,
    extensionUri: extUri,
    extension: ext,
    extensionMode: 1,
    asAbsolutePath(relativePath) {
      return path.resolve(extPath, relativePath || '');
    },
    globalState: createMemento(),
    workspaceState: createMemento(),
    environmentVariableCollection: createEnvironmentVariableCollection(),
    storagePath: extPath ? path.join(extPath, '.storage') : undefined,
    storageUri: extPath ? Uri.file(path.join(extPath, '.storage')) : undefined,
    globalStoragePath: extPath ? path.join(extPath, '.globalStorage') : undefined,
    globalStorageUri: extPath ? Uri.file(path.join(extPath, '.globalStorage')) : undefined,
    logPath: extPath ? path.join(extPath, '.logs') : undefined,
    logUri: extPath ? Uri.file(path.join(extPath, '.logs')) : undefined,
    secrets,
  };
}

function isPathInsideBase(basePath, targetPath) {
  const normBase = path.resolve(basePath);
  const normTarget = path.resolve(targetPath);
  const cmpBase = typeof process !== 'undefined' && process.platform === 'win32'
    ? normBase.toLowerCase()
    : normBase;
  const cmpTarget = typeof process !== 'undefined' && process.platform === 'win32'
    ? normTarget.toLowerCase()
    : normTarget;
  if (cmpTarget === cmpBase) return true;
  const sep = path.sep;
  const prefix = cmpBase.endsWith(sep) ? cmpBase : cmpBase + sep;
  return cmpTarget.startsWith(prefix);
}

async function _activateLocalModule(ext) {
  const key = (ext.id || '').toLowerCase();
  const existing = key ? sharedState.activatedExtensions.get(key) : null;
  if (existing && existing.isActive) {
    ext.isActive = true;
    ext.exports = existing.exports;
    return ext.exports;
  }
  if (existing && existing.activatingPromise) {
    return await existing.activatingPromise;
  }

  const runActivation = async () => {
    const mainRel = ext.packageJSON && typeof ext.packageJSON.main === 'string'
      ? ext.packageJSON.main.trim()
      : '';
    let exportsVal = ext.exports;
    let mod = null;
    let context = null;

    if (mainRel && ext.extensionPath) {
      if (typeof rpc.installVscodeModuleHook === 'function') {
        rpc.installVscodeModuleHook();
      }
      const entryPath = path.resolve(ext.extensionPath, mainRel);
      if (!isPathInsideBase(ext.extensionPath, entryPath)) {
        throw new Error(`Extension '${ext.id}' main '${mainRel}' escapes extensionPath`);
      }
      context = createExtensionContext(ext);
      mod = require(entryPath);
      if (mod && typeof mod.activate === 'function') {
        const t0 = Date.now();
        try {
          exportsVal = await mod.activate(context);
          if (rpc.isLiveStdioMode() || rpc.hasSyncTransport() || hasDenoOps) {
            rpc.fireAndForget('extHost.recordActivation', {
              extensionId: ext.id,
              activationEvent: '*',
              durationMs: Math.max(0, Date.now() - t0),
              succeeded: true,
            });
          }
        } catch (actErr) {
          if (rpc.isLiveStdioMode() || rpc.hasSyncTransport() || hasDenoOps) {
            rpc.fireAndForget('extHost.recordActivation', {
              extensionId: ext.id,
              activationEvent: '*',
              durationMs: Math.max(0, Date.now() - t0),
              succeeded: false,
              error: String((actErr && actErr.message) || actErr),
            });
          }
          throw actErr;
        }
      }
    }

    ext.isActive = true;
    ext.exports = exportsVal;
    if (key) {
      sharedState.activatedExtensions.set(key, {
        isActive: true,
        exports: exportsVal,
        context,
        module: mod,
      });
    }
    return exportsVal;
  };

  const activatingPromise = runActivation();
  if (key) {
    sharedState.activatedExtensions.set(key, {
      isActive: false,
      exports: undefined,
      activatingPromise,
    });
  }
  try {
    return await activatingPromise;
  } catch (err) {
    if (key) {
      sharedState.activatedExtensions.delete(key);
    }
    throw err;
  }
}

function normalizeExtension(raw) {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const extensionPath = typeof raw.extensionPath === 'string' ? raw.extensionPath : '';
  const extensionUri = raw.extensionUri
    ? (typeof raw.extensionUri === 'string' ? Uri.parse(raw.extensionUri) : raw.extensionUri)
    : Uri.file(extensionPath);
  const id = raw.id || '';
  const cached = id ? sharedState.activatedExtensions.get(id.toLowerCase()) : null;
  const ext = {
    id,
    extensionUri,
    extensionPath,
    isActive: Boolean(raw.isActive || (cached && cached.isActive)),
    packageJSON: raw.packageJSON || {},
    extensionKind: raw.extensionKind || 1,
    exports: cached && cached.isActive ? cached.exports : raw.exports,
    async activate() {
      const entry = ext.id ? sharedState.activatedExtensions.get(ext.id.toLowerCase()) : null;
      if (entry && entry.isActive) {
        ext.isActive = true;
        ext.exports = entry.exports;
        return ext.exports;
      }
      if (!ext.isActive && (rpc.isLiveStdioMode() || rpc.hasSyncTransport() || hasDenoOps)) {
        const res = await rpc.call('extensions.activate', { id: ext.id });
        if (res && res.activated === false) {
          ext.isActive = false;
          return undefined;
        }
        if (res && res.activated) {
          ext.isActive = true;
          if (res.exports !== undefined) {
            ext.exports = res.exports;
          }
        }
      }
      return await _activateLocalModule(ext);
    }
  };
  return ext;
}

const onDidChangeExtensions = rpc.createEvent('extensions.onDidChange');

const extensions = {
  all: [],
  onDidChange: onDidChangeExtensions,
  _setAll(list) {
    if (!Array.isArray(list)) {
      this.all = [];
      return this.all;
    }
    this.all = list.map(normalizeExtension).filter(Boolean);
    return this.all;
  },
  async refresh() {
    const list = await rpc.call('extensions.getAll');
    if (Array.isArray(list)) {
      this._setAll(list);
    }
    return this.all;
  },
  getExtension(extensionId) {
    if (typeof extensionId !== 'string' || !extensionId) {
      return undefined;
    }
    const target = extensionId.toLowerCase();
    return (
      this.all.find(e => typeof e.id === 'string' && e.id.toLowerCase() === target) ||
      this.all.find(
        e => e.packageJSON && typeof e.packageJSON.name === 'string' && e.packageJSON.name.toLowerCase() === target
      )
    );
  }
};

async function _activateExtensionFromHost(params) {
  if (!env._hydrated && (rpc.isLiveStdioMode() || hasDenoOps)) {
    try {
      await env.refresh();
    } catch (_) {}
  }
  if (extensions.all.length === 0 && (rpc.isLiveStdioMode() || hasDenoOps)) {
    try {
      await extensions.refresh();
    } catch (_) {}
  }
  const targetId = params && (params.id || params.extensionId);
  let ext = targetId ? extensions.getExtension(targetId) : undefined;
  if (!ext && params && typeof params === 'object') {
    ext = normalizeExtension({
      id: targetId,
      extensionPath: params.extensionPath || '',
      packageJSON: params.packageJSON || {},
      isActive: false,
    });
    if (ext) {
      extensions.all.push(ext);
    }
  }
  if (!ext) {
    const err = new Error(`Extension not found: ${targetId}`);
    err.code = 'NOT_FOUND';
    throw err;
  }
  await _activateLocalModule(ext);
  return { activated: true, id: ext.id };
}

if (rpc.hasSyncTransport()) {
  const initialEnv = rpc.callSync('env.getEnvironment');
  if (initialEnv && typeof initialEnv === 'object') {
    env._hydrate(initialEnv);
  }
  const initialExtensions = rpc.callSync('extensions.getAll');
  if (Array.isArray(initialExtensions)) {
    extensions._setAll(initialExtensions);
  }
}

rpc.createEvent('env.onDidChangeEnvironment')((payload) => {
  env._hydrate(payload);
});

onDidChangeExtensions((payload) => {
  if (Array.isArray(payload)) {
    extensions._setAll(payload);
  } else if (payload && Array.isArray(payload.extensions)) {
    extensions._setAll(payload.extensions);
  }
});

if (typeof rpc.registerInitHook === 'function') {
  rpc.registerInitHook(async () => {
    try {
      await env.refresh();
    } catch (_) {}
    try {
      await extensions.refresh();
    } catch (_) {}
  });
}

module.exports = {
  env,
  extensions,
  _activateExtensionFromHost,
};
