// vscode.notebooks API and notebook types implementation

const rpc = require('../rpc-bridge');
const { Disposable, CancellationTokenSource } = require('./types');

function _encodeUtf8(str) {
  const input = typeof str === 'string' ? str : String(str ?? '');
  if (typeof TextEncoder !== 'undefined') {
    return new TextEncoder().encode(input);
  }
  const bytes = [];
  for (let i = 0; i < input.length; i++) {
    let code = input.charCodeAt(i);
    if (code < 0x80) {
      bytes.push(code);
    } else if (code < 0x800) {
      bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
    } else if (code >= 0xd800 && code <= 0xdbff && i + 1 < input.length) {
      const low = input.charCodeAt(++i);
      code = 0x10000 + (((code & 0x3ff) << 10) | (low & 0x3ff));
      bytes.push(
        0xf0 | (code >> 18),
        0x80 | ((code >> 12) & 0x3f),
        0x80 | ((code >> 6) & 0x3f),
        0x80 | (code & 0x3f)
      );
    } else {
      bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    }
  }
  return new Uint8Array(bytes);
}

const NotebookCellKind = {
  Markup: 1,
  Code: 2,
};

class NotebookCellOutputItem {
  constructor(data, mime) {
    if (data instanceof Uint8Array) {
      this.data = data;
    } else if (Array.isArray(data)) {
      this.data = Uint8Array.from(data);
    } else if (typeof data === 'string') {
      this.data = _encodeUtf8(data);
    } else if (data && typeof data.byteLength === 'number') {
      this.data = new Uint8Array(data);
    } else {
      this.data = new Uint8Array(0);
    }
    this.mime = mime;
  }

  static text(value, mime = 'text/plain') {
    return new NotebookCellOutputItem(_encodeUtf8(value), mime);
  }

  static json(value, mime = 'application/json') {
    return new NotebookCellOutputItem(_encodeUtf8(JSON.stringify(value, null, 1)), mime);
  }

  static stdout(value) {
    return new NotebookCellOutputItem(_encodeUtf8(value), 'application/vnd.code.notebook.stdout');
  }

  static stderr(value) {
    return new NotebookCellOutputItem(_encodeUtf8(value), 'application/vnd.code.notebook.stderr');
  }

  static error(err) {
    const payload = {
      name: (err && err.name) || 'Error',
      message: (err && err.message) || String(err || ''),
      stack: (err && err.stack) || '',
    };
    return NotebookCellOutputItem.json(payload, 'application/vnd.code.notebook.error');
  }

  toJSON() {
    return {
      mime: this.mime,
      data: Array.from(this.data),
    };
  }
}

class NotebookCellOutput {
  constructor(items, metadata) {
    this.items = Array.isArray(items) ? items : items ? [items] : [];
    this.metadata = metadata;
  }

  toJSON() {
    return {
      items: this.items,
      metadata: this.metadata,
    };
  }
}

class NotebookCellData {
  constructor(kind, value, languageId) {
    this.kind = kind;
    this.value = value;
    this.languageId = languageId;
    this.outputs = [];
    this.metadata = undefined;
    this.executionSummary = undefined;
  }
}

class NotebookData {
  constructor(cells = []) {
    this.cells = cells;
    this.metadata = undefined;
  }
}

class NotebookRange {
  constructor(start, end) {
    this.start = start;
    this.end = end;
  }

  get isEmpty() {
    return this.start >= this.end;
  }

  with(change) {
    return new NotebookRange(
      change && change.start !== undefined ? change.start : this.start,
      change && change.end !== undefined ? change.end : this.end
    );
  }
}

class NotebookEdit {
  constructor(range, newCells) {
    this.range = range;
    this.newCells = newCells || [];
    this.newCellMetadata = undefined;
    this.newNotebookMetadata = undefined;
  }

  static replaceCells(range, newCells) {
    return new NotebookEdit(range, newCells);
  }

  static insertCells(index, newCells) {
    return new NotebookEdit(new NotebookRange(index, index), newCells);
  }

  static deleteCells(range) {
    return new NotebookEdit(range, []);
  }

  static updateCellMetadata(index, newCellMetadata) {
    const edit = new NotebookEdit(new NotebookRange(index, index + 1), []);
    edit.newCellMetadata = newCellMetadata;
    return edit;
  }

  static updateNotebookMetadata(newNotebookMetadata) {
    const edit = new NotebookEdit(new NotebookRange(0, 0), []);
    edit.newNotebookMetadata = newNotebookMetadata;
    return edit;
  }
}

class NotebookCellExecution {
  constructor(controllerId, cell) {
    this._controllerId = controllerId;
    this.cell = cell;
    this._cts = new CancellationTokenSource();
    this.token = this._cts.token;
    this.executionOrder = undefined;
  }

  _cellMeta() {
    const c = this.cell || {};
    return {
      notebookUri:
        (c.notebook && c.notebook.uri && c.notebook.uri.toString()) ||
        c.notebookUri ||
        '',
      cellIndex: typeof c.index === 'number' ? c.index : 0,
    };
  }

  start(startTime) {
    const meta = this._cellMeta();
    return rpc.call('notebooks.updateExecution', {
      controllerId: this._controllerId,
      notebookUri: meta.notebookUri,
      cellIndex: meta.cellIndex,
      state: 'start',
      executionOrder: this.executionOrder,
      startTime: startTime !== undefined ? startTime : Date.now(),
    });
  }

  end(success, endTime) {
    const meta = this._cellMeta();
    return rpc.call('notebooks.updateExecution', {
      controllerId: this._controllerId,
      notebookUri: meta.notebookUri,
      cellIndex: meta.cellIndex,
      state: 'end',
      success: success !== false,
      executionOrder: this.executionOrder,
      endTime: endTime !== undefined ? endTime : Date.now(),
    });
  }

