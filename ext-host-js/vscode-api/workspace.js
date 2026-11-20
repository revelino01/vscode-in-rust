// vscode.workspace API implementation

const rpc = require('../rpc-bridge');
const {
  Uri,
  Position,
  Range,
  EventEmitter,
  FileType,
  FileSystemError,
  EndOfLine,
} = require('./types');

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

function uriFromInput(input) {
  if (!input) {
    return Uri.file('');
  }
  if (input instanceof Uri) {
    return input;
  }
  if (typeof input === 'string') {
    return hasUriScheme(input) ? Uri.parse(input) : Uri.file(input);
  }
  if (typeof input === 'object' && typeof input.scheme === 'string') {
    return Uri.from(input);
  }
  if (typeof input.toString === 'function') {
    const s = input.toString();
    return hasUriScheme(s) ? Uri.parse(s) : Uri.file(s);
  }
  return Uri.file(String(input));
}

function normalizePathForCompare(rawPath) {
  if (typeof rawPath !== 'string' || !rawPath) {
    return '';
  }
  let slash = rawPath.split('\\').join('/');
  if (slash.length > 1 && !/^[A-Za-z]:\/$/.test(slash)) {
    slash = slash.replace(/\/+$/, '');
  }
  const isWinPath = /^[A-Za-z]:(\/|$)/.test(slash) || slash.startsWith('//');
  return isWinPath ? slash.toLowerCase() : slash;
}

function canonicalDocKey(uriLike) {
  const uri = uriFromInput(uriLike);
  if (uri.scheme === 'untitled') {
    const name = uri.path.replace(/^\/+/, '');
    return `untitled:${name}`;
  }
  if (uri.scheme === 'file') {
    return `file:${normalizePathForCompare(uri.fsPath || uri.path)}`;
  }
  return uri.toString();
}

function pathStartsWithNormalized(targetNorm, baseNorm) {
  if (!targetNorm || !baseNorm) {
    return false;
  }
  if (targetNorm === baseNorm) {
    return true;
  }
  if (baseNorm === '/' || /^[a-z]:\/$/.test(baseNorm)) {
    return targetNorm.startsWith(baseNorm);
  }
  return targetNorm.startsWith(baseNorm + '/');
}

function traverseConfigSection(root, section) {
  if (!root || typeof root !== 'object') {
    return undefined;
  }
  if (!section) {
    return root;
  }
  if (hasOwn(root, section)) {
    return root[section];
  }
  const parts = section.split('.');
  let cur = root;
  for (let i = 0; i < parts.length; i++) {
    if (!cur || typeof cur !== 'object') {
      return undefined;
    }
    if (hasOwn(cur, parts[i])) {
      cur = cur[parts[i]];
      if (i === parts.length - 1) {
        return cur;
      }
    } else {
      const rem = parts.slice(i).join('.');
      if (hasOwn(cur, rem)) {
        return cur[rem];
      }
      return undefined;
    }
  }
  return undefined;
}

function setNestedConfigValue(root, path, value) {
  if (!root || typeof root !== 'object' || !path) {
    return;
  }
  const parts = path.split('.');
  let cur = root;
  for (let i = 0; i < parts.length - 1; i++) {
    const k = parts[i];
    if (!cur[k] || typeof cur[k] !== 'object' || Array.isArray(cur[k])) {
      cur[k] = {};
    }
    cur = cur[k];
  }
  cur[parts[parts.length - 1]] = value;
}

function deleteNestedConfigValue(root, path) {
  if (!root || typeof root !== 'object' || !path) {
    return;
  }
  delete root[path];
  const parts = path.split('.');
  let cur = root;
  for (let i = 0; i < parts.length - 1; i++) {
    if (!cur || typeof cur !== 'object') return;
    cur = cur[parts[i]];
  }
  if (cur && typeof cur === 'object') {
    delete cur[parts[parts.length - 1]];
  }
}

function deepMergeConfig(target, src) {
  if (!src || typeof src !== 'object' || Array.isArray(src)) {
    return src;
  }
  const out = target && typeof target === 'object' && !Array.isArray(target) ? { ...target } : {};
  for (const key of Object.keys(src)) {
    const val = src[key];
    if (val && typeof val === 'object' && !Array.isArray(val) && out[key] && typeof out[key] === 'object' && !Array.isArray(out[key])) {
      out[key] = deepMergeConfig(out[key], val);
    } else {
      out[key] = val;
    }
  }
  return out;
}

