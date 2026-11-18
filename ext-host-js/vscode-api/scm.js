// vscode.scm API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

const SourceControlInputBoxValidationType = {
  Error: 0,
  Warning: 1,
  Information: 2,
};

function _uriToString(uri) {
  if (!uri) return undefined;
  return typeof uri.toString === 'function' ? uri.toString() : String(uri);
}

function _normalizeResourceState(state) {
  if (!state) return state;
  return {
    resourceUri: _uriToString(state.resourceUri),
    command: state.command || undefined,
    decorations: state.decorations || undefined,
    contextValue: state.contextValue || undefined,
  };
}

class SourceControlInputBox {
  constructor(sourceControlId) {
    this._sourceControlId = sourceControlId;
    this._value = '';
    this._placeholder = '';
    this._enabled = true;
    this._visible = true;
    this._validationMessage = undefined;
  }

  _sync() {
    if (!this._sourceControlId) return;
    rpc.call('scm.updateInputBox', {
      sourceControlId: this._sourceControlId,
      value: this._value,
      placeholder: this._placeholder,
      enabled: this._enabled,
      visible: this._visible,
      validationMessage: this._validationMessage,
    });
  }

  get value() {
    return this._value;
  }

  set value(val) {
    this._value = val !== undefined && val !== null ? String(val) : '';
    this._sync();
  }

  get placeholder() {
    return this._placeholder;
  }

  set placeholder(val) {
    this._placeholder = val !== undefined && val !== null ? String(val) : '';
    this._sync();
  }

  get enabled() {
    return this._enabled;
  }

  set enabled(val) {
    this._enabled = Boolean(val);
    this._sync();
  }

  get visible() {
    return this._visible;
  }

  set visible(val) {
    this._visible = Boolean(val);
    this._sync();
  }

  get validationMessage() {
    return this._validationMessage;
  }

  set validationMessage(val) {
    this._validationMessage = val;
    this._sync();
  }

  toJSON() {
    return {
      value: this._value,
      placeholder: this._placeholder,
      enabled: this._enabled,
      visible: this._visible,
      validationMessage: this._validationMessage,
    };
  }
}

class SourceControlResourceGroup {
  constructor(sourceControlId, id, label, onDispose) {
    this._sourceControlId = sourceControlId;
    this.id = id;
    this._label = label;
    this._hideWhenEmpty = false;
    this._resourceStates = [];
    this._disposed = false;
    this._onDispose = onDispose || null;
  }

  _sync() {
    if (this._disposed) return;
    rpc.call('scm.updateResourceGroup', {
      sourceControlId: this._sourceControlId,
      id: this.id,
      label: this._label,
      hideWhenEmpty: this._hideWhenEmpty,
      resourceStates: this._resourceStates.map(_normalizeResourceState),
    });
  }

  get label() {
    return this._label;
  }

  set label(val) {
    this._label = val !== undefined && val !== null ? String(val) : '';
    this._sync();
  }

  get hideWhenEmpty() {
    return this._hideWhenEmpty;
  }

  set hideWhenEmpty(val) {
    this._hideWhenEmpty = Boolean(val);
    this._sync();
  }

  get resourceStates() {
    return this._resourceStates;
  }

  set resourceStates(states) {
    this._resourceStates = Array.isArray(states) ? states.slice() : [];
    this._sync();
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    if (typeof this._onDispose === 'function') {
      this._onDispose();
      this._onDispose = null;
    }
    rpc.call('scm.disposeResourceGroup', {
      sourceControlId: this._sourceControlId,
      id: this.id,
    });
  }

  toJSON() {
    return {
      id: this.id,
      label: this._label,
      hideWhenEmpty: this._hideWhenEmpty,
      resourceStates: this._resourceStates.map(_normalizeResourceState),
    };
  }
}

class SourceControl {
  constructor(id, label, rootUri) {
    this.id = id;
    this.label = label;
    this.rootUri = rootUri;
    this.inputBox = new SourceControlInputBox(id);
    this._count = undefined;
    this._quickDiffProvider = undefined;
    this._commitTemplate = undefined;
    this._acceptInputCommand = undefined;
    this._statusBarCommands = undefined;
    this._groups = new Map();
    this._disposed = false;
    this._onDispose = null;
  }