  clearOutput(cell) {
    const target = cell || this.cell;
    const notebookUri =
      (target && target.notebook && target.notebook.uri && target.notebook.uri.toString()) ||
      (target && target.notebookUri) ||
      '';
    const cellIndex = target && typeof target.index === 'number' ? target.index : 0;
    return rpc.call('notebooks.updateCellOutputs', {
      controllerId: this._controllerId,
      notebookUri,
      cellIndex,
      outputs: [],
    });
  }

  replaceOutput(out, cell) {
    const target = cell || this.cell;
    const notebookUri =
      (target && target.notebook && target.notebook.uri && target.notebook.uri.toString()) ||
      (target && target.notebookUri) ||
      '';
    const cellIndex = target && typeof target.index === 'number' ? target.index : 0;
    const outputs = Array.isArray(out) ? out : out ? [out] : [];
    return rpc.call('notebooks.updateCellOutputs', {
      controllerId: this._controllerId,
      notebookUri,
      cellIndex,
      outputs,
    });
  }

  appendOutput(out, cell) {
    const target = cell || this.cell;
    const notebookUri =
      (target && target.notebook && target.notebook.uri && target.notebook.uri.toString()) ||
      (target && target.notebookUri) ||
      '';
    const cellIndex = target && typeof target.index === 'number' ? target.index : 0;
    const outputs = Array.isArray(out) ? out : out ? [out] : [];
    return rpc.call('notebooks.appendCellOutputs', {
      controllerId: this._controllerId,
      notebookUri,
      cellIndex,
      outputs,
    });
  }
}

class NotebookController {
  constructor(id, notebookType, label, handler) {
    this.id = id;
    this.notebookType = notebookType;
    this.label = label;
    this.description = undefined;
    this.detail = undefined;
    this.supportedLanguages = [];
    this.supportsExecutionOrder = false;
    this.executeHandler = handler;
    this.interruptHandler = undefined;
    this.onDidChangeSelectedNotebooks = rpc.createEvent(
      `notebooks.onDidChangeSelectedNotebooks.${id}`
    );
    this._onDispose = null;
  }

  createNotebookCellExecution(cell) {
    return new NotebookCellExecution(this.id, cell);
  }

  updateNotebookAffinity(notebook, affinity) {
    const uri = notebook && notebook.uri ? notebook.uri.toString() : String(notebook || '');
    return rpc.call('notebooks.updateControllerAffinity', {
      id: this.id,
      notebookUri: uri,
      affinity,
    });
  }

  dispose() {
    if (typeof this._onDispose === 'function') {
      this._onDispose();
      this._onDispose = null;
    }
  }
}

const notebooks = {
  registerNotebookSerializer(notebookType, serializer, options = {}) {
    const proxyDisp = rpc.registerObjectProxy(
      'NotebookSerializer',
      notebookType,
      serializer,
      ['deserializeNotebook', 'serializeNotebook'],
      { selector: notebookType, notebookType, options }
    );
    rpc.call('notebooks.registerSerializer', {
      id: notebookType,
      notebookType,
      options,
    });
    return new Disposable(() => {
      proxyDisp.dispose();
      rpc.call('notebooks.unregisterSerializer', {
        id: notebookType,
        notebookType,
      });
    });
  },

  createNotebookController(id, notebookType, label, handler) {
    const controller = new NotebookController(id, notebookType, label, handler);
    const proxyDisp = rpc.registerObjectProxy(
      'NotebookController',
      id,
      {
        execute: (cells, notebook) => {
          if (typeof controller.executeHandler === 'function') {
            return controller.executeHandler(cells, notebook, controller);
          }
          return undefined;
        },
        interrupt: (notebook) => {
          if (typeof controller.interruptHandler === 'function') {
            return controller.interruptHandler(notebook);
          }
          return undefined;
        },
      },
      ['execute', 'interrupt'],
      { selector: notebookType, id, notebookType, label }
    );
    rpc.call('notebooks.createController', {
      id,
      notebookType,
      label,
      supportedLanguages: controller.supportedLanguages,
      supportsExecutionOrder: controller.supportsExecutionOrder,
    });
    controller._onDispose = () => {
      proxyDisp.dispose();
      rpc.call('notebooks.disposeController', { id });
    };
    return controller;
  },

  createNotebookCellExecution(cell, controllerId = 'default') {
    return new NotebookCellExecution(controllerId, cell);
  },

  // Events
  onDidOpenNotebookDocument: rpc.createEvent('notebooks.onDidOpenNotebookDocument'),
  onDidCloseNotebookDocument: rpc.createEvent('notebooks.onDidCloseNotebookDocument'),
  onDidChangeNotebookDocument: rpc.createEvent('notebooks.onDidChangeNotebookDocument'),
  onDidSaveNotebookDocument: rpc.createEvent('notebooks.onDidSaveNotebookDocument'),
  onDidChangeActiveNotebookEditor: rpc.createEvent('notebooks.onDidChangeActiveNotebookEditor'),
  onDidChangeVisibleNotebookEditors: rpc.createEvent('notebooks.onDidChangeVisibleNotebookEditors'),
  onDidChangeNotebookEditorSelection: rpc.createEvent('notebooks.onDidChangeNotebookEditorSelection'),
};

module.exports = {
  notebooks,
  NotebookCellKind,
  NotebookCellData,
  NotebookData,
  NotebookCellOutput,
  NotebookCellOutputItem,
  NotebookRange,
  NotebookEdit,
  NotebookController,
  NotebookCellExecution,
};
