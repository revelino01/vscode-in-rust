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
    rpc.fireAndForget('languages.setDiagnostics', {
      name: this.name,
      uri: key,
      diagnostics: mappedDiagnostics,
    });
  }

  delete(uri) {
    const key = uri.toString();
    this._entries.delete(key);
    rpc.fireAndForget('languages.clearDiagnostics', { name: this.name, uri: key });
  }

  clear() {
    this._entries.clear();
    rpc.fireAndForget('languages.clearAllDiagnostics', { name: this.name });
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

  registerCodeActionsProvider(selector, provider, metadata = {}) {
    const id = `codeaction_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('CodeAction', id, provider, ['provideCodeActions'], {
      selector: sel,
      providedCodeActionKinds: metadata.providedCodeActionKinds || [],
    });
  },

  registerReferenceProvider(selector, provider) {
    const id = `references_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('References', id, provider, ['provideReferences'], {
      selector: sel,
    });
  },

  registerSignatureHelpProvider(selector, provider, ...triggerCharacters) {
    const id = `sighelp_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('SignatureHelp', id, provider, ['provideSignatureHelp'], {
      selector: sel,
      triggerCharacters,
    });
  },

  registerImplementationProvider(selector, provider) {
    const id = `impl_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('Implementation', id, provider, ['provideImplementation'], {
      selector: sel,
    });
  },

  registerTypeDefinitionProvider(selector, provider) {
    const id = `typedef_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('TypeDefinition', id, provider, ['provideTypeDefinition'], {
      selector: sel,
    });
  },

  registerRenameProvider(selector, provider) {
    const id = `rename_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('Rename', id, provider, ['provideRenameEdits', 'prepareRename'], {
      selector: sel,
    });
  },

  registerDocumentSemanticTokensProvider(selector, provider, legend) {
    const id = `semtok_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('SemanticTokens', id, provider, ['provideDocumentSemanticTokens', 'provideDocumentSemanticTokensEdits'], {
      selector: sel,
      legend: legend || { tokenTypes: [], tokenModifiers: [] },
    });
  },

  registerInlayHintsProvider(selector, provider) {
    const id = `inlay_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('InlayHints', id, provider, ['provideInlayHints', 'resolveInlayHint'], {
      selector: sel,
    });
  },

  registerCodeLensProvider(selector, provider) {
    const id = `codelens_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('CodeLens', id, provider, ['provideCodeLenses', 'resolveCodeLens'], {
      selector: sel,
    });
  },

  registerDocumentLinkProvider(selector, provider) {
    const id = `doclink_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('DocumentLink', id, provider, ['provideDocumentLinks', 'resolveDocumentLink'], {
      selector: sel,
    });
  },

  registerCallHierarchyProvider(selector, provider) {
    const id = `callhier_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('CallHierarchy', id, provider, ['prepareCallHierarchy', 'provideCallHierarchyIncomingCalls', 'provideCallHierarchyOutgoingCalls'], {
      selector: sel,
    });
  },

  registerTypeHierarchyProvider(selector, provider) {
    const id = `typehier_${nextProviderId++}`;
    const sel = typeof selector === 'string' ? selector : (selector && selector.language) || '*';
    return rpc.registerObjectProxy('TypeHierarchy', id, provider, ['prepareTypeHierarchy', 'provideTypeHierarchySupertypes', 'provideTypeHierarchySubtypes'], {
      selector: sel,
    });
  },

  async format(documentOrUri, options = { tabSize: 4, insertSpaces: true }) {
    const uri = (documentOrUri && documentOrUri.uri) ? documentOrUri.uri.toString() : (documentOrUri ? documentOrUri.toString() : '');
    const languageId = (documentOrUri && documentOrUri.languageId) ? documentOrUri.languageId : '*';
    return await rpc.call('languages.format', {
      uri,
      languageId,
      options,
    });
  },

  async codeAction(documentOrUri, range, context = { diagnostics: [] }) {
    const uri = (documentOrUri && documentOrUri.uri) ? documentOrUri.uri.toString() : (documentOrUri ? documentOrUri.toString() : '');
    const languageId = (documentOrUri && documentOrUri.languageId) ? documentOrUri.languageId : '*';
    return await rpc.call('languages.codeAction', {
      uri,
      languageId,
      range,
      context,
    });
  },
};

module.exports = languages;