  _sync() {
    if (this._disposed) return;
    rpc.call('scm.updateSourceControl', {
      id: this.id,
      label: this.label,
      rootUri: _uriToString(this.rootUri),
      count: this._count,
      commitTemplate: this._commitTemplate,
      acceptInputCommand: this._acceptInputCommand,
      statusBarCommands: this._statusBarCommands,
      hasQuickDiffProvider: Boolean(this._quickDiffProvider),
    });
  }

  get count() {
    return this._count;
  }

  set count(val) {
    this._count = val;
    this._sync();
  }

  get quickDiffProvider() {
    return this._quickDiffProvider;
  }

  set quickDiffProvider(val) {
    this._quickDiffProvider = val;
    this._sync();
  }

  get commitTemplate() {
    return this._commitTemplate;
  }

  set commitTemplate(val) {
    this._commitTemplate = val;
    this._sync();
  }

  get acceptInputCommand() {
    return this._acceptInputCommand;
  }

  set acceptInputCommand(val) {
    this._acceptInputCommand = val;
    this._sync();
  }

  get statusBarCommands() {
    return this._statusBarCommands;
  }

  set statusBarCommands(val) {
    this._statusBarCommands = val;
    this._sync();
  }

  createResourceGroup(id, label) {
    if (this._disposed) {
      throw new Error(`SourceControl '${this.id}' is disposed`);
    }
    const group = new SourceControlResourceGroup(this.id, id, label, () => {
      this._groups.delete(id);
    });
    this._groups.set(id, group);
    rpc.call('scm.createResourceGroup', {
      sourceControlId: this.id,
      id,
      label,
      hideWhenEmpty: group.hideWhenEmpty,
    });
    return group;
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    for (const group of Array.from(this._groups.values())) {
      group.dispose();
    }
    this._groups.clear();
    if (typeof this._onDispose === 'function') {
      this._onDispose();
      this._onDispose = null;
    }
    rpc.call('scm.disposeSourceControl', { id: this.id });
  }

  toJSON() {
    return {
      id: this.id,
      label: this.label,
      rootUri: _uriToString(this.rootUri),
      count: this._count,
      commitTemplate: this._commitTemplate,
      hasQuickDiffProvider: Boolean(this._quickDiffProvider),
    };
  }
}

const activeSourceControls = [];
const defaultInputBox = new SourceControlInputBox('default');

const scm = {
  createSourceControl(id, label, rootUri) {
    const sc = new SourceControl(id, label, rootUri);
    const rootUriStr = _uriToString(rootUri);
    const proxyDisp = rpc.registerObjectProxy(
      'SourceControl',
      id,
      {
        provideOriginalResource: (uri, token) => {
          if (
            sc.quickDiffProvider &&
            typeof sc.quickDiffProvider.provideOriginalResource === 'function'
          ) {
            return sc.quickDiffProvider.provideOriginalResource(uri, token);
          }
          return undefined;
        },
        acceptInput: (value) => {
          if (value !== undefined && value !== null) {
            sc.inputBox._value = String(value);
          }
          return sc.acceptInputCommand || { accepted: true, value: sc.inputBox._value };
        },
      },
      ['provideOriginalResource', 'acceptInput'],
      { selector: id, id, label, rootUri: rootUriStr }
    );

    activeSourceControls.push(sc);
    rpc.call('scm.createSourceControl', {
      id,
      label,
      rootUri: rootUriStr,
    });

    sc._onDispose = () => {
      const idx = activeSourceControls.indexOf(sc);
      if (idx !== -1) {
        activeSourceControls.splice(idx, 1);
      }
      proxyDisp.dispose();
    };

    return sc;
  },

  get inputBox() {
    for (let i = activeSourceControls.length - 1; i >= 0; i--) {
      if (!activeSourceControls[i]._disposed) {
        return activeSourceControls[i].inputBox;
      }
    }
    return defaultInputBox;
  },

  onDidChangeActiveSourceControl: rpc.createEvent('scm.onDidChangeActiveSourceControl'),
};

module.exports = {
  scm,
  SourceControlInputBoxValidationType,
  SourceControlInputBox,
  SourceControlResourceGroup,
  SourceControl,
};