const _configCache = {
  root: {},
  folders: new Map(), // normFsPath -> object
};

let _workspaceFolders = [];
const _openDocuments = new Map();

const _onDidChangeWorkspaceFolders = new EventEmitter();
const _onDidChangeConfiguration = new EventEmitter();
const _onDidOpenTextDocument = new EventEmitter();
const _onDidChangeTextDocument = new EventEmitter();
const _onDidSaveTextDocument = new EventEmitter();
const _onDidCloseTextDocument = new EventEmitter();

function normalizeWorkspaceFolder(raw, fallbackIdx = 0) {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const uri = raw.uri ? uriFromInput(raw.uri) : Uri.file(raw.path || '');
  const idx = typeof raw.index === 'number' ? raw.index : fallbackIdx;
  const fsPath = uri.fsPath || raw.path || '';
  const parts = fsPath.replace(/[\\/]+$/, '').split(/[\\/]/);
  const name = (typeof raw.name === 'string' && raw.name) || parts[parts.length - 1] || fsPath;
  return { uri, name, index: idx };
}

function applyWorkspaceFoldersList(list) {
  if (!Array.isArray(list)) {
    return { added: [], removed: [] };
  }
  const next = list.map((f, i) => normalizeWorkspaceFolder(f, i)).filter(Boolean);
  const prev = _workspaceFolders;
  const prevKeys = new Set(prev.map(f => normalizePathForCompare(f.uri.fsPath || f.uri.path)));
  const nextKeys = new Set(next.map(f => normalizePathForCompare(f.uri.fsPath || f.uri.path)));
  const added = next.filter(f => !prevKeys.has(normalizePathForCompare(f.uri.fsPath || f.uri.path)));
  const removed = prev.filter(f => !nextKeys.has(normalizePathForCompare(f.uri.fsPath || f.uri.path)));
  _workspaceFolders = next;
  return { added, removed };
}

function resolveScopeFsPath(scope) {
  if (!scope) {
    return null;
  }
  if (typeof scope === 'string') {
    return uriFromInput(scope).fsPath;
  }
  if (scope instanceof Uri) {
    return scope.fsPath;
  }
  if (typeof scope === 'object') {
    if (scope.uri) {
      return uriFromInput(scope.uri).fsPath;
    }
    if (typeof scope.fsPath === 'string') {
      return scope.fsPath;
    }
    if (typeof scope.path === 'string') {
      return scope.path;
    }
  }
  return null;
}

