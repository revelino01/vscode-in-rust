// vscode.env and vscode.extensions

const rpc = require('../rpc-bridge');

const env = {
  appName: 'RustCode',
  appRoot: '/rustcode',
  language: 'en',
  clipboard: {
    readText() {
      return rpc.call('env.clipboardRead');
    },
    writeText(value) {
      return rpc.call('env.clipboardWrite', { value });
    }
  },
  openExternal(uri) {
    return rpc.call('env.openExternal', { uri: uri.toString() });
  }
};

const extensions = {
  all: [],
  getExtension(extensionId) {
    return this.all.find(e => e.id === extensionId);
  }
};

module.exports = {
  env,
  extensions,
};
