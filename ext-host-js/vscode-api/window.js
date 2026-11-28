// vscode.window API implementation

const rpc = require('../rpc-bridge');
const {
  StatusBarAlignment,
  Disposable,
  EventEmitter,
  Position,
  Range,
  Selection,
  WorkspaceEdit,
  LogLevel,
  DataTransfer,
} = require('./types');
const workspace = require('./workspace');

class StatusBarItemProxy {
  constructor(id, alignment, priority) {
    this.id = id;
    this.alignment = alignment;
    this.priority = priority;
    this._text = '';
    this._tooltip = '';
    this._command = null;
    this._color = null;
    this._visible = false;
  }

  get text() { return this._text; }
  set text(val) {
    this._text = val;
    this._sync();
  }

  get tooltip() { return this._tooltip; }
  set tooltip(val) {
    this._tooltip = val;
    this._sync();
  }

  get command() { return this._command; }
  set command(val) {
    this._command = val;
    this._sync();
  }

  show() {
    this._visible = true;
    this._sync();
  }

  hide() {
    this._visible = false;
    rpc.fireAndForget('window.hideStatusBarItem', { id: this.id });
  }

  dispose() {
    this.hide();
  }

  _sync() {
    if (this._visible) {
      rpc.fireAndForget('window.setStatusBarItem', {
        id: this.id,
        text: this._text,
        tooltip: this._tooltip,
        command: this._command,
        alignment: this.alignment,
        priority: this.priority,
      });
    }
  }
}

class OutputChannelProxy {
  constructor(id, name, languageId = null, log = false) {
    this._id = id;
    this._name = name;
    this._languageId = languageId;
    this._log = log;
    this._logLevel = LogLevel.Info;
    this._onDidChangeLogLevel = new EventEmitter();
    this.onDidChangeLogLevel = this._onDidChangeLogLevel.event;
    this._disposed = false;
  }

  get name() {
    return this._name;
  }

  get logLevel() {
    return this._logLevel;
  }

  setLogLevel(level) {
    if (this._disposed) return;
    this._logLevel = Number(level);
    rpc.fireAndForget('window.setOutputChannelLogLevel', {
      id: this._id,
      name: this._name,
      logLevel: this._logLevel,
    });
    this._onDidChangeLogLevel.fire(this._logLevel);
  }

  _emitLog(level, prefix, message) {
    if (this._disposed) return;
    if (this._log) {
      rpc.fireAndForget('window.logOutputChannelMessage', {
        id: this._id,
        name: this._name,
        level,
        message: String(message ?? ''),
      });
      return;
    }
    this.appendLine(`[${prefix}] ${message ?? ''}`);
  }

  append(value) {
    if (this._disposed) return;
    rpc.fireAndForget('window.appendOutputChannel', {
      id: this._id,
      name: this._name,
      value: String(value ?? ''),
    });
  }

  appendLine(value) {
    if (this._disposed) return;
    rpc.fireAndForget('window.appendOutputChannel', {
      id: this._id,
      name: this._name,
      value: `${value ?? ''}\n`,
    });
  }

  replace(value) {
    if (this._disposed) return;
    rpc.fireAndForget('window.replaceOutputChannel', {
      id: this._id,
      name: this._name,
      value: String(value ?? ''),
    });
  }

  clear() {
    if (this._disposed) return;
    rpc.fireAndForget('window.clearOutputChannel', {
      id: this._id,
      name: this._name,
    });
  }

  show(columnOrPreserveFocus, preserveFocus) {
    if (this._disposed) return;
    let resolvedPreserveFocus = false;
    let viewColumn = null;
    if (typeof columnOrPreserveFocus === 'boolean') {
      resolvedPreserveFocus = columnOrPreserveFocus;
    } else if (typeof columnOrPreserveFocus === 'number') {
      viewColumn = columnOrPreserveFocus;
      resolvedPreserveFocus = Boolean(preserveFocus);
    } else if (preserveFocus !== undefined) {
      resolvedPreserveFocus = Boolean(preserveFocus);
    }
    rpc.fireAndForget('window.showOutputChannel', {
      id: this._id,
      name: this._name,
      preserveFocus: resolvedPreserveFocus,
      viewColumn,
    });
  }

  hide() {
    if (this._disposed) return;
    rpc.fireAndForget('window.hideOutputChannel', {
      id: this._id,
      name: this._name,
    });
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    rpc.fireAndForget('window.disposeOutputChannel', {
      id: this._id,
      name: this._name,
    });
  }