function getMergedConfigFromCache(section, scope) {
  let merged = _configCache.root || {};
  const scopePath = resolveScopeFsPath(scope);
  if (scopePath && _configCache.folders.size > 0) {
    const scopeNorm = normalizePathForCompare(scopePath);
    const matching = [];
    for (const [folderNorm, folderCfg] of _configCache.folders.entries()) {
      if (pathStartsWithNormalized(scopeNorm, folderNorm)) {
        matching.push([folderNorm, folderCfg]);
      }
    }
    matching.sort((a, b) => a[0].length - b[0].length);
    for (const [, folderCfg] of matching) {
      merged = deepMergeConfig(merged, folderCfg);
    }
  }
  if (!section) {
    return merged;
  }
  const sub = traverseConfigSection(merged, section);
  return sub && typeof sub === 'object' && !Array.isArray(sub) ? sub : {};
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
    const res = await rpc.call('workspace.updateConfiguration', {
      key: fullKey,
      value,
      target: configurationTarget,
      scope: this._scope,
    });
    // Refresh local snapshot from Rust if live RPC responded, otherwise update local view directly
    if (res !== null && res !== undefined) {
      const refreshed = await rpc.call('workspace.getConfiguration', {
        section: this._section,
        scope: this._scope,
      });
      if (refreshed && typeof refreshed === 'object') {
        this._data = refreshed;
      }
      const refreshedRoot = await rpc.call('workspace.getConfiguration', {});
      if (refreshedRoot && typeof refreshedRoot === 'object') {
        _configCache.root = refreshedRoot;
      }
    } else if (section) {
      if (value === undefined) {
        deleteNestedConfigValue(this._data, section);
      } else {
        setNestedConfigValue(this._data, section, value);
      }
    }
    return res;
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
    this.isClosed = Boolean(raw.isClosed);
    this.text = typeof raw.text === 'string' ? raw.text : '';
    this.eol = raw.eol || (this.text.indexOf('\r\n') !== -1 ? EndOfLine.CRLF : EndOfLine.LF);
    this._rebuildLines();
    this.lineCount = typeof raw.lineCount === 'number' ? raw.lineCount : this._lines.length;
  }

  _rebuildLines() {
    this._lines = [];
    this._lineStarts = [];
    let start = 0;
    const len = this.text.length;
    for (let i = 0; i <= len; i++) {
      if (i === len || this.text.charCodeAt(i) === 10) {
        let lineEnd = i;
        if (lineEnd > start && this.text.charCodeAt(lineEnd - 1) === 13) {
          lineEnd--;
        }
        this._lineStarts.push(start);
        this._lines.push(this.text.slice(start, lineEnd));
        start = i + 1;
      }
    }
    if (this._lines.length === 0) {
      this._lineStarts.push(0);
      this._lines.push('');
    }
  }

  _updateFromRaw(raw) {
    if (!raw || typeof raw !== 'object') return;
    if (raw.languageId !== undefined) this.languageId = raw.languageId;
    if (typeof raw.version === 'number') this.version = raw.version;
    if (raw.isDirty !== undefined) this.isDirty = Boolean(raw.isDirty);
    if (raw.isUntitled !== undefined) this.isUntitled = Boolean(raw.isUntitled);
    if (raw.isClosed !== undefined) this.isClosed = Boolean(raw.isClosed);
    if (typeof raw.text === 'string') {
      this.text = raw.text;
      this.eol = raw.eol || (this.text.indexOf('\r\n') !== -1 ? EndOfLine.CRLF : EndOfLine.LF);
      this._rebuildLines();
      this.lineCount = typeof raw.lineCount === 'number' ? raw.lineCount : this._lines.length;
    } else if (typeof raw.eol === 'number') {
      this.eol = raw.eol;
    }
  }

  validatePosition(position) {
    const line = Math.max(0, Math.min(Math.trunc((position && position.line) || 0), this._lines.length - 1));
    const maxChar = this._lines[line].length;
    const character = Math.max(0, Math.min(Math.trunc((position && position.character) || 0), maxChar));
    return new Position(line, character);
  }

  validateRange(range) {
    const start = this.validatePosition(range && range.start);
    const end = this.validatePosition(range && range.end);
    if (start.isAfter(end)) {
      return new Range(end, start);
    }
    return new Range(start, end);
  }

  offsetAt(position) {
    const p = this.validatePosition(position);
    return this._lineStarts[p.line] + p.character;
  }

  positionAt(offset) {
    const clamped = Math.max(0, Math.min(Math.trunc(offset || 0), this.text.length));
    let lo = 0;
    let hi = this._lineStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this._lineStarts[mid] <= clamped) {
        lo = mid;
      } else {
        hi = mid - 1;
      }
    }
    const col = Math.min(clamped - this._lineStarts[lo], this._lines[lo].length);
    return new Position(lo, col);
  }

  getText(range) {
    if (!range || !range.start || !range.end) {
      return this.text;
    }
    const startOffset = this.offsetAt(range.start);
    const endOffset = this.offsetAt(range.end);
    const lo = Math.min(startOffset, endOffset);
    const hi = Math.max(startOffset, endOffset);
    return this.text.slice(lo, hi);
  }

  lineAt(lineOrPos) {
    const lineNum = typeof lineOrPos === 'number' ? lineOrPos : (lineOrPos && lineOrPos.line) || 0;
    const clamped = Math.max(0, Math.min(Math.trunc(lineNum), this._lines.length - 1));
    const text = this._lines[clamped] || '';
    const firstNonWs = text.search(/\S/);
    const lineStart = this._lineStarts[clamped];
    const nextStart = clamped + 1 < this._lineStarts.length ? this._lineStarts[clamped + 1] : this.text.length;
    const endInc = this.positionAt(nextStart);
    return {
      lineNumber: clamped,
      text,
      range: new Range(clamped, 0, clamped, text.length),
      rangeIncludingLineBreak: new Range(new Position(clamped, 0), endInc),
      firstNonWhitespaceCharacterIndex: firstNonWs === -1 ? text.length : firstNonWs,
      isEmptyOrWhitespace: text.trim().length === 0,
      _byteStart: lineStart,
    };
  }

  async save() {
    const res = await rpc.call('workspace.saveTextDocument', {
      uri: this.uri.toString(),
    });
    if (res === true) {
      this.isDirty = false;
      return true;
    }
    return false;
  }
}

