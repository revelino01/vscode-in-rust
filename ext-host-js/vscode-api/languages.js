// vscode.languages API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

let nextProviderId = 1;

class DiagnosticCollection {
  constructor(name = '') {
    this.name = name;
    this._entries = new Map();
  }

  set(uri, diagnostics) {
    const key = uri.toString();
    this._entries.set(key, diagnostics);
    rpc.call('languages.setDiagnostics', {
      name: this.name,
      uri: key,
      diagnostics,
    });
  }

  delete(uri) {
    const key = uri.toString();
    this._entries.delete(key);
    rpc.call('languages.clearDiagnostics', { name: this.name, uri: key });
  }

  clear() {
    this._entries.clear();
    rpc.call('languages.clearAllDiagnostics', { name: this.name });
  }

  dispose() {
    this.clear();
  }
}

const languages = {
  createDiagnosticCollection(name) {
    return new DiagnosticCollection(name);
  },

  registerCompletionItemProvider(selector, provider, ...triggerCharacters) {
    const id = `completion_${nextProviderId++}`;
    return rpc.registerObjectProxy('Completion', id, provider, ['provideCompletionItems', 'resolveCompletionItem']);
  },

  registerHoverProvider(selector, provider) {
    const id = `hover_${nextProviderId++}`;
    return rpc.registerObjectProxy('Hover', id, provider, ['provideHover']);
  },

  registerDefinitionProvider(selector, provider) {
    const id = `definition_${nextProviderId++}`;
    return rpc.registerObjectProxy('Definition', id, provider, ['provideDefinition']);
  },

  registerDocumentFormattingEditProvider(selector, provider) {
    const id = `formatting_${nextProviderId++}`;
    return rpc.registerObjectProxy('DocumentFormatting', id, provider, ['provideDocumentFormattingEdits']);
  },
};

module.exports = languages;
