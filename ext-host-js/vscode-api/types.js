// VS Code API types implemented in JavaScript for extension compatibility.

class Position {
  constructor(line, character) {
    this.line = Math.max(0, line);
    this.character = Math.max(0, character);
  }

  isBefore(other) {
    if (this.line < other.line) return true;
    if (this.line > other.line) return false;
    return this.character < other.character;
  }

  isBeforeOrEqual(other) {
    return this.isBefore(other) || this.isEqual(other);
  }

  isAfter(other) {
    return !this.isBeforeOrEqual(other);
  }

  isAfterOrEqual(other) {
    return !this.isBefore(other);
  }

  isEqual(other) {
    return this.line === other.line && this.character === other.character;
  }

  compareTo(other) {
    if (this.line < other.line) return -1;
    if (this.line > other.line) return 1;
    if (this.character < other.character) return -1;
    if (this.character > other.character) return 1;
    return 0;
  }

  translate(lineDelta = 0, characterDelta = 0) {
    return new Position(this.line + lineDelta, this.character + characterDelta);
  }

  with(line, character) {
    return new Position(
      line !== undefined ? line : this.line,
      character !== undefined ? character : this.character
    );
  }
}

class Range {
  constructor(startLineOrStart, startCharOrEnd, endLine, endChar) {
    if (startLineOrStart instanceof Position && startCharOrEnd instanceof Position) {
      this.start = startLineOrStart;
      this.end = startCharOrEnd;
    } else {
      this.start = new Position(startLineOrStart, startCharOrEnd);
      this.end = new Position(endLine, endChar);
    }
  }

  get isEmpty() {
    return this.start.isEqual(this.end);
  }

  get isSingleLine() {
    return this.start.line === this.end.line;
  }

  contains(positionOrRange) {
    if (positionOrRange instanceof Range) {
      return this.contains(positionOrRange.start) && this.contains(positionOrRange.end);
    }
    return positionOrRange.isAfterOrEqual(this.start) && positionOrRange.isBeforeOrEqual(this.end);
  }

  isEqual(other) {
    return this.start.isEqual(other.start) && this.end.isEqual(other.end);
  }

  with(start, end) {
    return new Range(
      start !== undefined ? start : this.start,
      end !== undefined ? end : this.end
    );
  }
}

class Selection extends Range {
  constructor(anchorLineOrAnchor, anchorCharOrActive, activeLine, activeChar) {
    super(anchorLineOrAnchor, anchorCharOrActive, activeLine, activeChar);
    if (anchorLineOrAnchor instanceof Position && anchorCharOrActive instanceof Position) {
      this.anchor = anchorLineOrAnchor;
      this.active = anchorCharOrActive;
    } else {
      this.anchor = new Position(anchorLineOrAnchor, anchorCharOrActive);
      this.active = new Position(activeLine, activeChar);
    }
  }

  get isReversed() {
    return this.anchor.isAfter(this.active);
  }
}

function isAsciiAlpha(code) {
  return (code >= 65 && code <= 90) || (code >= 97 && code <= 122);
}

function hasWindowsDrivePrefix(str, offset) {
  return (
    str.length >= offset + 2 &&
    isAsciiAlpha(str.charCodeAt(offset)) &&
    str.charCodeAt(offset + 1) === 58 &&
    (str.length === offset + 2 || str.charCodeAt(offset + 2) === 47)
  );
}

class Uri {
  constructor(scheme, authority, path, query, fragment) {
    this.scheme = scheme || 'file';
    this.authority = authority || '';
    this.path = path || '';
    this.query = query || '';
    this.fragment = fragment || '';
  }

  get fsPath() {
    if (this.scheme === 'file') {
      if (
        this.authority &&
        this.authority.toLowerCase() !== 'localhost' &&
        this.path.length > 1
      ) {
        const winTail = this.path.split('/').join('\\');
        return `\\\\${this.authority}${winTail}`;
      }
      if (this.path.startsWith('/') && hasWindowsDrivePrefix(this.path, 1)) {
        const winPath = this.path.slice(1).split('/').join('\\');
        return winPath.length === 2 ? `${winPath}\\` : winPath;
      }
    }
    return this.path;
  }

  static file(path) {
    const raw = typeof path === 'string' ? path : String(path || '');
    const normalized = raw.split('\\').join('/');
    if (normalized.startsWith('//')) {
      const idx = normalized.indexOf('/', 2);
      if (idx === -1) {
        return new Uri('file', normalized.slice(2), '', '', '');
      }
      return new Uri('file', normalized.slice(2, idx), normalized.slice(idx), '', '');
    }
    if (hasWindowsDrivePrefix(normalized, 0)) {
      const withSlash = normalized.length === 2 ? `/${normalized}/` : `/${normalized}`;
      return new Uri('file', '', withSlash, '', '');
    }
    return new Uri('file', '', normalized, '', '');
  }

