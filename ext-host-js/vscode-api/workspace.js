// vscode.workspace API implementation

const rpc = require('../rpc-bridge');
const { Uri } = require('./types');

class WorkspaceConfiguration {
  constructor(section, data = {}) {
    this._section = section;
    this._data = data;
  }

  get(section, defaultValue) {
    if (this._data && section in this._data) {
      return this._data[section];
    }
    return defaultValue;
  }

  has(section) {
    return this._data && section in this._data;
  }

  async update(section, value, configurationTarget) {
    const fullKey = this._section ? `${this._section}.${section}` : section;
    return await rpc.call('workspace.updateConfiguration', {
      key: fullKey,
      value,
      target: configurationTarget,
    });
  }
}

const workspace = {
  rootPath: null,
  workspaceFolders: [],

  getConfiguration(section, scope) {
    const data = rpc.callSync('workspace.getConfiguration', { section });
    return new WorkspaceConfiguration(section, data || {});
  },

  async openTextDocument(uriOrFileName) {
    const uri = typeof uriOrFileName === 'string' ? Uri.file(uriOrFileName) : uriOrFileName;
    return await rpc.call('workspace.openTextDocument', { uri: uri.toString() });
  },

  // Events
  onDidChangeConfiguration: rpc.createEvent('workspace.onDidChangeConfiguration'),
  onDidOpenTextDocument: rpc.createEvent('workspace.onDidOpenTextDocument'),
  onDidCloseTextDocument: rpc.createEvent('workspace.onDidCloseTextDocument'),
  onDidChangeTextDocument: rpc.createEvent('workspace.onDidChangeTextDocument'),
  onDidSaveTextDocument: rpc.createEvent('workspace.onDidSaveTextDocument'),
};

module.exports = workspace;
