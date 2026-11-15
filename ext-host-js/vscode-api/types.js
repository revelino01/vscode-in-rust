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

class Uri {
  constructor(scheme, authority, path, query, fragment) {
    this.scheme = scheme || 'file';
    this.authority = authority || '';
    this.path = path || '';
    this.query = query || '';
    this.fragment = fragment || '';
  }

  get fsPath() {
    return this.path;
  }

  static file(path) {
    return new Uri('file', '', path, '', '');
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

module.exports = {
  Position,
  Range,
  Selection,
  Uri,
  Disposable,
  CancellationTokenSource,
  CancellationToken,
  DiagnosticSeverity,
  Diagnostic,
  CompletionItemKind,
  CompletionItem,
  Hover,
  StatusBarAlignment,
  ThemeColor,
  ThemeIcon,
  SnippetString,
};