  static parse(value) {
    try {
      const u = new URL(value);
      return new Uri(u.protocol.replace(':', ''), u.host, u.pathname, u.search, u.hash);
    } catch {
      return new Uri('file', '', value, '', '');
    }
  }

  toString() {
    return `${this.scheme}://${this.authority}${this.path}`;
  }

  toJSON() {
    return {
      $mid: 1,
      scheme: this.scheme,
      authority: this.authority,
      path: this.path,
      query: this.query,
      fragment: this.fragment,
    };
  }
}

class Disposable {
  constructor(callOnDispose) {
    this.callOnDispose = callOnDispose;
  }

  dispose() {
    if (typeof this.callOnDispose === 'function') {
      this.callOnDispose();
      this.callOnDispose = null;
    }
  }

  static from(...disposables) {
    return new Disposable(() => {
      for (const d of disposables) {
        if (d && typeof d.dispose === 'function') {
          d.dispose();
        }
      }
    });
  }
}

class CancellationTokenSource {
  constructor() {
    this.token = new CancellationToken();
  }

  cancel() {
    this.token._cancel();
  }

  dispose() {
    this.cancel();
  }
}

class CancellationToken {
  constructor() {
    this.isCancellationRequested = false;
    this._listeners = [];
  }

  _cancel() {
    if (!this.isCancellationRequested) {
      this.isCancellationRequested = true;
      for (const fn of this._listeners) {
        fn();
      }
    }
  }

  onCancellationRequested(listener) {
    if (this.isCancellationRequested) {
      listener();
      return new Disposable(() => {});
    }
    this._listeners.push(listener);
    return new Disposable(() => {
      const idx = this._listeners.indexOf(listener);
      if (idx !== -1) this._listeners.splice(idx, 1);
    });
  }
}

const DiagnosticSeverity = {
  Error: 0,
  Warning: 1,
  Information: 2,
  Hint: 3,
};

class Diagnostic {
  constructor(range, message, severity = DiagnosticSeverity.Error) {
    this.range = range;
    this.message = message;
    this.severity = severity;
    this.source = '';
    this.code = undefined;
  }
}

const CompletionItemKind = {
  Text: 0,
  Method: 1,
  Function: 2,
  Constructor: 3,
  Field: 4,
  Variable: 5,
  Class: 6,
  Interface: 7,
  Module: 8,
  Property: 9,
  Unit: 10,
  Value: 11,
  Enum: 12,
  Keyword: 13,
  Snippet: 14,
  Color: 15,
  File: 16,
  Reference: 17,
  Folder: 18,
  EnumMember: 19,
  Constant: 20,
  Struct: 21,
  Event: 22,
  Operator: 23,
  TypeParameter: 24,
};

class CompletionItem {
  constructor(label, kind = CompletionItemKind.Text) {
    this.label = label;
    this.kind = kind;
    this.detail = '';
    this.documentation = '';
    this.insertText = undefined;
  }
}

class Hover {
  constructor(contents, range) {
    this.contents = Array.isArray(contents) ? contents : [contents];
    this.range = range;
  }
}

const StatusBarAlignment = {
  Left: 1,
  Right: 2,
};

class ThemeColor {
  constructor(id) {
    this.id = id;
  }
}

class ThemeIcon {
  constructor(id) {
    this.id = id;
  }
}

class SnippetString {
  constructor(value = '') {
    this.value = value;
  }

  appendText(string) {
    this.value += string;
    return this;
  }

  appendTabstop(number = 0) {
    this.value += `$${number}`;
    return this;
  }
}

class TextEdit {
  constructor(range, newText) {
    this.range = range;
    this.newText = newText;
  }

  static replace(range, newText) {
    return new TextEdit(range, newText);
  }

  static insert(position, newText) {
    return new TextEdit(new Range(position, position), newText);
  }

  static delete(range) {
    return new TextEdit(range, '');
  }
}

class WorkspaceEdit {
  constructor() {
    this._changes = new Map();
  }

  set(uri, edits) {
    const key = uri.toString();
    this._changes.set(key, (edits || []).slice());
  }

  get(uri) {
    const key = uri.toString();
    return this._changes.get(key) || [];
  }

  has(uri) {
    const key = uri.toString();
    return this._changes.has(key);
  }

  replace(uri, range, newText) {
    const edit = TextEdit.replace(range, newText);
    const edits = this.get(uri);
    edits.push(edit);
    this.set(uri, edits);
  }

  insert(uri, position, newText) {
    const edit = TextEdit.insert(position, newText);
    const edits = this.get(uri);
    edits.push(edit);
    this.set(uri, edits);
  }

  delete(uri, range) {
    const edit = TextEdit.delete(range);
    const edits = this.get(uri);
    edits.push(edit);
    this.set(uri, edits);
  }

  entries() {
    return Array.from(this._changes.entries()).map(([k, edits]) => [Uri.parse(k), edits]);
  }

  get size() {
    let count = 0;
    for (const edits of this._changes.values()) {
      count += edits.length;
    }
    return count;
  }