function upsertDocumentProxy(raw) {
  if (!raw || typeof raw !== 'object' || !raw.uri) {
    return null;
  }
  const key = canonicalDocKey(raw.uri);
  let doc = _openDocuments.get(key);
  if (doc) {
    doc._updateFromRaw(raw);
  } else {
    doc = new TextDocumentProxy(raw);
    _openDocuments.set(key, doc);
  }
  return doc;
}

function mapFsRpcError(err, uri) {
  if (!err || typeof err !== 'object') {
    return err;
  }
  const code = err.code;
  if (code === 'ENOENT') return FileSystemError.FileNotFound(uri);
  if (code === 'EEXIST') return FileSystemError.FileExists(uri);
  if (code === 'ENOTDIR') return FileSystemError.FileNotADirectory(uri);
  if (code === 'EISDIR') return FileSystemError.FileIsADirectory(uri);
  if (code === 'EACCES' || code === 'EPERM') return FileSystemError.NoPermissions(uri);
  return err;
}

const workspaceFs = {
  async stat(uri) {
    const u = uriFromInput(uri);
    try {
      const st = await rpc.call('fs.stat', { path: u.toString() });
      if (!st) {
        return { type: FileType.Unknown, ctime: 0, mtime: 0, size: 0 };
      }
      let type = FileType.Unknown;
      if (st.isFile) type |= FileType.File;
      if (st.isDirectory) type |= FileType.Directory;
      if (st.isSymbolicLink) type |= FileType.SymbolicLink;
      return {
        type,
        ctime: 0,
        mtime: typeof st.mtimeMs === 'number' ? st.mtimeMs : 0,
        size: typeof st.size === 'number' ? st.size : 0,
      };
    } catch (err) {
      throw mapFsRpcError(err, u);
    }
  },

  async readDirectory(uri) {
    const u = uriFromInput(uri);
    try {
      const entries = await rpc.call('fs.readdir', { path: u.toString(), withFileTypes: true });
      if (!Array.isArray(entries)) {
        return [];
      }
      return entries.map((e) => {
        if (typeof e === 'string') {
          return [e, FileType.Unknown];
        }
        let t = FileType.Unknown;
        if (e.isFile) t |= FileType.File;
        if (e.isDirectory) t |= FileType.Directory;
        if (e.isSymbolicLink) t |= FileType.SymbolicLink;
        return [e.name, t];
      });
    } catch (err) {
      throw mapFsRpcError(err, u);
    }
  },

  async createDirectory(uri) {
    const u = uriFromInput(uri);
    try {
      await rpc.call('fs.mkdir', { path: u.toString(), options: { recursive: true } });
    } catch (err) {
      throw mapFsRpcError(err, u);
    }
  },

  async readFile(uri) {
    const u = uriFromInput(uri);
    try {
      const bytes = await rpc.call('fs.readFile', { path: u.toString(), encoding: null });
      if (bytes instanceof Uint8Array) return bytes;
      if (Array.isArray(bytes)) return Uint8Array.from(bytes);
      if (typeof bytes === 'string') return Buffer.from(bytes, 'utf8');
      return new Uint8Array(0);
    } catch (err) {
      throw mapFsRpcError(err, u);
    }
  },

  async writeFile(uri, content) {
    const u = uriFromInput(uri);
    try {
      const data = content instanceof Uint8Array
        ? Array.from(content)
        : (Buffer.isBuffer(content) ? Array.from(content) : String(content ?? ''));
      await rpc.call('fs.writeFile', { path: u.toString(), data });
    } catch (err) {
      throw mapFsRpcError(err, u);
    }
  },

  async delete(uri, options = {}) {
    const u = uriFromInput(uri);
    try {
      await rpc.call('fs.rm', {
        path: u.toString(),
        options: {
          recursive: Boolean(options && options.recursive),
          force: false,
        },
      });
    } catch (err) {
      throw mapFsRpcError(err, u);
    }
  },

  async rename(source, target, options = {}) {
    const src = uriFromInput(source);
    const dst = uriFromInput(target);
    try {
      if (!(options && options.overwrite)) {
        try {
          const existing = await rpc.call('fs.stat', { path: dst.toString() });
          if (existing && !existing.error) {
            throw FileSystemError.FileExists(dst);
          }
        } catch (stErr) {
          if (stErr instanceof FileSystemError) throw stErr;
        }
      }
      await rpc.call('fs.rename', { oldPath: src.toString(), newPath: dst.toString() });
    } catch (err) {
      throw mapFsRpcError(err, src);
    }
  },

  async copy(source, target, options = {}) {
    const src = uriFromInput(source);
    const dst = uriFromInput(target);
    try {
      const mode = options && options.overwrite ? 0 : 1;
      await rpc.call('fs.copyFile', { src: src.toString(), dest: dst.toString(), mode });
    } catch (err) {
      throw mapFsRpcError(err, src);
    }
  },

  isWritableFileSystem(scheme) {
    return scheme === 'file' ? true : undefined;
  },
};

