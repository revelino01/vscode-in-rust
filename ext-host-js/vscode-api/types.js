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

function safeDecodeUriPath(p) {
  if (typeof p !== 'string' || p.indexOf('%') === -1) {
    return p;
  }
  return p.replace(/%(?:[0-9A-Fa-f]{2})+/g, (seq) => {
    try {
      return decodeURIComponent(seq);
    } catch {
      return seq;
    }
  });
}

function encodeUriPath(p) {
  if (typeof p !== 'string' || !p) {
    return '';
  }
  try {
    return encodeURI(p).replace(/#/g, '%23').replace(/\?/g, '%3F');
  } catch {
    return p;
  }
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
    const decodedPath = safeDecodeUriPath(this.path);
    if (this.scheme === 'file') {
      if (
        this.authority &&
        this.authority.toLowerCase() !== 'localhost' &&
        decodedPath.length > 1
      ) {
        const winTail = decodedPath.split('/').join('\\');
        return `\\\\${this.authority}${winTail}`;
      }
      if (decodedPath.startsWith('/') && hasWindowsDrivePrefix(decodedPath, 1)) {
        const winPath = decodedPath.slice(1).split('/').join('\\');
        return winPath.length === 2 ? `${winPath}\\` : winPath;
      }
    }
    return decodedPath;
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
    const str = typeof value === 'string' ? value : String(value || '');
    const slashNorm = str.split('\\').join('/');
    if (hasWindowsDrivePrefix(slashNorm, 0)) {
      return Uri.file(str);
    }
    try {
      const u = new URL(str);
      const query = u.search ? u.search.slice(1) : '';
      const fragment = u.hash ? u.hash.slice(1) : '';
      return new Uri(
        u.protocol.replace(':', ''),
        u.host,
        safeDecodeUriPath(u.pathname),
        query,
        fragment
      );
    } catch {
      return new Uri('file', '', str, '', '');
    }
  }

  static from(components = {}) {
    return new Uri(
      components.scheme || 'file',
      components.authority || '',
      components.path || '',
      components.query || '',
      components.fragment || ''
    );
  }

  toString(skipEncoding = false) {
    const pathPart = skipEncoding ? this.path : encodeUriPath(this.path);
    const hasAuthority = Boolean(this.authority && this.authority.length > 0);
    const useDoubleSlash =
      this.scheme === 'file' || hasAuthority || pathPart.startsWith('//');
    let out = useDoubleSlash
      ? `${this.scheme}://${this.authority}${
          hasAuthority && pathPart && !pathPart.startsWith('/') ? `/${pathPart}` : pathPart
        }`
      : `${this.scheme}:${pathPart}`;
    if (this.query) {
      out += this.query.startsWith('?') ? this.query : `?${this.query}`;
    }
    if (this.fragment) {
      out += this.fragment.startsWith('#') ? this.fragment : `#${this.fragment}`;
    }
    return out;
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
    } else if (labelOrResourceUri === undefined || labelOrResourceUri === null) {
      this.label = undefined;
    } else {
      this.label = String(labelOrResourceUri);
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

const UIKind = {
  Desktop: 1,
  Web: 2,
};

const ViewColumn = {
  Active: -1,
  Beside: -2,
  One: 1,
  Two: 2,
  Three: 3,
  Four: 4,
  Five: 5,
  Six: 6,
  Seven: 7,
  Eight: 8,
  Nine: 9,
};

const TextEditorRevealType = {
  Default: 0,
  InCenter: 1,
  InCenterIfOutsideViewport: 2,
  AtTop: 3,
};

const TextEditorSelectionChangeKind = {
  Keyboard: 1,
  Mouse: 2,
  Command: 3,
};

const FileType = {
  Unknown: 0,
  File: 1,
  Directory: 2,
  SymbolicLink: 64,
};

class FileSystemError extends Error {
  constructor(messageOrUri, code = 'Unknown') {
    const msg =
      messageOrUri && typeof messageOrUri === 'object' && typeof messageOrUri.toString === 'function'
        ? messageOrUri.toString()
        : String(messageOrUri || code);
    super(msg);
    this.name = 'FileSystemError';
    this.code = code;
  }

  static FileNotFound(messageOrUri) {
    return new FileSystemError(messageOrUri, 'FileNotFound');
  }

  static FileExists(messageOrUri) {
    return new FileSystemError(messageOrUri, 'FileExists');
  }

  static FileNotADirectory(messageOrUri) {
    return new FileSystemError(messageOrUri, 'FileNotADirectory');
  }

  static FileIsADirectory(messageOrUri) {
    return new FileSystemError(messageOrUri, 'FileIsADirectory');
  }

  static NoPermissions(messageOrUri) {
    return new FileSystemError(messageOrUri, 'NoPermissions');
  }

  static Unavailable(messageOrUri) {
    return new FileSystemError(messageOrUri, 'Unavailable');
  }
}

class RelativePattern {
  constructor(base, pattern) {
    if (base && typeof base === 'object' && base.uri) {
      this.baseUri = base.uri;
      this.base = base.uri.fsPath || base.uri.path || String(base.uri);
    } else if (base && typeof base === 'object' && (base.fsPath || base.path)) {
      this.baseUri = base;
      this.base = base.fsPath || base.path || String(base);
    } else {
      this.base = String(base || '');
      this.baseUri = Uri.file(this.base);
    }
    this.pattern = String(pattern || '');
  }

  toJSON() {
    return {
      base: this.base,
      baseUri: this.baseUri,
      pattern: this.pattern,
    };
  }
}

class MarkdownString {
  constructor(value = '', supportThemeIcons = false) {
    this.value = String(value || '');
    this.isTrusted = false;
    this.supportThemeIcons = Boolean(supportThemeIcons);
    this.supportHtml = false;
  }

  appendText(value) {
    this.value += String(value || '');
    return this;
  }

  appendMarkdown(value) {
    this.value += String(value || '');
    return this;
  }

  appendCodeblock(value, language = '') {
    this.value += `\n\`\`\`${language || ''}\n${String(value || '')}\n\`\`\`\n`;
    return this;
  }
}

class Location {
  constructor(uri, rangeOrPosition) {
    this.uri = uri instanceof Uri ? uri.toString() : String(uri || '');
    if (rangeOrPosition instanceof Position) {
      this.range = new Range(rangeOrPosition, rangeOrPosition);
    } else {
      this.range = rangeOrPosition;
    }
  }
}

class ParameterInformation {
  constructor(label, documentation) {
    this.label = label;
    this.documentation = documentation;
  }
}

class SignatureInformation {
  constructor(label, documentation) {
    this.label = label;
    this.documentation = documentation;
    this.parameters = [];
    this.activeParameter = undefined;
  }
}

class SignatureHelp {
  constructor() {
    this.signatures = [];
    this.activeSignature = 0;
    this.activeParameter = 0;
  }
}

class SemanticTokensLegend {
  constructor(tokenTypes = [], tokenModifiers = []) {
    this.tokenTypes = Array.isArray(tokenTypes) ? tokenTypes.slice() : [];
    this.tokenModifiers = Array.isArray(tokenModifiers) ? tokenModifiers.slice() : [];
  }
}

class SemanticTokens {
  constructor(data, resultId) {
    this.data = data instanceof Uint32Array ? Array.from(data) : Array.isArray(data) ? data.slice() : [];
    this.resultId = resultId;
  }
}

class SemanticTokensBuilder {
  constructor(legend) {
    this._legend = legend || new SemanticTokensLegend();
    this._data = [];
    this._prevLine = 0;
    this._prevChar = 0;
  }

  push(lineOrRange, charOrTokenType, lengthOrModifiers, tokenTypeOrUndefined, tokenModifiers) {
    let line;
    let char;
    let length;
    let tokenTypeIdx = 0;
    let modBits = 0;

    if (typeof lineOrRange === 'number') {
      line = lineOrRange;
      char = charOrTokenType;
      length = lengthOrModifiers;
      tokenTypeIdx = typeof tokenTypeOrUndefined === 'number'
        ? tokenTypeOrUndefined
        : Math.max(0, this._legend.tokenTypes.indexOf(tokenTypeOrUndefined));
      if (Array.isArray(tokenModifiers)) {
        for (const m of tokenModifiers) {
          const idx = this._legend.tokenModifiers.indexOf(m);
          if (idx >= 0 && idx < 32) modBits |= (1 << idx);
        }
      } else if (typeof tokenModifiers === 'number') {
        modBits = tokenModifiers;
      }
    } else if (lineOrRange && lineOrRange.start && lineOrRange.end) {
      line = lineOrRange.start.line;
      char = lineOrRange.start.character;
      length = Math.max(0, lineOrRange.end.character - lineOrRange.start.character);
      tokenTypeIdx = Math.max(0, this._legend.tokenTypes.indexOf(charOrTokenType));
      if (Array.isArray(lengthOrModifiers)) {
        for (const m of lengthOrModifiers) {
          const idx = this._legend.tokenModifiers.indexOf(m);
          if (idx >= 0 && idx < 32) modBits |= (1 << idx);
        }
      }
    } else {
      return;
    }

    const deltaLine = line - this._prevLine;
    const deltaChar = deltaLine === 0 ? (char - this._prevChar) : char;
    this._data.push(deltaLine, deltaChar, length, tokenTypeIdx, modBits);
    this._prevLine = line;
    this._prevChar = char;
  }

  build(resultId) {
    return new SemanticTokens(this._data, resultId);
  }
}

const InlayHintKind = {
  Type: 1,
  Parameter: 2,
};

class InlayHintLabelPart {
  constructor(value) {
    this.value = String(value || '');
    this.tooltip = undefined;
    this.location = undefined;
    this.command = undefined;
  }
}

class InlayHint {
  constructor(position, label, kind) {
    this.position = position;
    this.label = label;
    this.kind = kind;
    this.paddingLeft = false;
    this.paddingRight = false;
    this.tooltip = undefined;
  }
}

class CodeLens {
  constructor(range, command) {
    this.range = range;
    this.command = command;
  }

  get isResolved() {
    return Boolean(this.command);
  }
}

class DocumentLink {
  constructor(range, target) {
    this.range = range;
    this.target = target instanceof Uri ? target.toString() : target;
    this.tooltip = undefined;
  }
}

const SymbolKind = {
  File: 0,
  Module: 1,
  Namespace: 2,
  Package: 3,
  Class: 4,
  Method: 5,
  Property: 6,
  Field: 7,
  Constructor: 8,
  Enum: 9,
  Interface: 10,
  Function: 11,
  Variable: 12,
  Constant: 13,
  String: 14,
  Number: 15,
  Boolean: 16,
  Array: 17,
  Object: 18,
  Key: 19,
  Null: 20,
  EnumMember: 21,
  Struct: 22,
  Event: 23,
  Operator: 24,
  TypeParameter: 25,
};

class CallHierarchyItem {
  constructor(kind, name, detail, uri, range, selectionRange) {
    this.kind = kind;
    this.name = name;
    this.detail = detail;
    this.uri = uri instanceof Uri ? uri.toString() : String(uri || '');
    this.range = range;
    this.selectionRange = selectionRange;
  }
}

class CallHierarchyIncomingCall {
  constructor(from, fromRanges) {
    this.from = from;
    this.fromRanges = fromRanges || [];
  }
}

class CallHierarchyOutgoingCall {
  constructor(to, fromRanges) {
    this.to = to;
    this.fromRanges = fromRanges || [];
  }
}

class TypeHierarchyItem {
  constructor(kind, name, detail, uri, range, selectionRange) {
    this.kind = kind;
    this.name = name;
    this.detail = detail;
    this.uri = uri instanceof Uri ? uri.toString() : String(uri || '');
    this.range = range;
    this.selectionRange = selectionRange;
  }
}

const LogLevel = {
  Off: 0,
  Trace: 1,
  Debug: 2,
  Info: 3,
  Warning: 4,
  Error: 5,
};

const FileChangeType = {
  Changed: 1,
  Created: 2,
  Deleted: 3,
};

const TreeItemCheckboxState = {
  Unchecked: 0,
  Checked: 1,
};

class DataTransferItem {
  constructor(value) {
    this.value = value;
  }

  asString() {
    if (typeof this.value === 'string') {
      return Promise.resolve(this.value);
    }
    if (this.value instanceof Uint8Array || (typeof Buffer !== 'undefined' && Buffer.isBuffer(this.value))) {
      return Promise.resolve(Buffer.from(this.value).toString('utf8'));
    }
    return Promise.resolve(JSON.stringify(this.value ?? ''));
  }

  asFile() {
    return this.value && typeof this.value === 'object' && this.value.name ? this.value : undefined;
  }
}

class DataTransfer {
  constructor(initialEntries = []) {
    this._map = new Map();
    if (Array.isArray(initialEntries)) {
      for (const [mime, val] of initialEntries) {
        this.set(mime, val instanceof DataTransferItem ? val : new DataTransferItem(val));
      }
    } else if (initialEntries && typeof initialEntries === 'object') {
      for (const [mime, val] of Object.entries(initialEntries)) {
        this.set(mime, val instanceof DataTransferItem ? val : new DataTransferItem(val));
      }
    }
  }

  get(mimeType) {
    return this._map.get(mimeType);
  }

  set(mimeType, value) {
    this._map.set(mimeType, value instanceof DataTransferItem ? value : new DataTransferItem(value));
  }

  forEach(cb, thisArg) {
    for (const [mime, item] of this._map.entries()) {
      cb.call(thisArg, item, mime, this);
    }
  }

  [Symbol.iterator]() {
    return this._map.entries();
  }

  toJSON() {
    const out = {};
    for (const [mime, item] of this._map.entries()) {
      out[mime] = item && item.value !== undefined ? item.value : null;
    }
    return out;
  }
}

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
  UIKind,
  ViewColumn,
  TextEditorRevealType,
  TextEditorSelectionChangeKind,
  FileType,
  FileSystemError,
  RelativePattern,
  MarkdownString,
  Location,
  ParameterInformation,
  SignatureInformation,
  SignatureHelp,
  SemanticTokensLegend,
  SemanticTokens,
  SemanticTokensBuilder,
  InlayHintKind,
  InlayHintLabelPart,
  InlayHint,
  CodeLens,
  DocumentLink,
  SymbolKind,
  CallHierarchyItem,
  CallHierarchyIncomingCall,
  CallHierarchyOutgoingCall,
  TypeHierarchyItem,
  LogLevel,
  FileChangeType,
  TreeItemCheckboxState,
  DataTransferItem,
  DataTransfer,
};