  trace(message) {
    this._emitLog(LogLevel.Trace, 'trace', message);
  }

  debug(message) {
    this._emitLog(LogLevel.Debug, 'debug', message);
  }

  info(message) {
    this._emitLog(LogLevel.Info, 'info', message);
  }

  warn(message) {
    this._emitLog(LogLevel.Warning, 'warn', message);
  }

  error(message) {
    this._emitLog(LogLevel.Error, 'error', message);
  }
}

function toSelection(raw) {
  if (!raw || typeof raw !== 'object') {
    return new Selection(0, 0, 0, 0);
  }
  const a = raw.anchor || raw.start || { line: 0, character: 0 };
  const b = raw.active || raw.end || a;
  return new Selection(
    new Position(a.line || 0, a.character || 0),
    new Position(b.line || 0, b.character || 0)
  );
}

function toRange(raw) {
  if (!raw || typeof raw !== 'object') {
    return new Range(0, 0, 0, 0);
  }
  const s = raw.start || { line: 0, character: 0 };
  const e = raw.end || s;
  return new Range(
    new Position(s.line || 0, s.character || 0),
    new Position(e.line || 0, e.character || 0)
  );
}

class TextEditorProxy {
  constructor(raw) {
    this.id = raw.id || `editor_${Date.now()}`;
    const docRaw = raw.document || {};
    this.document = typeof workspace._upsertDocumentProxy === 'function'
      ? (workspace._upsertDocumentProxy(docRaw) || new workspace._TextDocumentProxy(docRaw))
      : new workspace._TextDocumentProxy(docRaw);
    this.selection = toSelection(raw.selection);
    this.selections = Array.isArray(raw.selections) && raw.selections.length > 0
      ? raw.selections.map(toSelection)
      : [this.selection];
    this.visibleRanges = Array.isArray(raw.visibleRanges)
      ? raw.visibleRanges.map(toRange)
      : [new Range(0, 0, Math.max(0, this.document.lineCount - 1), 0)];
    this.viewColumn = typeof raw.viewColumn === 'number' ? raw.viewColumn : 1;
    this.options = raw.options || {};
  }

  async edit(callback) {
    if (typeof callback !== 'function') {
      return false;
    }
    const pendingEdits = [];
    const builder = {
      replace: (location, value) => {
        const range = location instanceof Range
          ? location
          : new Range(location, location);
        pendingEdits.push({ kind: 'replace', range, text: String(value ?? '') });
      },
      insert: (location, value) => {
        const pos = location instanceof Position ? location : (location && location.start) || new Position(0, 0);
        pendingEdits.push({ kind: 'insert', range: new Range(pos, pos), text: String(value ?? '') });
      },
      delete: (location) => {
        const range = location instanceof Range ? location : new Range(location, location);
        pendingEdits.push({ kind: 'delete', range, text: '' });
      },
      setEndOfLine: () => {},
    };
    callback(builder);
    if (pendingEdits.length === 0) {
      return true;
    }
    const wsEdit = new WorkspaceEdit();
    for (const item of pendingEdits) {
      wsEdit.replace(this.document.uri, item.range, item.text);
    }
    return await workspace.applyEdit(wsEdit);
  }

  async insertSnippet(snippet, location) {
    const text = snippet && typeof snippet.value === 'string' ? snippet.value : String(snippet ?? '');
    const target = location || this.selection;
    return await this.edit((builder) => {
      builder.replace(target, text);
    });
  }

  setDecorations() {}
  revealRange() {}
  show() {}
  hide() {}
}

let nextStatusBarId = 1;
let nextOutputChannelId = 1;
let nextTerminalId = 1;
let nextPromptClientId = 1;

let _activeTextEditor = undefined;
let _activeTextEditorFromSync = false;
let _visibleTextEditors = [];
const _onDidChangeActiveTextEditor = new EventEmitter();
const _onDidChangeVisibleTextEditors = new EventEmitter();
const _webviewViewProviders = new Map();
const _webviewViews = new Map();
const _treeDndControllers = new Map();
const _terminalProxies = new Map();
const _onDidChangeTerminalShellIntegration = new EventEmitter();
const _onDidStartTerminalShellExecution = new EventEmitter();
const _onDidEndTerminalShellExecution = new EventEmitter();