  toJSON() {
    const changes = {};
    for (const [uri, edits] of this._changes.entries()) {
      changes[uri] = edits;
    }
    return { changes };
  }
}

const CodeActionKind = {
  Empty: '',
  QuickFix: 'quickfix',
  Refactor: 'refactor',
  RefactorExtract: 'refactor.extract',
  RefactorInline: 'refactor.inline',
  RefactorRewrite: 'refactor.rewrite',
  Source: 'source',
  SourceOrganizeImports: 'source.organizeImports',
  SourceFixAll: 'source.fixAll',
};

const CodeActionTriggerKind = {
  Invoke: 1,
  Automatic: 2,
};

class CodeAction {
  constructor(title, kind) {
    this.title = title;
    this.kind = kind;
    this.diagnostics = [];
    this.isPreferred = false;
    this.edit = undefined;
    this.command = undefined;
  }
}

class EventEmitter {
  constructor(options) {
    this._options = options || {};
    this._listeners = [];
    this._disposed = false;
    this._event = null;
  }

  get event() {
    if (!this._event) {
      this._event = (listener, thisArgs, disposables) => {
        if (this._disposed) {
          return new Disposable(() => {});
        }
        if (this._listeners.length === 0 && typeof this._options.onWillAddFirstListener === 'function') {
          this._options.onWillAddFirstListener(this);
        }
        const fn =
          thisArgs !== undefined && thisArgs !== null ? listener.bind(thisArgs) : listener;
        const entry = { listener, fn, removed: false };
        this._listeners.push(entry);
        if (this._listeners.length === 1 && typeof this._options.onDidAddFirstListener === 'function') {
          this._options.onDidAddFirstListener(this);
        }

        const disposable = new Disposable(() => {
          if (entry.removed) return;
          entry.removed = true;
          const idx = this._listeners.indexOf(entry);
          if (idx !== -1) {
            this._listeners.splice(idx, 1);
            if (this._listeners.length === 0 && typeof this._options.onDidRemoveLastListener === 'function') {
              this._options.onDidRemoveLastListener(this);
            }
          }
        });

        if (Array.isArray(disposables)) {
          disposables.push(disposable);
        }
        return disposable;
      };
    }
    return this._event;
  }

  fire(data) {
    if (this._disposed) {
      return;
    }
    const snapshot = this._listeners.slice();
    for (const entry of snapshot) {
      if (this._disposed || entry.removed) {
        continue;
      }
      try {
        entry.fn(data);
      } catch (err) {
        console.error('[RustCode ExtHost] Error in EventEmitter listener:', err);
      }
    }
  }

  dispose() {
    if (this._disposed) {
      return;
    }
    this._disposed = true;
    for (const entry of this._listeners) {
      entry.removed = true;
    }
    this._listeners.length = 0;
    this._event = null;
  }
}

const TreeItemCollapsibleState = {
  None: 0,
  Collapsed: 1,
  Expanded: 2,
};

class TreeItem {
  constructor(labelOrResourceUri, collapsibleState = TreeItemCollapsibleState.None) {
    if (typeof labelOrResourceUri === 'string') {
      this.label = labelOrResourceUri;
    } else if (labelOrResourceUri && typeof labelOrResourceUri === 'object') {
      if (typeof labelOrResourceUri.label === 'string') {
        this.label = labelOrResourceUri.label;
      } else if (typeof labelOrResourceUri.path === 'string' || typeof labelOrResourceUri.fsPath === 'string') {
        this.resourceUri = labelOrResourceUri;
        const rawPath = labelOrResourceUri.fsPath || labelOrResourceUri.path || '';
        const trimmedPath = rawPath.replace(/[\\/]+$/, '');
        const parts = trimmedPath.split(/[\\/]/);
        this.label = parts[parts.length - 1] || rawPath;
      } else {
        this.label = String(labelOrResourceUri);
      }
    } else {
      this.label = '';
    }
    this.id = undefined;
    this.description = undefined;
    this.tooltip = undefined;
    this.collapsibleState = collapsibleState;
    this.iconPath = undefined;
    this.contextValue = undefined;
    this.command = undefined;
  }
}

const ConfigurationTarget = {
  Global: 1,
  Workspace: 2,
  WorkspaceFolder: 3,
};

const EndOfLine = {
  LF: 1,
  CRLF: 2,
};

module.exports = {
  Position,
  Range,
  Selection,
  Uri,
  Disposable,
  CancellationTokenSource,
  CancellationToken,
  EventEmitter,
  DiagnosticSeverity,
  Diagnostic,
  CompletionItemKind,
  CompletionItem,
  Hover,
  StatusBarAlignment,
  ThemeColor,
  ThemeIcon,
  SnippetString,
  TextEdit,
  WorkspaceEdit,
  CodeActionKind,
  CodeActionTriggerKind,
  CodeAction,
  TreeItemCollapsibleState,
  TreeItem,
  ConfigurationTarget,
  EndOfLine,
};
