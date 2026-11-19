// vscode.workspace API implementation

const rpc = require('../rpc-bridge');
const { Uri } = require('./types');

function hasOwn(obj, key) {
  return Object.prototype.hasOwnProperty.call(obj, key);
}

function hasUriScheme(str) {
  const colonIdx = str.indexOf(':');
  // Single-letter prefixes (e.g. `C:\...` or `C:/...`) are Windows drive paths, not URI schemes.
  if (colonIdx <= 1) {
    return false;
  }
  const scheme = str.slice(0, colonIdx);
  for (let i = 0; i < scheme.length; i++) {
    const ch = scheme.charCodeAt(i);
    const isAlpha = (ch >= 65 && ch <= 90) || (ch >= 97 && ch <= 122);
    const isDigit = ch >= 48 && ch <= 57;
    const isExtra = ch === 43 || ch === 45 || ch === 46;
    if (i === 0 ? !isAlpha : !(isAlpha || isDigit || isExtra)) {
      return false;
    }
  }
  return true;
}

class WorkspaceConfiguration {
  constructor(section, data = {}, scope = undefined) {
    this._section = section;
    this._data = data && typeof data === 'object' ? data : {};
    this._scope = scope;
  }

  _resolve(section) {
    if (!this._data || typeof this._data !== 'object' || typeof section !== 'string' || section.length === 0) {
      return { found: false, value: undefined };
    }
    if (hasOwn(this._data, section)) {
      return { found: true, value: this._data[section] };
    }
    if (section.indexOf('.') === -1) {
      return { found: false, value: undefined };
    }
    const parts = section.split('.');
    let cur = this._data;
    for (let i = 0; i < parts.length; i++) {
      if (!cur || typeof cur !== 'object') {
        return { found: false, value: undefined };
      }
      if (hasOwn(cur, parts[i])) {
        cur = cur[parts[i]];
        if (i === parts.length - 1) {
          return { found: true, value: cur };
        }
      } else {
        const remainder = parts.slice(i).join('.');
        if (hasOwn(cur, remainder)) {
          return { found: true, value: cur[remainder] };
        }
        return { found: false, value: undefined };
      }
    }
    return { found: false, value: undefined };
  }

  get(section, defaultValue) {
    const res = this._resolve(section);
    return res.found && res.value !== undefined ? res.value : defaultValue;
  }

  has(section) {
    const res = this._resolve(section);
    return Boolean(res.found && res.value !== undefined);
  }

  inspect(section) {
    const fullKey = (this._section && section) ? `${this._section}.${section}` : (this._section || section || '');
    const remote = rpc.callSync('workspace.inspectConfiguration', {
      key: fullKey,
      scope: this._scope,
    });
    if (remote && typeof remote === 'object') {
      return remote;
    }
    const res = this._resolve(section);
    return {
      key: fullKey,
      defaultValue: res.found ? res.value : undefined,
      globalValue: undefined,
      workspaceValue: undefined,
      workspaceFolderValue: undefined,
    };
  }

  async update(section, value, configurationTarget) {
    const fullKey = (this._section && section) ? `${this._section}.${section}` : (this._section || section || '');
    return await rpc.call('workspace.updateConfiguration', {
      key: fullKey,
      value,
      target: configurationTarget,
      scope: this._scope,
    });
  }
}

class TextDocumentProxy {
  constructor(raw) {
    this.uri = typeof raw.uri === 'string' ? Uri.parse(raw.uri) : raw.uri;
    this.fileName = raw.fileName || (this.uri ? this.uri.fsPath : '');
    this.languageId = raw.languageId || 'plaintext';
    this.version = typeof raw.version === 'number' ? raw.version : 1;
    this.isDirty = Boolean(raw.isDirty);
    this.isUntitled = Boolean(raw.isUntitled);
    this.eol = raw.eol || 1;
    this.text = typeof raw.text === 'string' ? raw.text : '';
    this._lines = this.text.split('\n');
    this.lineCount = typeof raw.lineCount === 'number' ? raw.lineCount : this._lines.length;
  }

  getText(range) {
    if (!range || !range.start || !range.end) {
      return this.text;
    }
    const startLine = Math.max(0, Math.min(range.start.line, this._lines.length - 1));
    const endLine = Math.max(startLine, Math.min(range.end.line, this._lines.length - 1));
    if (startLine === endLine) {
      return this._lines[startLine].slice(range.start.character, range.end.character);
    }
    const out = [this._lines[startLine].slice(range.start.character)];
    for (let i = startLine + 1; i < endLine; i++) {
      out.push(this._lines[i]);
    }
    out.push(this._lines[endLine].slice(0, range.end.character));
    return out.join('\n');
  }

  lineAt(lineOrPos) {
    const lineNum = typeof lineOrPos === 'number' ? lineOrPos : (lineOrPos && lineOrPos.line) || 0;
    const clamped = Math.max(0, Math.min(lineNum, this._lines.length - 1));
    const lineText = this._lines[clamped] || '';
    return {
      lineNumber: clamped,
      text: lineText.endsWith('\r') ? lineText.slice(0, -1) : lineText,
      isEmptyOrWhitespace: lineText.trim().length === 0,
    };
  }
}

const workspace = {
  rootPath: null,
  workspaceFolders: [],

  getConfiguration(section, scope) {
    const data = rpc.callSync('workspace.getConfiguration', { section, scope });
    return new WorkspaceConfiguration(section, data || {}, scope);
  },

  async openTextDocument(uriOrFileName) {
    let payload;
    if (typeof uriOrFileName === 'string') {
      const uri = hasUriScheme(uriOrFileName) ? Uri.parse(uriOrFileName) : Uri.file(uriOrFileName);
      payload = { uri: uri.toString() };
    } else if (uriOrFileName && typeof uriOrFileName === 'object' && !(uriOrFileName instanceof Uri) && ('language' in uriOrFileName || 'content' in uriOrFileName) && !('uri' in uriOrFileName) && !('scheme' in uriOrFileName)) {
      payload = {
        language: uriOrFileName.language,
        content: uriOrFileName.content,
      };
    } else if (uriOrFileName && typeof uriOrFileName.toString === 'function') {
      payload = { uri: uriOrFileName.toString() };
    } else {
      payload = { uri: String(uriOrFileName || '') };
    }

    const res = await rpc.call('workspace.openTextDocument', payload);
    if (res && typeof res === 'object') {
      return new TextDocumentProxy(res);
    }
    return res;
  },

  // Events
  onDidChangeConfiguration: rpc.createEvent('workspace.onDidChangeConfiguration'),
  onDidOpenTextDocument: rpc.createEvent('workspace.onDidOpenTextDocument'),
  onDidCloseTextDocument: rpc.createEvent('workspace.onDidCloseTextDocument'),
  onDidChangeTextDocument: rpc.createEvent('workspace.onDidChangeTextDocument'),
  onDidSaveTextDocument: rpc.createEvent('workspace.onDidSaveTextDocument'),
  onDidOpenNotebookDocument: rpc.createEvent('notebooks.onDidOpenNotebookDocument'),
  onDidCloseNotebookDocument: rpc.createEvent('notebooks.onDidCloseNotebookDocument'),
  onDidChangeNotebookDocument: rpc.createEvent('notebooks.onDidChangeNotebookDocument'),
  onDidSaveNotebookDocument: rpc.createEvent('notebooks.onDidSaveNotebookDocument'),
};

module.exports = workspace;