function wrapTextEditor(raw) {
  if (!raw || typeof raw !== 'object' || !raw.document) {
    return undefined;
  }
  return new TextEditorProxy(raw);
}

function normalizeSerialOptions(options) {
  if (!options || typeof options !== 'object') {
    return {};
  }
  const clean = {};
  for (const key of Object.keys(options)) {
    const val = options[key];
    if (typeof val !== 'function' && val !== undefined) {
      clean[key] = val;
    }
  }
  return clean;
}

const window = {
  get activeTextEditor() {
    if (rpc.hasSyncTransport()) {
      const raw = rpc.callSync('window.getActiveTextEditor');
      if (raw === null) {
        if (_activeTextEditorFromSync) {
          _activeTextEditor = undefined;
          _activeTextEditorFromSync = false;
        }
      } else if (raw !== undefined) {
        _activeTextEditor = wrapTextEditor(raw);
        _activeTextEditorFromSync = Boolean(_activeTextEditor);
      }
    }
    return _activeTextEditor;
  },

  get visibleTextEditors() {
    if (rpc.hasSyncTransport()) {
      const raw = rpc.callSync('window.getVisibleTextEditors');
      if (Array.isArray(raw)) {
        _visibleTextEditors = raw.map(wrapTextEditor).filter(Boolean);
      }
    }
    return _visibleTextEditors.slice();
  },

  async showTextDocument(documentOrUri, columnOrOptions, preserveFocus) {
    let uriStr = '';
    if (typeof documentOrUri === 'string') {
      uriStr = workspace._uriFromInput(documentOrUri).toString();
    } else if (documentOrUri && documentOrUri.uri) {
      uriStr = documentOrUri.uri.toString();
    } else if (documentOrUri && typeof documentOrUri.toString === 'function') {
      uriStr = documentOrUri.toString();
    }

    let viewColumn = undefined;
    let resolvedPreserveFocus = Boolean(preserveFocus);
    let options = {};
    if (typeof columnOrOptions === 'number') {
      viewColumn = columnOrOptions;
    } else if (columnOrOptions && typeof columnOrOptions === 'object') {
      options = columnOrOptions;
      if (typeof columnOrOptions.viewColumn === 'number') {
        viewColumn = columnOrOptions.viewColumn;
      }
      if (columnOrOptions.preserveFocus !== undefined) {
        resolvedPreserveFocus = Boolean(columnOrOptions.preserveFocus);
      }
    }

    const res = await rpc.call('window.showTextDocument', {
      uri: uriStr,
      viewColumn,
      preserveFocus: resolvedPreserveFocus,
      options,
    });
    const editor = wrapTextEditor(res);
    if (editor) {
      if (!resolvedPreserveFocus || !_activeTextEditor) {
        _activeTextEditor = editor;
        _activeTextEditorFromSync = false;
      }
      if (!_visibleTextEditors.some((e) => e.document.uri.toString() === editor.document.uri.toString())) {
        _visibleTextEditors.push(editor);
      }
      return editor;
    }
    return undefined;
  },

  // Messages
  async showInformationMessage(message, ...items) {
    const res = await rpc.call('window.showInformationMessage', { message, items });
    return res === null || res === undefined ? undefined : res;
  },

  async showWarningMessage(message, ...items) {
    const res = await rpc.call('window.showWarningMessage', { message, items });
    return res === null || res === undefined ? undefined : res;
  },

  async showErrorMessage(message, ...items) {
    const res = await rpc.call('window.showErrorMessage', { message, items });
    return res === null || res === undefined ? undefined : res;
  },

  // Quick Pick & Input
  async showQuickPick(items, options = {}, token) {
    if (token && token.isCancellationRequested) {
      return undefined;
    }
    const rawItems = Array.isArray(items) ? items : await Promise.resolve(items);
    if (token && token.isCancellationRequested) {
      return undefined;
    }
    const list = Array.isArray(rawItems) ? rawItems : [];
    const allStrings = list.length > 0 && list.every(i => typeof i === 'string');
    const normalized = list.map(i => (typeof i === 'string' ? { label: i } : i));
    const clientId = `prompt_qp_${nextPromptClientId++}`;
    const hasOnDidSelectItem = Boolean(options && typeof options.onDidSelectItem === 'function');

    let selectSub = null;
    if (hasOnDidSelectItem) {
      const onSelectEvent = rpc.createEvent('window.quickPickDidSelectItem');
      selectSub = onSelectEvent((payload) => {
        if (!payload || payload.clientId !== clientId) return;
        const rawItem = payload.item;
        const mapped = allStrings && rawItem && typeof rawItem === 'object' && typeof rawItem.label === 'string'
          ? rawItem.label
          : rawItem;
        options.onDidSelectItem(mapped);
      });
    }

    let tokenSub = null;
    let cancelResolve = null;
    const cancelPromise = token
      ? new Promise((resolve) => {
          cancelResolve = resolve;
          tokenSub = token.onCancellationRequested(() => {
            rpc.fireAndForget('window.cancelPrompt', { clientId });
            resolve(undefined);
          });
        })
      : null;

    try {
      const rpcPromise = rpc.call('window.showQuickPick', {
        items: normalized,
        options: normalizeSerialOptions(options),
        clientId,
        hasOnDidSelectItem,
      });
      if (cancelPromise) {
        rpcPromise.catch(() => {});
      }
      const res = cancelPromise
        ? await Promise.race([rpcPromise, cancelPromise])
        : await rpcPromise;
      if ((token && token.isCancellationRequested) || res === null || res === undefined) {
        return undefined;
      }
      if (allStrings) {
        if (options && options.canPickMany && Array.isArray(res)) {
          return res.map(item => (item && typeof item === 'object' && typeof item.label === 'string' ? item.label : item));
        }
        if (res && typeof res === 'object' && typeof res.label === 'string') {
          return res.label;
        }
      }
      return res;
    } finally {
      if (cancelResolve) cancelResolve = null;
      if (tokenSub && typeof tokenSub.dispose === 'function') tokenSub.dispose();
      if (selectSub && typeof selectSub.dispose === 'function') selectSub.dispose();
    }
  },

  async showInputBox(options = {}, token) {
    if (token && token.isCancellationRequested) {
      return undefined;
    }
    const clientId = `prompt_ib_${nextPromptClientId++}`;
    const hasValidateInput = Boolean(options && typeof options.validateInput === 'function');
    if (hasValidateInput && typeof rpc.registerInputValidator === 'function') {
      rpc.registerInputValidator(clientId, options.validateInput);
    }

    let tokenSub = null;
    const cancelPromise = token
      ? new Promise((resolve) => {
          tokenSub = token.onCancellationRequested(() => {
            rpc.fireAndForget('window.cancelPrompt', { clientId });
            resolve(undefined);
          });
        })
      : null;

    try {
      const rpcPromise = rpc.call('window.showInputBox', {
        options: normalizeSerialOptions(options),
        clientId,
        hasValidateInput,
      });
      if (cancelPromise) {
        rpcPromise.catch(() => {});
      }
      const res = cancelPromise
        ? await Promise.race([rpcPromise, cancelPromise])
        : await rpcPromise;
      if ((token && token.isCancellationRequested) || res === null || res === undefined) {
        return undefined;
      }
      if (hasValidateInput && typeof options.validateInput === 'function') {
        const validationResult = await options.validateInput(res);
        const invalidMsg = typeof validationResult === 'string'
          ? validationResult
          : (validationResult && typeof validationResult === 'object' && typeof validationResult.message === 'string'
              ? validationResult.message
              : '');
        if (invalidMsg) {
          return undefined;
        }
      }
      return res;
    } finally {
      if (hasValidateInput && typeof rpc.unregisterInputValidator === 'function') {
        rpc.unregisterInputValidator(clientId);
      }
      if (tokenSub && typeof tokenSub.dispose === 'function') tokenSub.dispose();
    }
  },

  // Status Bar
  createStatusBarItem(alignment = StatusBarAlignment.Left, priority = 0) {
    const id = `sb_item_${nextStatusBarId++}`;
    return new StatusBarItemProxy(id, alignment, priority);
  },

  // Tree Data Provider
  registerTreeDataProvider(viewId, provider) {
    const proxyDisp = rpc.registerObjectProxy('TreeDataProvider', viewId, provider, [
      'getTreeItem',
      'getChildren',
      'getParent',
      'resolveTreeItem',
    ]);
    let changeSub = null;
    if (provider && typeof provider.onDidChangeTreeData === 'function') {
      changeSub = provider.onDidChangeTreeData((elementOrElements) => {
        const elementIds = [];
        if (Array.isArray(elementOrElements)) {
          for (const el of elementOrElements) {
            const handle = typeof rpc.resolveTreeElementHandle === 'function'
              ? rpc.resolveTreeElementHandle(viewId, el)
              : null;
            if (handle) elementIds.push(handle);
          }
        } else if (elementOrElements !== undefined && elementOrElements !== null) {
          const handle = typeof rpc.resolveTreeElementHandle === 'function'
            ? rpc.resolveTreeElementHandle(viewId, elementOrElements)
            : null;
          if (handle) elementIds.push(handle);
        }
        rpc.fireAndForget('treeView.onDidChangeTreeData', {
          providerId: viewId,
          elementIds,
        });
      });
    }
    return new Disposable(() => {
      if (changeSub && typeof changeSub.dispose === 'function') {
        changeSub.dispose();
      }
      proxyDisp.dispose();
    });
  },

  createTreeView(viewId, options = {}) {
    const providerDisposable = options && options.treeDataProvider
      ? this.registerTreeDataProvider(viewId, options.treeDataProvider)
      : new Disposable(() => {});
    const checkboxEmitter = new EventEmitter();
    if (options && options.dragAndDropController) {
      _treeDndControllers.set(viewId, options.dragAndDropController);
    }
    const dnd = options && options.dragAndDropController;
    rpc.fireAndForget('treeView.setOptions', {
      viewId,
      showCollapseAll: Boolean(options && options.showCollapseAll),
      canSelectMany: Boolean(options && options.canSelectMany),
      dragMimeTypes: dnd && Array.isArray(dnd.dragMimeTypes) ? dnd.dragMimeTypes : [],
      dropMimeTypes: dnd && Array.isArray(dnd.dropMimeTypes) ? dnd.dropMimeTypes : [],
      manageCheckboxStateManually: Boolean(options && options.manageCheckboxStateManually),
    });
    return {
      viewId,
      visible: true,
      selection: [],
      onDidExpandElement: rpc.createEvent(`treeView.onDidExpandElement.${viewId}`),
      onDidCollapseElement: rpc.createEvent(`treeView.onDidCollapseElement.${viewId}`),
      onDidChangeSelection: rpc.createEvent(`treeView.onDidChangeSelection.${viewId}`),
      onDidChangeVisibility: rpc.createEvent(`treeView.onDidChangeVisibility.${viewId}`),
      onDidChangeCheckboxState: checkboxEmitter.event,
      async reveal() {},
      dispose() {
        _treeDndControllers.delete(viewId);
        providerDisposable.dispose();
      },
    };
  },

  // Output Channel
  createOutputChannel(name, languageIdOrOptions) {
    const channelName = name ? String(name) : 'Output';
    let languageId = null;
    let log = false;
    if (typeof languageIdOrOptions === 'string') {
      languageId = languageIdOrOptions;
    } else if (languageIdOrOptions && typeof languageIdOrOptions === 'object') {
      languageId = languageIdOrOptions.languageId || null;
      log = Boolean(languageIdOrOptions.log);
    }
    const id = `output_${nextOutputChannelId++}`;
    rpc.fireAndForget('window.createOutputChannel', {
      id,
      name: channelName,
      languageId,
      log,
    });
    return new OutputChannelProxy(id, channelName, languageId, log);
  },

  // Terminal
  createTerminal(nameOrOptions) {
    const options = typeof nameOrOptions === 'string' ? { name: nameOrOptions } : (nameOrOptions || {});
    const id = `term_${Date.now()}_${nextTerminalId++}`;
    rpc.fireAndForget('window.createTerminal', { id, ...options });

    const shellIntegration = {
      cwd: options.cwd ? (typeof options.cwd === 'string' ? workspace._uriFromInput(options.cwd) : options.cwd) : undefined,
      async executeCommand(commandLine, args) {
        const cmdText = Array.isArray(args) && args.length > 0
          ? `${commandLine} ${args.join(' ')}`
          : String(commandLine || '');
        const execution = {
          commandLine: { value: cmdText, isTrusted: true, confidence: 2 },
          cwd: shellIntegration.cwd,
          read() {
            async function* gen() {
              const res = await rpc.call('window.executeTerminalCommand', {
                id,
                commandLine: cmdText,
              });
              if (res && typeof res.output === 'string' && res.output) {
                yield res.output;
              }
            }
            return gen();
          },
        };
        const res = await rpc.call('window.executeTerminalCommand', { id, commandLine: cmdText });
        _onDidStartTerminalShellExecution.fire({ terminal: proxy, execution });
        _onDidEndTerminalShellExecution.fire({ terminal: proxy, execution, exitCode: res ? res.exitCode : 0 });
        return execution;
      },
    };

    const proxy = {
      id,
      name: options.name || 'Terminal',
      shellIntegration,
      show(preserveFocus = false) {
        rpc.fireAndForget('window.showTerminal', { id, preserveFocus: Boolean(preserveFocus) });
      },
      sendText(text, addNewLine = true) {
        rpc.fireAndForget('window.sendTerminalText', { id, text, addNewLine });
      },
      dispose() {
        _terminalProxies.delete(id);
        rpc.fireAndForget('window.disposeTerminal', { id });
      },
    };
    _terminalProxies.set(id, proxy);
    return proxy;
  },

  // Webview
  createWebviewPanel(viewType, title, showOptions = {}, options = {}) {
    const id = `webview_${Date.now()}_${Math.random().toString(36).substring(2, 7)}`;
    rpc.fireAndForget('window.createWebviewPanel', { id, viewType, title, showOptions, options });
    return rpc.createWebviewPanelProxy(id, viewType, title, options);
  },

  // WebviewView (Sidebar / Panel contributed views)
  registerWebviewViewProvider(viewId, provider, options = {}) {
    if (typeof viewId !== 'string' || !viewId.trim()) {
      throw new Error('registerWebviewViewProvider requires a non-empty viewId');
    }
    if (!provider || typeof provider.resolveWebviewView !== 'function') {
      throw new Error('registerWebviewViewProvider requires provider.resolveWebviewView()');
    }
    _webviewViewProviders.set(viewId, { provider, options });
    rpc.fireAndForget('window.registerWebviewViewProvider', { viewId, options });
    return new Disposable(() => {
      _webviewViewProviders.delete(viewId);
      _webviewViews.delete(viewId);
      rpc.fireAndForget('window.unregisterWebviewViewProvider', { viewId });
    });
  },

  // Custom Editors
  registerCustomEditorProvider(viewType, provider, options = {}) {
    if (typeof viewType !== 'string' || !viewType.trim()) {
      throw new Error('registerCustomEditorProvider requires a non-empty viewType string');
    }
    if (!provider || typeof provider !== 'object' || Array.isArray(provider)) {
      throw new Error('registerCustomEditorProvider requires a provider object');
    }
    const normalizedOptions = {
      webviewOptions: (options && options.webviewOptions) || {},
      supportsMultipleEditorsPerDocument: Boolean(options && options.supportsMultipleEditorsPerDocument),
    };
    const proxyDisposable = rpc.registerObjectProxy(
      'CustomEditor',
      viewType,
      provider,
      [
        'resolveCustomTextEditor',
        'resolveCustomEditor',
        'openCustomDocument',
        'saveCustomDocument',
        'saveCustomDocumentAs',
        'revertCustomDocument',
        'backupCustomDocument',
      ],
      {
        selector: viewType,
        ...normalizedOptions,
      }
    );
    rpc.fireAndForget('window.registerCustomEditorProvider', {
      viewType,
      options: normalizedOptions,
    });
    return new Disposable(() => {
      proxyDisposable.dispose();
      rpc.fireAndForget('window.unregisterCustomEditorProvider', { viewType });
    });
  },

  // Events
  onDidChangeActiveTextEditor: _onDidChangeActiveTextEditor.event,
  onDidChangeVisibleTextEditors: _onDidChangeVisibleTextEditors.event,
  onDidChangeTextEditorSelection: rpc.createEvent('window.onDidChangeTextEditorSelection'),
  onDidChangeTerminalShellIntegration: _onDidChangeTerminalShellIntegration.event,
  onDidStartTerminalShellExecution: _onDidStartTerminalShellExecution.event,
  onDidEndTerminalShellExecution: _onDidEndTerminalShellExecution.event,

  async _resolveWebviewView(params) {
    const viewId = params && params.viewId;
    const entry = viewId ? _webviewViewProviders.get(viewId) : null;
    if (!entry || !entry.provider) {
      throw new Error(`WebviewViewProvider not registered: ${viewId}`);
    }
    const msgEmitter = new EventEmitter();
    const disposeEmitter = new EventEmitter();
    const visEmitter = new EventEmitter();
    let _html = '';
    let _title = (params && params.title) || undefined;
    let _description = undefined;
    let _badge = undefined;
    let _options = { enableScripts: true };
    const view = {
      viewType: viewId,
      visible: true,
      get title() { return _title; },
      set title(v) {
        _title = v;
        rpc.fireAndForget('window.setWebviewViewMeta', { viewId, title: v });
      },
      get description() { return _description; },
      set description(v) {
        _description = v;
        rpc.fireAndForget('window.setWebviewViewMeta', { viewId, description: v });
      },
      get badge() { return _badge; },
      set badge(v) {
        _badge = v;
        rpc.fireAndForget('window.setWebviewViewMeta', { viewId, badge: v });
      },
      onDidDispose: disposeEmitter.event,
      onDidChangeVisibility: visEmitter.event,
      show() {
        rpc.fireAndForget('window.showWebviewView', { viewId });
      },
      webview: {
        get options() { return _options; },
        set options(v) {
          _options = v || {};
          rpc.fireAndForget('window.setWebviewViewMeta', { viewId, options: _options });
        },
        get html() { return _html; },
        set html(v) {
          _html = String(v ?? '');
          rpc.fireAndForget('window.setWebviewViewHtml', { viewId, html: _html });
        },
        onDidReceiveMessage: msgEmitter.event,
        async postMessage(message) {
          await rpc.call('window.postMessageToWebviewView', { viewId, message });
          return true;
        },
        asWebviewUri(uri) { return uri; },
        cspSource: 'https://*.vscode-cdn.net',
      },
      _msgEmitter: msgEmitter,
    };
    _webviewViews.set(viewId, view);
    await entry.provider.resolveWebviewView(
      view,
      { state: undefined },
      params.token || { isCancellationRequested: false }
    );
    return { resolved: true, viewId, html: _html };
  },

  _dispatchWebviewViewMessage(payload) {
    const viewId = payload && payload.viewId;
    const view = viewId ? _webviewViews.get(viewId) : null;
    if (view && view._msgEmitter) {
      view._msgEmitter.fire(payload.message);
    }
  },

  async _handleTreeDrag(params) {
    const viewId = params && params.viewId;
    const ctrl = viewId ? _treeDndControllers.get(viewId) : null;
    if (!ctrl || typeof ctrl.handleDrag !== 'function') {
      return {};
    }
    const dt = new DataTransfer();
    await ctrl.handleDrag(
      Array.isArray(params.sourceElementIds) ? params.sourceElementIds : [],
      dt,
      params.token || { isCancellationRequested: false }
    );
    return dt.toJSON();
  },

  async _handleTreeDrop(params) {
    const viewId = params && params.viewId;
    const ctrl = viewId ? _treeDndControllers.get(viewId) : null;
    if (!ctrl || typeof ctrl.handleDrop !== 'function') {
      return { dropped: false };
    }
    const dt = new DataTransfer((params && params.dataTransfer) || {});
    await ctrl.handleDrop(
      params ? params.targetElementId : undefined,
      dt,
      params.token || { isCancellationRequested: false }
    );
    return { dropped: true };
  },
};

