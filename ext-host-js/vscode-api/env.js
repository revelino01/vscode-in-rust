// vscode.env and vscode.extensions

const rpc = require('../rpc-bridge');
const { Uri } = require('./types');

const env = {
  appName: 'RustCode',
  appRoot: '/rustcode',
  language: 'en',
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

function normalizeExtension(raw) {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const extensionPath = typeof raw.extensionPath === 'string' ? raw.extensionPath : '';
  const extensionUri = raw.extensionUri
    ? (typeof raw.extensionUri === 'string' ? Uri.parse(raw.extensionUri) : raw.extensionUri)
    : Uri.file(extensionPath);
  const ext = {
    id: raw.id || '',
    extensionUri,
    extensionPath,
    isActive: Boolean(raw.isActive),
    packageJSON: raw.packageJSON || {},
    extensionKind: raw.extensionKind || 1,
    exports: raw.exports,
    async activate() {
      if (!ext.isActive) {
        const res = await rpc.call('extensions.activate', { id: ext.id });
        if (res && res.activated) {
          ext.isActive = true;
        }
      }
      return ext.exports;
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
    return this.all.find(e => typeof e.id === 'string' && e.id.toLowerCase() === target);
  }
};

const initialExtensions = rpc.callSync('extensions.getAll');
if (Array.isArray(initialExtensions)) {
  extensions._setAll(initialExtensions);
}

onDidChangeExtensions((payload) => {
  if (Array.isArray(payload)) {
    extensions._setAll(payload);
  }
});

module.exports = {
  env,
  extensions,
};
