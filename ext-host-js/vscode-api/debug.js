// vscode.debug API implementation

const rpc = require('../rpc-bridge');
const { Disposable, Uri } = require('./types');

const DebugConsoleMode = {
  Separate: 0,
  MergeWithParent: 1,
};

const DebugConfigurationProviderTriggerKind = {
  Initial: 1,
  Dynamic: 2,
};

let nextBreakpointId = 1;
let nextDebugConfigId = 1;
let nextDebugAdapterId = 1;

function _uriToString(uri) {
  if (!uri) return undefined;
  return typeof uri.toString === 'function' ? uri.toString() : String(uri);
}

class Breakpoint {
  constructor(enabled = true, condition, hitCondition, logMessage) {
    this.id = `bp_${nextBreakpointId++}`;
    this.enabled = enabled !== false;
    this.condition = condition;
    this.hitCondition = hitCondition;
    this.logMessage = logMessage;
  }
}

class SourceBreakpoint extends Breakpoint {
  constructor(location, enabled = true, condition, hitCondition, logMessage) {
    super(enabled, condition, hitCondition, logMessage);
    this.location = location;
  }

  toJSON() {
    let loc = null;
    if (this.location) {
      const uri = this.location.uri ? _uriToString(this.location.uri) : undefined;
      const range = this.location.range
        ? {
            start: {
              line: this.location.range.start ? this.location.range.start.line : 0,
              character: this.location.range.start ? this.location.range.start.character : 0,
            },
            end: {
              line: this.location.range.end ? this.location.range.end.line : 0,
              character: this.location.range.end ? this.location.range.end.character : 0,
            },
          }
        : undefined;
      loc = { uri, range };
    }
    return {
      id: this.id,
      kind: 'source',
      enabled: this.enabled,
      condition: this.condition,
      hitCondition: this.hitCondition,
      logMessage: this.logMessage,
      location: loc,
    };
  }
}

class FunctionBreakpoint extends Breakpoint {
  constructor(functionName, enabled = true, condition, hitCondition, logMessage) {
    super(enabled, condition, hitCondition, logMessage);
    this.functionName = functionName;
  }

  toJSON() {
    return {
      id: this.id,
      kind: 'function',
      enabled: this.enabled,
      condition: this.condition,
      hitCondition: this.hitCondition,
      logMessage: this.logMessage,
      functionName: this.functionName,
    };
  }
}

class DebugAdapterExecutable {
  constructor(command, args = [], options) {
    this.type = 'executable';
    this.command = command;
    this.args = Array.isArray(args) ? args : [];
    this.options = options;
  }
}

class DebugAdapterServer {
  constructor(port, host) {
    this.type = 'server';
    this.port = port;
    this.host = host;
  }
}

class DebugAdapterNamedPipeServer {
  constructor(path) {
    this.type = 'namedPipeServer';
    this.path = path;
  }
}

class DebugAdapterInlineImplementation {
  constructor(implementation) {
    this.type = 'inline';
    this.implementation = implementation;
  }

  toJSON() {
    return { type: this.type };
  }
}

const trackedBreakpoints = [];
let activeSession = undefined;

const activeDebugConsole = {
  append(value) {
    rpc.fireAndForget('debug.appendConsole', {
      value: value !== undefined && value !== null ? String(value) : '',
      newLine: false,
    });
  },
  appendLine(value) {
    rpc.fireAndForget('debug.appendConsole', {
      value: value !== undefined && value !== null ? String(value) : '',
      newLine: true,
    });
  },
};