const workspace = {
  get rootPath() {
    return _workspaceFolders.length > 0 ? _workspaceFolders[0].uri.fsPath : null;
  },

  get workspaceFolders() {
    return _workspaceFolders.slice();
  },

  get name() {
    return _workspaceFolders.length > 0 ? _workspaceFolders[0].name : undefined;
  },

  get textDocuments() {
    return Array.from(_openDocuments.values());
  },

  fs: workspaceFs,

  getWorkspaceFolder(uri) {
    if (!uri || _workspaceFolders.length === 0) {
      return undefined;
    }
    const u = uriFromInput(uri);
    const targetNorm = normalizePathForCompare(u.fsPath || u.path);
    if (!targetNorm) {
      return undefined;
    }
    let best = undefined;
    let bestLen = -1;
    for (const folder of _workspaceFolders) {
      const folderNorm = normalizePathForCompare(folder.uri.fsPath || folder.uri.path);
      if (pathStartsWithNormalized(targetNorm, folderNorm) && folderNorm.length > bestLen) {
        best = folder;
        bestLen = folderNorm.length;
      }
    }
    return best;
  },

  asRelativePath(pathOrUri, includeWorkspaceFolder) {
    const rawStr = typeof pathOrUri === 'string'
      ? pathOrUri
      : (pathOrUri && (pathOrUri.fsPath || pathOrUri.path)) || '';
    const u = typeof pathOrUri === 'string'
      ? (hasUriScheme(pathOrUri) ? Uri.parse(pathOrUri) : Uri.file(pathOrUri))
      : uriFromInput(pathOrUri);
    const folder = this.getWorkspaceFolder(u);
    if (!folder) {
      return rawStr.split('\\').join('/');
    }
    const targetNorm = (u.fsPath || u.path).split('\\').join('/');
    const folderFs = (folder.uri.fsPath || folder.uri.path).split('\\').join('/').replace(/\/+$/, '');
    let rel = targetNorm.slice(folderFs.length).replace(/^\/+/, '');
    const shouldPrepend = includeWorkspaceFolder === true ||
      (includeWorkspaceFolder === undefined && _workspaceFolders.length > 1);
    if (shouldPrepend && folder.name) {
      rel = rel ? `${folder.name}/${rel}` : folder.name;
    }
    return rel;
  },

  getConfiguration(section, scope) {
    const syncData = rpc.hasSyncTransport()
      ? rpc.callSync('workspace.getConfiguration', { section, scope })
      : null;
    const data = syncData !== null && syncData !== undefined
      ? syncData
      : getMergedConfigFromCache(section, scope);
    return new WorkspaceConfiguration(section, data || {}, scope);
  },

  async openTextDocument(uriOrFileName) {
    let payload;
    if (uriOrFileName === undefined || uriOrFileName === null) {
      payload = {};
    } else if (typeof uriOrFileName === 'string') {
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
      return upsertDocumentProxy(res);
    }
    return res;
  },

  async saveAll(includeUntitled = false) {
    const res = await rpc.call('workspace.saveAll', { includeUntitled: Boolean(includeUntitled) });
    return res === null ? false : Boolean(res);
  },

  async applyEdit(edit) {
    if (!edit) {
      return false;
    }
    const payload = typeof edit.toJSON === 'function' ? edit.toJSON() : edit;
    const res = await rpc.call('workspace.applyEdit', { edit: payload });
    return res === null ? false : Boolean(res);
  },

  async findFiles(include, exclude, maxResults, token) {
    if (token && token.isCancellationRequested) {
      return [];
    }
    const includePayload = include && typeof include.toJSON === 'function'
      ? include.toJSON()
      : include;
    const excludePayload = exclude === undefined
      ? undefined
      : (exclude && typeof exclude.toJSON === 'function' ? exclude.toJSON() : exclude);
    const params = { include: includePayload };
    if (excludePayload !== undefined) {
      params.exclude = excludePayload;
    }
    if (typeof maxResults === 'number') {
      params.maxResults = maxResults;
    }
    const res = await rpc.call('workspace.findFiles', params);
    if (token && token.isCancellationRequested) {
      return [];
    }
    if (!Array.isArray(res)) {
      return [];
    }
    return res.map((u) => Uri.parse(u));
  },

  // Events
  onDidChangeWorkspaceFolders: _onDidChangeWorkspaceFolders.event,
  onDidChangeConfiguration: _onDidChangeConfiguration.event,
  onDidOpenTextDocument: _onDidOpenTextDocument.event,
  onDidCloseTextDocument: _onDidCloseTextDocument.event,
  onDidChangeTextDocument: _onDidChangeTextDocument.event,
  onDidSaveTextDocument: _onDidSaveTextDocument.event,
  onDidOpenNotebookDocument: rpc.createEvent('notebooks.onDidOpenNotebookDocument'),
  onDidCloseNotebookDocument: rpc.createEvent('notebooks.onDidCloseNotebookDocument'),
  onDidChangeNotebookDocument: rpc.createEvent('notebooks.onDidChangeNotebookDocument'),
  onDidSaveNotebookDocument: rpc.createEvent('notebooks.onDidSaveNotebookDocument'),
};

