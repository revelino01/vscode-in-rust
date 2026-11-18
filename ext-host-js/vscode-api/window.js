// vscode.window API implementation

const rpc = require('../rpc-bridge');
const { StatusBarAlignment, Disposable } = require('./types');

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
    rpc.call('window.hideStatusBarItem', { id: this.id });
  }

  dispose() {
    this.hide();
  }

  _sync() {
    if (this._visible) {
      rpc.call('window.setStatusBarItem', {
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
    this._disposed = false;
  }

  get name() {
    return this._name;
  }

  append(value) {
    if (this._disposed) return;
    rpc.call('window.appendOutputChannel', {
      id: this._id,
      name: this._name,
      value: String(value ?? ''),
    });
  }

  appendLine(value) {
    if (this._disposed) return;
    rpc.call('window.appendOutputChannel', {
      id: this._id,
      name: this._name,
      value: `${value ?? ''}\n`,
    });
  }

  replace(value) {
    if (this._disposed) return;
    rpc.call('window.replaceOutputChannel', {
      id: this._id,
      name: this._name,
      value: String(value ?? ''),
    });
  }

  clear() {
    if (this._disposed) return;
    rpc.call('window.clearOutputChannel', {
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
    rpc.call('window.showOutputChannel', {
      id: this._id,
      name: this._name,
      preserveFocus: resolvedPreserveFocus,
      viewColumn,
    });
  }

  hide() {
    if (this._disposed) return;
    rpc.call('window.hideOutputChannel', {
      id: this._id,
      name: this._name,
    });
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    rpc.call('window.disposeOutputChannel', {
      id: this._id,
      name: this._name,
    });
  }

  trace(message) {
    this.appendLine(`[trace] ${message ?? ''}`);
  }

  debug(message) {
    this.appendLine(`[debug] ${message ?? ''}`);
  }

  info(message) {
    this.appendLine(`[info] ${message ?? ''}`);
  }

  warn(message) {
    this.appendLine(`[warn] ${message ?? ''}`);
  }

  error(message) {
    this.appendLine(`[error] ${message ?? ''}`);
  }
}

let nextStatusBarId = 1;
let nextOutputChannelId = 1;
let nextTerminalId = 1;

const window = {
  // Messages
  async showInformationMessage(message, ...items) {
    return await rpc.call('window.showInformationMessage', { message, items });
  },

  async showWarningMessage(message, ...items) {
    return await rpc.call('window.showWarningMessage', { message, items });
  },

  async showErrorMessage(message, ...items) {
    return await rpc.call('window.showErrorMessage', { message, items });
  },

  // Quick Pick & Input
  async showQuickPick(items, options = {}, token) {
    const rawItems = Array.isArray(items) ? items : await Promise.resolve(items);
    const normalized = rawItems.map(i => (typeof i === 'string' ? { label: i } : i));
    return await rpc.call('window.showQuickPick', { items: normalized, options });
  },

  async showInputBox(options = {}, token) {
    return await rpc.call('window.showInputBox', { options });
  },

  // Status Bar
  createStatusBarItem(alignment = StatusBarAlignment.Left, priority = 0) {
    const id = `sb_item_${nextStatusBarId++}`;
    return new StatusBarItemProxy(id, alignment, priority);
  },

  // Tree Data Provider
  registerTreeDataProvider(viewId, provider) {
    return rpc.registerObjectProxy('TreeDataProvider', viewId, provider, [
      'getTreeItem',
      'getChildren',
      'getParent',
      'resolveTreeItem',
    ]);
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
    rpc.call('window.createOutputChannel', {
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
    rpc.call('window.createTerminal', { id, ...options });

    return {
      name: options.name || 'Terminal',
      show(preserveFocus = false) {
        rpc.call('window.showTerminal', { id, preserveFocus: Boolean(preserveFocus) });
      },
      sendText(text, addNewLine = true) {
        rpc.call('window.sendTerminalText', { id, text, addNewLine });
      },
      dispose() {
        rpc.call('window.disposeTerminal', { id });
      }
    };
  },

  // Webview
  createWebviewPanel(viewType, title, showOptions = {}, options = {}) {
    const id = `webview_${Date.now()}_${Math.random().toString(36).substring(2, 7)}`;
    let _html = '';
    const messageEvent = rpc.createEvent(`webview.onDidReceiveMessage.${id}`);
    const disposeEvent = rpc.createEvent(`webview.onDidDispose.${id}`);

    rpc.call('window.createWebviewPanel', { id, viewType, title, showOptions, options });

    const webview = {
      get html() {
        return _html;
      },
      set html(value) {
        _html = value;
        rpc.call('window.setWebviewHtml', { id, html: value });
      },
      postMessage(message) {
        return rpc.call('window.postMessageToWebview', { id, message });
      },
      onDidReceiveMessage: messageEvent,
      options,
    };

    return {
      viewType,
      title,
      webview,
      reveal(viewColumn) {
        rpc.call('window.revealWebviewPanel', { id, viewColumn });
      },
      dispose() {
        rpc.call('window.disposeWebviewPanel', { id });
        rpc.dispatchNotification(`webview.onDidDispose.${id}`, {});
      },
      onDidDispose: disposeEvent,
    };
  },

  // Events
  onDidChangeActiveTextEditor: rpc.createEvent('window.onDidChangeActiveTextEditor'),
  onDidChangeVisibleTextEditors: rpc.createEvent('window.onDidChangeVisibleTextEditors'),
  onDidChangeTextEditorSelection: rpc.createEvent('window.onDidChangeTextEditorSelection'),
};

module.exports = window;