rpc.createEvent('window.onDidChangeActiveTextEditor')((payload) => {
  const editor = wrapTextEditor(payload);
  _activeTextEditor = editor;
  _activeTextEditorFromSync = Boolean(editor);
  _onDidChangeActiveTextEditor.fire(editor);
});

rpc.createEvent('window.onDidChangeVisibleTextEditors')((payload) => {
  const list = Array.isArray(payload) ? payload.map(wrapTextEditor).filter(Boolean) : [];
  _visibleTextEditors = list;
  _onDidChangeVisibleTextEditors.fire(list);
});

rpc.createEvent('window.onDidReceiveWebviewViewMessage')((payload) => {
  window._dispatchWebviewViewMessage(payload);
});

if (typeof rpc.registerInitHook === 'function') {
  rpc.registerInitHook(async () => {
    try {
      const active = await rpc.call('window.getActiveTextEditor');
      if (active && typeof active === 'object' && !_activeTextEditor) {
        _activeTextEditor = wrapTextEditor(active);
      }
    } catch (_) {}
    try {
      const visible = await rpc.call('window.getVisibleTextEditors');
      if (Array.isArray(visible) && _visibleTextEditors.length === 0) {
        _visibleTextEditors = visible.map(wrapTextEditor).filter(Boolean);
      }
    } catch (_) {}
  });
}

module.exports = window;