Object.defineProperty(workspace, '_TextDocumentProxy', {
  value: TextDocumentProxy,
  enumerable: false,
});
Object.defineProperty(workspace, '_uriFromInput', {
  value: uriFromInput,
  enumerable: false,
});
Object.defineProperty(workspace, '_upsertDocumentProxy', {
  value: upsertDocumentProxy,
  enumerable: false,
});

// Wire raw RPC notifications -> cache updates + VS Code proxy events
rpc.createEvent('workspace.onDidChangeWorkspaceFolders')((payload) => {
  const list = Array.isArray(payload) ? payload : (payload && payload.folders);
  if (!Array.isArray(list)) {
    return;
  }
  const diff = applyWorkspaceFoldersList(list);
  if (diff.added.length > 0 || diff.removed.length > 0) {
    _onDidChangeWorkspaceFolders.fire(diff);
  }
});

rpc.createEvent('workspace.onDidChangeConfiguration')((payload) => {
  const affectedKeys = payload && Array.isArray(payload.affectedKeys)
    ? payload.affectedKeys
    : ['*'];
  const rawAffectedFolders = payload && Array.isArray(payload.affectedFolderUris)
    ? payload.affectedFolderUris
    : (payload && payload.scope ? [payload.scope] : []);
  const affectedFolderNorms = rawAffectedFolders
    .map((u) => normalizePathForCompare(uriFromInput(u).fsPath))
    .filter(Boolean);
  if (payload && payload.config && typeof payload.config === 'object') {
    _configCache.root = payload.config;
  }
  if (payload && payload.folderConfigs && typeof payload.folderConfigs === 'object') {
    _configCache.folders.clear();
    for (const [uriStr, cfgVal] of Object.entries(payload.folderConfigs)) {
      const fsPath = uriFromInput(uriStr).fsPath;
      _configCache.folders.set(normalizePathForCompare(fsPath), cfgVal);
    }
  }
  const ev = {
    affectsConfiguration(section, scope) {
      if (typeof section !== 'string' || !section) {
        return false;
      }
      let keyMatches = false;
      if (affectedKeys.includes('*')) {
        keyMatches = true;
      } else {
        for (const k of affectedKeys) {
          if (k === section || k.startsWith(section + '.') || section.startsWith(k + '.')) {
            keyMatches = true;
            break;
          }
        }
      }
      if (!keyMatches) {
        return false;
      }
      if (scope !== undefined && scope !== null && affectedFolderNorms.length > 0) {
        let scopeUriLike = null;
        if (scope instanceof Uri || typeof scope === 'string') {
          scopeUriLike = scope;
        } else if (typeof scope === 'object') {
          if (scope.uri !== undefined && scope.uri !== null) {
            scopeUriLike = scope.uri;
          } else if (typeof scope.scheme === 'string' && typeof scope.path === 'string') {
            scopeUriLike = scope;
          }
        }
        if (scopeUriLike !== null) {
          const scopeUri = uriFromInput(scopeUriLike);
          if (scopeUri.scheme !== 'file') {
            return false;
          }
          const scopeNorm = normalizePathForCompare(scopeUri.fsPath);
          if (!scopeNorm) {
            return false;
          }
          return affectedFolderNorms.some((folderNorm) =>
            pathStartsWithNormalized(scopeNorm, folderNorm)
          );
        }
      }
      return true;
    },
  };
  _onDidChangeConfiguration.fire(ev);
});

