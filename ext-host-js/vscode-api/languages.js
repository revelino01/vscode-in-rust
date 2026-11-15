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
    const mappedDiagnostics = (diagnostics || []).map(d => ({
      ...d,
      // Convert VS Code API 0-indexed severity (0=Error) to LSP 1-indexed (1=Error)
      severity: typeof d.severity === 'number' ? d.severity + 1 : 1,
    }));
    rpc.call('languages.setDiagnostics', {
      name: this.name,
      uri: key,
      diagnostics: mappedDiagnostics,
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
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('Completion', id, provider, ['provideCompletionItems', 'resolveCompletionItem'], {
      selector: sel,
      triggerCharacters,
    });
  },

  registerHoverProvider(selector, provider) {
    const id = `hover_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('Hover', id, provider, ['provideHover'], {
      selector: sel,
    });
  },

  registerDefinitionProvider(selector, provider) {
    const id = `definition_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('Definition', id, provider, ['provideDefinition'], {
      selector: sel,
    });
  },

  registerDocumentFormattingEditProvider(selector, provider) {
    const id = `formatting_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('DocumentFormatting', id, provider, ['provideDocumentFormattingEdits'], {
      selector: sel,
    });
  },
};

module.exports = languages;