const debug = {
  get activeDebugSession() {
    return activeSession;
  },

  get activeDebugConsole() {
    return activeDebugConsole;
  },

  get breakpoints() {
    return trackedBreakpoints.slice();
  },

  async startDebugging(folder, nameOrConfiguration, parentSessionOrOptions) {
    const res = await rpc.call('debug.startDebugging', {
      folder: folder && folder.uri ? { name: folder.name, uri: _uriToString(folder.uri) } : folder,
      nameOrConfiguration,
      options: parentSessionOrOptions,
    });
    if (res && res.session) {
      activeSession = res.session;
    }
    return res !== null ? Boolean(res.started !== false) : true;
  },

  async stopDebugging(session) {
    const sessionId = session && session.id ? session.id : activeSession && activeSession.id;
    const res = await rpc.call('debug.stopDebugging', {
      sessionId: sessionId || null,
    });
    if (!session || (activeSession && activeSession.id === session.id)) {
      activeSession = undefined;
    }
    return res;
  },

  registerDebugConfigurationProvider(
    debugType,
    provider,
    triggerKind = DebugConfigurationProviderTriggerKind.Initial
  ) {
    const id = `debug_cfg_${nextDebugConfigId++}`;
    const proxyDisp = rpc.registerObjectProxy(
      'DebugConfiguration',
      id,
      provider,
      [
        'provideDebugConfigurations',
        'resolveDebugConfiguration',
        'resolveDebugConfigurationWithSubstitutedVariables',
      ],
      { selector: debugType, debugType, triggerKind }
    );
    rpc.fireAndForget('debug.registerConfigurationProvider', {
      id,
      debugType,
      triggerKind,
    });
    return new Disposable(() => {
      proxyDisp.dispose();
      rpc.fireAndForget('debug.unregisterConfigurationProvider', { id, debugType });
    });
  },

  registerDebugAdapterDescriptorFactory(debugType, factory) {
    const id = `debug_adapter_${nextDebugAdapterId++}`;
    const proxyDisp = rpc.registerObjectProxy(
      'DebugAdapterFactory',
      id,
      factory,
      ['createDebugAdapterDescriptor'],
      { selector: debugType, debugType }
    );
    rpc.fireAndForget('debug.registerAdapterDescriptorFactory', {
      id,
      debugType,
    });
    return new Disposable(() => {
      proxyDisp.dispose();
      rpc.fireAndForget('debug.unregisterAdapterDescriptorFactory', { id, debugType });
    });
  },

  registerDebugAdapterTrackerFactory(debugType, factory) {
    const id = `debug_tracker_${nextDebugAdapterId++}`;
    rpc.fireAndForget('debug.registerAdapterTrackerFactory', {
      id,
      debugType,
      hasFactory: Boolean(factory),
    });
    return new Disposable(() => {
      rpc.fireAndForget('debug.unregisterAdapterTrackerFactory', { id, debugType });
    });
  },

  addBreakpoints(breakpoints) {
    const added = [];
    for (const bp of breakpoints || []) {
      if (!bp) continue;
      if (!trackedBreakpoints.some((existing) => existing.id === bp.id)) {
        trackedBreakpoints.push(bp);
        added.push(bp);
      }
    }
    if (added.length > 0) {
      rpc.fireAndForget('debug.addBreakpoints', { breakpoints: added });
    }
  },

  removeBreakpoints(breakpoints) {
    const ids = [];
    for (const bp of breakpoints || []) {
      if (!bp || !bp.id) continue;
      const idx = trackedBreakpoints.findIndex((existing) => existing.id === bp.id);
      if (idx !== -1) {
        trackedBreakpoints.splice(idx, 1);
      }
      ids.push(bp.id);
    }
    if (ids.length > 0) {
      rpc.fireAndForget('debug.removeBreakpoints', { ids });
    }
  },

  asDebugSourceUri(source, session) {
    if (source && source.path) {
      return Uri.file(source.path);
    }
    const ref = source && source.sourceReference ? source.sourceReference : 0;
    const sessionId = session && session.id ? session.id : 'default';
    return Uri.parse(`debug:///${sessionId}/${ref}`);
  },

  onDidStartDebugSession: rpc.createEvent('debug.onDidStartDebugSession'),
  onDidTerminateDebugSession: rpc.createEvent('debug.onDidTerminateDebugSession'),
  onDidChangeActiveDebugSession: rpc.createEvent('debug.onDidChangeActiveDebugSession'),
  onDidReceiveDebugSessionCustomEvent: rpc.createEvent('debug.onDidReceiveDebugSessionCustomEvent'),
  onDidChangeBreakpoints: rpc.createEvent('debug.onDidChangeBreakpoints'),
};

module.exports = {
  debug,
  DebugConsoleMode,
  DebugConfigurationProviderTriggerKind,
  Breakpoint,
  SourceBreakpoint,
  FunctionBreakpoint,
  DebugAdapterExecutable,
  DebugAdapterServer,
  DebugAdapterNamedPipeServer,
  DebugAdapterInlineImplementation,
};