rpc.createEvent('workspace.onDidOpenTextDocument')((payload) => {
  const doc = upsertDocumentProxy(payload);
  if (doc) {
    _onDidOpenTextDocument.fire(doc);
  }
});

rpc.createEvent('workspace.onDidChangeTextDocument')((payload) => {
  if (!payload || typeof payload !== 'object') return;
  const rawDoc = payload.document || payload;
  const doc = upsertDocumentProxy(rawDoc);
  if (doc) {
    _onDidChangeTextDocument.fire({
      document: doc,
      contentChanges: Array.isArray(payload.contentChanges) ? payload.contentChanges : [],
      reason: payload.reason === null ? undefined : payload.reason,
    });
  }
});

rpc.createEvent('workspace.onDidSaveTextDocument')((payload) => {
  const doc = upsertDocumentProxy(payload);
  if (doc) {
    doc.isDirty = false;
    _onDidSaveTextDocument.fire(doc);
  }
});

rpc.createEvent('workspace.onDidCloseTextDocument')((payload) => {
  if (!payload || typeof payload !== 'object' || !payload.uri) return;
  const key = canonicalDocKey(payload.uri);
  const existing = _openDocuments.get(key);
  let doc;
  if (existing) {
    existing._updateFromRaw({ ...payload, isClosed: true });
    _openDocuments.delete(key);
    doc = existing;
  } else {
    doc = new TextDocumentProxy({ ...payload, isClosed: true });
  }
  _onDidCloseTextDocument.fire(doc);
});

if (rpc.hasSyncTransport()) {
  const initialFolders = rpc.callSync('workspace.getWorkspaceFolders');
  if (Array.isArray(initialFolders)) {
    applyWorkspaceFoldersList(initialFolders);
  }
  const initialCfg = rpc.callSync('workspace.getConfiguration', {});
  if (initialCfg && typeof initialCfg === 'object') {
    _configCache.root = initialCfg;
  }
  const initialDocs = rpc.callSync('workspace.getTextDocuments');
  if (Array.isArray(initialDocs)) {
    for (const d of initialDocs) {
      upsertDocumentProxy(d);
    }
  }
}

if (typeof rpc.registerInitHook === 'function') {
  rpc.registerInitHook(async () => {
    try {
      const folders = await rpc.call('workspace.getWorkspaceFolders');
      if (Array.isArray(folders) && _workspaceFolders.length === 0) {
        applyWorkspaceFoldersList(folders);
      }
    } catch (_) {}
    try {
      const cfg = await rpc.call('workspace.getConfiguration', {});
      if (cfg && typeof cfg === 'object') {
        _configCache.root = cfg;
      }
    } catch (_) {}
    try {
      const docs = await rpc.call('workspace.getTextDocuments');
      if (Array.isArray(docs)) {
        for (const d of docs) {
          upsertDocumentProxy(d);
        }
      }
    } catch (_) {}
  });
}

module.exports = workspace;
