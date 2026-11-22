// MessagePack / JSON RPC bridge between the Extension Host JavaScript runtime and RustCore.

const { Uri } = require('./vscode-api/types');

const globalDeno = typeof Deno !== 'undefined' ? Deno : null;
const ops = (globalDeno && globalDeno.core && globalDeno.core.ops) || {};
const nodeUtil = typeof require === 'function'
  ? (() => {
    try {
      return require('util');
    } catch (_) {
      return null;
    }
  })()
  : null;

const MAX_HEADER_BYTES = 64 * 1024;
const MAX_RPC_FRAME_BYTES = 64 * 1024 * 1024;
const BRIDGE_STATE_SYMBOL = Symbol.for('rustcode.rpcBridgeState');
const rootGlobal = typeof globalThis !== 'undefined'
  ? globalThis
  : (typeof global !== 'undefined' ? global : {});
const bridgeState = rootGlobal[BRIDGE_STATE_SYMBOL] || (rootGlobal[BRIDGE_STATE_SYMBOL] = {
  syncHandler: null,
  stdioTransport: null,
  nextId: 1,
  objectProxies: new Map(),
  eventSubscribers: new Map(),
  customDocuments: new Map(),
  treeElements: new Map(),
  treeElementReverse: new Map(),
  commandHandlers: new Map(),
  nextCommandToken: 1,
  inputValidators: new Map(),
  activatedExtensions: new Map(),
  initHooks: [],
  moduleHookInstalled: false,
});
if (!bridgeState.treeElementReverse) bridgeState.treeElementReverse = new Map();
if (!bridgeState.nextCommandToken) bridgeState.nextCommandToken = 1;
if (!bridgeState.inputValidators) bridgeState.inputValidators = new Map();
if (!bridgeState.activatedExtensions) bridgeState.activatedExtensions = new Map();
if (!bridgeState.initHooks) bridgeState.initHooks = [];

function createRpcError(errPayload) {
  if (!errPayload) {
    return new Error('RPC error');
  }
  if (typeof errPayload === 'string') {
    const sepIdx = errPayload.indexOf(': ');
    if (sepIdx > 0) {
      const prefix = errPayload.slice(0, sepIdx);
      let isCode = true;
      for (let i = 0; i < prefix.length; i++) {
        const ch = prefix.charCodeAt(i);
        const ok = (ch >= 65 && ch <= 90) || (ch >= 48 && ch <= 57) || ch === 95;
        if (!ok) {
          isCode = false;
          break;
        }
      }
      if (isCode) {
        const err = new Error(errPayload.slice(sepIdx + 2));
        err.code = prefix;
        return err;
      }
    }
    return new Error(errPayload);
  }
  const err = new Error(errPayload.message || 'RPC error');
  if (errPayload.code) {
    err.code = errPayload.code;
  }
  return err;
}

function encodeRpcFrame(msg, format) {
  const json = JSON.stringify(msg);
  if (format === 'newline-json') {
    return Buffer.from(json + '\n', 'utf8');
  }
  const body = Buffer.from(json, 'utf8');
  const header = Buffer.from('Content-Length: ' + body.length + '\r\n\r\n', 'ascii');
  return Buffer.concat([header, body]);
}

function parseContentLengthHeader(headerStr) {
  const lines = headerStr.split('\n');
  let contentLength = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) {
      continue;
    }
    const colonIdx = line.indexOf(':');
    if (colonIdx === -1) {
      continue;
    }
    const key = line.slice(0, colonIdx).trim().toLowerCase();
    if (key === 'content-length') {
      if (contentLength !== null) {
        throw new Error('Duplicate Content-Length header');
      }
      const valStr = line.slice(colonIdx + 1).trim();
      if (!valStr) {
        throw new Error('Invalid empty Content-Length header');
      }
      for (let j = 0; j < valStr.length; j++) {
        const ch = valStr.charCodeAt(j);
        if (ch < 48 || ch > 57) {
          throw new Error('Invalid Content-Length header: ' + valStr);
        }
      }
      const len = Number.parseInt(valStr, 10);
      if (!Number.isSafeInteger(len) || len < 0 || len > MAX_RPC_FRAME_BYTES) {
        throw new Error('Out-of-range Content-Length header: ' + valStr);
      }
      contentLength = len;
    }
  }
  if (contentLength === null) {
    throw new Error('Missing Content-Length header');
  }
  return contentLength;
}

function sendStdioFrame(msg) {
  if (!bridgeState.stdioTransport || !bridgeState.stdioTransport.active) {
    return false;
  }
  const buf = encodeRpcFrame(msg, bridgeState.stdioTransport.format);
  bridgeState.stdioTransport.rawWrite(buf);
  return true;
}

function drainStdioBuffer(transport, onMessage) {
  if (transport.format === 'newline-json') {
    while (true) {
      const nlIdx = transport.buffer.indexOf(0x0a);
      if (nlIdx === -1) {
        if (transport.buffer.length > MAX_RPC_FRAME_BYTES) {
          throw new Error('RPC line exceeds maximum size');
        }
        break;
      }
      const lineStr = transport.buffer.subarray(0, nlIdx).toString('utf8').trim();
      transport.buffer = transport.buffer.subarray(nlIdx + 1);
      if (!lineStr) {
        continue;
      }
      if (lineStr.length > MAX_RPC_FRAME_BYTES) {
        throw new Error('RPC line exceeds maximum size');
      }
      onMessage(JSON.parse(lineStr));
    }
    return;
  }

  while (true) {
    let lead = 0;
    while (lead < transport.buffer.length) {
      const b = transport.buffer[lead];
      if (b === 0x0d || b === 0x0a) {
        lead++;
      } else {
        break;
      }
    }
    if (lead > 0) {
      transport.buffer = transport.buffer.subarray(lead);
    }

    const searchEnd = Math.min(transport.buffer.length, MAX_HEADER_BYTES);
    const searchSlice = transport.buffer.subarray(0, searchEnd);
    const crlfIdx = searchSlice.indexOf('\r\n\r\n');
    const lfIdx = searchSlice.indexOf('\n\n');
    let headerEnd = -1;
    let bodyStart = -1;
    if (crlfIdx !== -1 && (lfIdx === -1 || crlfIdx <= lfIdx)) {
      headerEnd = crlfIdx;
      bodyStart = crlfIdx + 4;
    } else if (lfIdx !== -1) {
      headerEnd = lfIdx;
      bodyStart = lfIdx + 2;
    } else {
      if (transport.buffer.length >= MAX_HEADER_BYTES) {
        throw new Error('RPC header exceeds maximum size of ' + MAX_HEADER_BYTES + ' bytes');
      }
      break;
    }

    const headerStr = transport.buffer.subarray(0, headerEnd).toString('utf8');
    const contentLength = parseContentLengthHeader(headerStr);
    if (transport.buffer.length < bodyStart + contentLength) {
      break;
    }

    const bodyStr = transport.buffer
      .subarray(bodyStart, bodyStart + contentLength)
      .toString('utf8');
    transport.buffer = transport.buffer.subarray(bodyStart + contentLength);
    onMessage(JSON.parse(bodyStr));
  }
}

// Shared provider, event, tree, custom-document, and command registries live on bridgeState.

function rememberTreeElementHandle(providerId, rawElement, handle) {
  if (rawElement === undefined || rawElement === null || !handle) {
    return;
  }
  let entry = bridgeState.treeElementReverse.get(providerId);
  if (!entry) {
    entry = { seq: 1, objects: new WeakMap(), primitives: new Map() };
    bridgeState.treeElementReverse.set(providerId, entry);
  }
  const isObject = (typeof rawElement === 'object' && rawElement !== null) || typeof rawElement === 'function';
  if (isObject) {
    entry.objects.set(rawElement, handle);
  } else {
    entry.primitives.set(rawElement, handle);
  }
}

function getOrCreateTreeElementHandle(providerId, rawElement) {
  if (rawElement === undefined || rawElement === null) {
    return null;
  }
  let entry = bridgeState.treeElementReverse.get(providerId);
  if (!entry) {
    entry = { seq: 1, objects: new WeakMap(), primitives: new Map() };
    bridgeState.treeElementReverse.set(providerId, entry);
  }
  const isObject = (typeof rawElement === 'object' && rawElement !== null) || typeof rawElement === 'function';
  if (isObject) {
    let handle = entry.objects.get(rawElement);
    if (!handle) {
      handle = `__rustcode_tree_${providerId}_${entry.seq++}`;
      entry.objects.set(rawElement, handle);
    }
    return handle;
  }
  let handle = entry.primitives.get(rawElement);
  if (!handle) {
    handle = `__rustcode_tree_${providerId}_${entry.seq++}`;
    entry.primitives.set(rawElement, handle);
  }
  return handle;
}

function lookupTreeElementHandle(providerId, rawElement) {
  if (rawElement === undefined || rawElement === null) {
    return null;
  }
  if (typeof rawElement === 'string' && bridgeState.treeElements.has(`${providerId}:${rawElement}`)) {
    return rawElement;
  }
  if (typeof rawElement === 'object' && typeof rawElement.id === 'string' && rawElement.id.length > 0) {
    return rawElement.id;
  }
  const entry = bridgeState.treeElementReverse.get(providerId);
  if (!entry) {
    return typeof rawElement === 'string' ? rawElement : null;
  }
  const isObject = (typeof rawElement === 'object' && rawElement !== null) || typeof rawElement === 'function';
  if (isObject) {
    return entry.objects.get(rawElement) || null;
  }
  return entry.primitives.get(rawElement) || (typeof rawElement === 'string' ? rawElement : null);
}

function normalizeIconPathValue(rawIcon) {
  if (typeof rawIcon === 'string') {
    if (rawIcon.startsWith('file://')) {
      return Uri.parse(rawIcon).fsPath;
    }
    return rawIcon;
  }
  if (rawIcon && typeof rawIcon === 'object') {
    if (typeof rawIcon.id === 'string') {
      return rawIcon.id;
    }
    if (typeof rawIcon.fsPath === 'string' || typeof rawIcon.path === 'string') {
      return rawIcon.fsPath || rawIcon.path;
    }
  }
  return null;
}

function normalizeTreeItem(providerId, rawElement, treeItem) {
  const item = treeItem && typeof treeItem === 'object' ? treeItem : { label: String(treeItem ?? '') };
  let label = '';
  if (typeof item.label === 'string') {
    label = item.label;
  } else if (item.label && typeof item.label === 'object' && typeof item.label.label === 'string') {
    label = item.label.label;
  } else if (item.resourceUri) {
    const rawPath = item.resourceUri.fsPath || item.resourceUri.path || String(item.resourceUri);
    const trimmedPath = rawPath.replace(/[\\/]+$/, '');
    const parts = trimmedPath.split(/[\\/]/);
    label = parts[parts.length - 1] || rawPath;
  }

  let tooltip = null;
  if (typeof item.tooltip === 'string') {
    tooltip = item.tooltip;
  } else if (item.tooltip && typeof item.tooltip === 'object' && typeof item.tooltip.value === 'string') {
    tooltip = item.tooltip.value;
  }

  let iconPath = null;
  if (typeof item.iconPath === 'string') {
    iconPath = normalizeIconPathValue(item.iconPath);
  } else if (item.iconPath && typeof item.iconPath === 'object') {
    if (typeof item.iconPath.id === 'string') {
      iconPath = item.iconPath.id;
    } else if (typeof item.iconPath.fsPath === 'string' || typeof item.iconPath.path === 'string') {
      iconPath = item.iconPath.fsPath || item.iconPath.path;
    } else if (item.iconPath.dark || item.iconPath.light) {
      const picked = item.iconPath.dark || item.iconPath.light;
      iconPath = normalizeIconPathValue(picked);
    }
  }

  let itemId = typeof item.id === 'string' && item.id.length > 0 ? item.id : null;
  if (itemId === null && rawElement !== undefined && rawElement !== null) {
    itemId = getOrCreateTreeElementHandle(providerId, rawElement);
  } else if (itemId !== null && rawElement !== undefined && rawElement !== null) {
    rememberTreeElementHandle(providerId, rawElement, itemId);
  }
  if (itemId !== null && rawElement !== undefined) {
    bridgeState.treeElements.set(`${providerId}:${itemId}`, rawElement);
  }

  let resourceUri = null;
  if (item.resourceUri) {
    resourceUri = typeof item.resourceUri === 'string'
      ? item.resourceUri
      : (typeof item.resourceUri.toString === 'function' ? item.resourceUri.toString() : String(item.resourceUri));
  }

  let collapsibleState = 0;
  if (typeof item.collapsibleState === 'number' && Number.isFinite(item.collapsibleState)) {
    const truncated = Math.trunc(item.collapsibleState);
    if (truncated === 0 || truncated === 1 || truncated === 2) {
      collapsibleState = truncated;
    } else if (truncated > 2) {
      collapsibleState = 2;
    }
  } else if (typeof item.collapsibleState === 'string') {
    const lower = item.collapsibleState.toLowerCase();
    if (lower === 'collapsed') {
      collapsibleState = 1;
    } else if (lower === 'expanded') {
      collapsibleState = 2;
    }
  }

  let command = null;
  if (item.command && typeof item.command === 'object' && typeof item.command.command === 'string') {
    const c = item.command;
    command = {
      title: String(c.title !== undefined && c.title !== null ? c.title : ''),
      command: c.command,
      ...(Array.isArray(c.arguments) ? { arguments: c.arguments } : {}),
    };
  }

  return {
    id: itemId,
    label,
    description: typeof item.description === 'string' ? item.description : null,
    tooltip,
    collapsibleState,
    iconPath,
    resourceUri,
    contextValue: typeof item.contextValue === 'string' ? item.contextValue : null,
    command,
  };
}

function findRegisteredProvider(type, id) {
  const key = `${type}:${id}`;
  let proxy = bridgeState.objectProxies.get(key);
  if (!proxy) {
    for (const [k, p] of bridgeState.objectProxies.entries()) {
      if (k.startsWith(`${type}:`) && p.metadata && p.metadata.selector === id) {
        proxy = p;
        break;
      }
    }
  }
  return proxy || null;
}

function parseCustomEditorUri(uri) {
  if (!uri) {
    return Uri.file('');
  }
  if (typeof uri === 'string') {
    return Uri.parse(uri);
  }
  if (uri instanceof Uri) {
    return uri;
  }
  if (typeof uri === 'object' && typeof uri.scheme === 'string') {
    return new Uri(
      uri.scheme,
      uri.authority || '',
      uri.path || uri.fsPath || '',
      uri.query || '',
      uri.fragment || ''
    );
  }
  return Uri.file(String(uri));
}

function formatConsoleArgs(args) {
  if (nodeUtil && typeof nodeUtil.format === 'function') {
    return nodeUtil.format(...args);
  }
  return args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ');
}

function dispatchIncomingStdioMessage(msg) {
  if (!msg || typeof msg !== 'object') {
    return;
  }
  if (msg.type === 'response') {
    const transport = bridgeState.stdioTransport;
    if (!transport) {
      return;
    }
    const entry = transport.pendingCalls.get(msg.id);
    if (!entry) {
      return;
    }
    transport.pendingCalls.delete(msg.id);
    if (entry.timer) {
      clearTimeout(entry.timer);
    }
    if (msg.error !== undefined && msg.error !== null) {
      entry.reject(createRpcError(msg.error));
    } else {
      entry.resolve(msg.result === undefined ? null : msg.result);
    }
    return;
  }

  if (msg.type === 'request') {
    (async () => {
      try {
        const result = await rpc.handleIncomingRequest(msg.method, msg.params || {});
        sendStdioFrame({
          type: 'response',
          id: msg.id,
          result: result === undefined ? null : result,
          error: null,
        });
      } catch (err) {
        const code = err && typeof err.code === 'string' && err.code ? err.code : 'JS_ERROR';
        const message = err && err.message ? err.message : String(err);
        sendStdioFrame({
          type: 'response',
          id: msg.id,
          result: null,
          error: { code, message },
        });
      }
    })();
    return;
  }

  if (msg.type === 'notification') {
    if (
      msg.channel === 'commands.executeCommand' &&
      msg.data &&
      typeof msg.data === 'object' &&
      bridgeState.commandHandlers.has(msg.data.command)
    ) {
      Promise.resolve(rpc.handleIncomingRequest('commands.executeCommand', msg.data)).catch((err) => {
        if (typeof console !== 'undefined' && console.error) {
          console.error('[RustCode ExtHost] Notification command error:', err);
        }
      });
    } else {
      rpc.dispatchNotification(msg.channel, msg.data);
    }
  }
}

const rpc = {
  installVscodeModuleHook() {
    if (bridgeState.moduleHookInstalled || typeof require !== 'function') {
      return;
    }
    try {
      const Module = require('module');
      const path = require('path');
      const vscodeApiIndex = path.resolve(__dirname, 'vscode-api', 'index.js');
      const origResolveFilename = Module._resolveFilename;
      Module._resolveFilename = function (request, parent, isMain, options) {
        if (request === 'vscode') {
          return vscodeApiIndex;
        }
        return origResolveFilename.call(this, request, parent, isMain, options);
      };
      bridgeState.moduleHookInstalled = true;
    } catch (_) {}
  },

  registerInitHook(fn) {
    if (typeof fn === 'function') {
      bridgeState.initHooks.push(fn);
    }
  },

  isLiveStdioMode() {
    return Boolean(bridgeState.stdioTransport && bridgeState.stdioTransport.active);
  },

  hasSyncTransport() {
    return Boolean(ops.op_rpc_call_sync || bridgeState.syncHandler);
  },

  setSyncHandler(fn) {
    bridgeState.syncHandler = typeof fn === 'function' ? fn : null;
  },

  startStdioTransport(options = {}) {
    rpc.installVscodeModuleHook();
    if (bridgeState.stdioTransport && bridgeState.stdioTransport.active) {
      rpc.stopStdioTransport();
    }
    if (typeof Buffer === 'undefined' || typeof process === 'undefined') {
      throw new Error('Stdio RPC transport requires Node.js Buffer and process');
    }
    const input = options.input || process.stdin;
    const output = options.output || process.stdout;
    const formatEnv = (process.env && process.env.RUSTCODE_RPC_FORMAT) || 'content-length';
    const format = options.format || formatEnv;
    const redirectConsole = options.redirectConsole !== false;
    const rawWrite = output.write.bind(output);

    let origConsole = null;
    let origStdoutWrite = null;
    if (redirectConsole && process.stderr) {
      const stderrWrite = process.stderr.write.bind(process.stderr);
      origConsole = {
        log: console.log,
        info: console.info,
        debug: console.debug,
        warn: console.warn,
      };
      const logToStderr = (...args) => {
        stderrWrite(formatConsoleArgs(args) + '\n');
      };
      console.log = logToStderr;
      console.info = logToStderr;
      console.debug = logToStderr;
      console.warn = logToStderr;

      if (output === process.stdout) {
        origStdoutWrite = process.stdout.write;
        process.stdout.write = (chunk, encoding, cb) => stderrWrite(chunk, encoding, cb);
      }
    }

    const transport = {
      input,
      output,
      format,
      active: true,
      buffer: Buffer.alloc(0),
      pendingCalls: new Map(),
      nextRpcId: 1,
      rawWrite,
      origConsole,
      origStdoutWrite,
      onData: null,
      onEnd: null,
      onError: null,
      stop: () => rpc.stopStdioTransport(),
    };

    transport.onData = (chunk) => {
      if (!transport.active) {
        return;
      }
      const bufChunk = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk, 'utf8');
      transport.buffer = transport.buffer.length === 0
        ? bufChunk
        : Buffer.concat([transport.buffer, bufChunk]);
      try {
        drainStdioBuffer(transport, dispatchIncomingStdioMessage);
      } catch (err) {
        transport.buffer = Buffer.alloc(0);
        if (process.stderr) {
          process.stderr.write('[RustCode ExtHost] Stdio frame error: ' + err.message + '\n');
        }
        rpc.stopStdioTransport();
      }
    };

    transport.onEnd = () => {
      rpc.stopStdioTransport();
    };

    transport.onError = () => {
      rpc.stopStdioTransport();
    };

    bridgeState.stdioTransport = transport;
    input.on('data', transport.onData);
    input.on('end', transport.onEnd);
    input.on('error', transport.onError);
    if (typeof input.resume === 'function') {
      input.resume();
    }
    Promise.resolve().then(() => {
      for (const hook of bridgeState.initHooks) {
        try {
          Promise.resolve(hook()).catch(() => {});
        } catch (_) {}
      }
    });
    return transport;
  },

  stopStdioTransport() {
    const transport = bridgeState.stdioTransport;
    if (!transport) {
      return;
    }
    transport.active = false;
    transport.buffer = Buffer.alloc(0);
    bridgeState.stdioTransport = null;

    if (transport.input) {
      if (typeof transport.input.off === 'function') {
        transport.input.off('data', transport.onData);
        transport.input.off('end', transport.onEnd);
        transport.input.off('error', transport.onError);
      } else if (typeof transport.input.removeListener === 'function') {
        transport.input.removeListener('data', transport.onData);
        transport.input.removeListener('end', transport.onEnd);
        transport.input.removeListener('error', transport.onError);
      }
      if (typeof transport.input.pause === 'function') {
        transport.input.pause();
      }
    }

    if (transport.origConsole) {
      console.log = transport.origConsole.log;
      console.info = transport.origConsole.info;
      console.debug = transport.origConsole.debug;
      console.warn = transport.origConsole.warn;
    }
    if (transport.origStdoutWrite && typeof process !== 'undefined' && process.stdout) {
      process.stdout.write = transport.origStdoutWrite;
    }

    for (const entry of transport.pendingCalls.values()) {
      if (entry.timer) {
        clearTimeout(entry.timer);
      }
      const closeErr = new Error('RPC stdio transport closed');
      closeErr.code = 'ERR_RPC_CLOSED';
      entry.reject(closeErr);
    }
    transport.pendingCalls.clear();
  },

  // Asynchronous RPC call to Rust Core
  async call(method, params = {}, options = {}) {
    if (ops.op_rpc_call) {
      const raw = await ops.op_rpc_call(method, JSON.stringify(params));
      const parsed = typeof raw === 'string' ? JSON.parse(raw) : raw;
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed) && parsed.error) {
        throw createRpcError(parsed.error);
      }
      return parsed;
    }
    if (bridgeState.stdioTransport && bridgeState.stdioTransport.active) {
      const transport = bridgeState.stdioTransport;
      const id = transport.nextRpcId++;
      return new Promise((resolve, reject) => {
        const entry = { resolve, reject, timer: null };
        if (options && typeof options.timeoutMs === 'number' && options.timeoutMs > 0) {
          entry.timer = setTimeout(() => {
            transport.pendingCalls.delete(id);
            const timeoutErr = new Error(`RPC call '${method}' timed out after ${options.timeoutMs}ms`);
            timeoutErr.code = 'ERR_RPC_TIMEOUT';
            reject(timeoutErr);
          }, options.timeoutMs);
        }
        transport.pendingCalls.set(id, entry);
        sendStdioFrame({
          type: 'request',
          id,
          method,
          params: params === undefined ? {} : params,
        });
      });
    }
    // Headless / fallback mock mode
    return null;
  },

  notify(channel, data = {}) {
    if (bridgeState.stdioTransport && bridgeState.stdioTransport.active) {
      return sendStdioFrame({
        type: 'notification',
        channel,
        data: data === undefined ? {} : data,
      });
    }
    return false;
  },

  fireAndForget(method, params = {}) {
    if (bridgeState.stdioTransport && bridgeState.stdioTransport.active && !ops.op_rpc_call) {
      return rpc.notify(method, params);
    }
    try {
      const res = rpc.call(method, params);
      if (res && typeof res.catch === 'function') {
        res.catch(() => {});
      }
      return true;
    } catch (_) {
      return false;
    }
  },

  // Synchronous RPC call for property getters that must return immediately
  callSync(method, params = {}) {
    if (ops.op_rpc_call_sync) {
      const raw = ops.op_rpc_call_sync(method, JSON.stringify(params));
      const parsed = typeof raw === 'string' ? JSON.parse(raw) : raw;
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed) && parsed.error) {
        throw createRpcError(parsed.error);
      }
      return parsed;
    }
    if (bridgeState.syncHandler) {
      return bridgeState.syncHandler(method, params);
    }
    return null;
  },

  // Subscribable event helper
  createEvent(channel) {
    return function onEvent(listener, thisArgs, disposables) {
      const id = `${channel}_${bridgeState.nextId++}`;
      const callback = thisArgs ? listener.bind(thisArgs) : listener;
      
      let subs = bridgeState.eventSubscribers.get(channel);
      if (!subs) {
        subs = new Map();
        bridgeState.eventSubscribers.set(channel, subs);
      }
      subs.set(id, callback);

      const disposable = {
        dispose() {
          const s = bridgeState.eventSubscribers.get(channel);
          if (s) {
            s.delete(id);
          }
        }
      };

      if (disposables && Array.isArray(disposables)) {
        disposables.push(disposable);
      }
      return disposable;
    };
  },

  // Dispatch an incoming notification from Rust Core to JS event subscribers
  dispatchNotification(channel, data) {
    const subs = bridgeState.eventSubscribers.get(channel);
    if (subs) {
      for (const cb of subs.values()) {
        try {
          cb(data);
        } catch (err) {
          console.error(`[RustCode ExtHost] Error in event listener for '${channel}':`, err);
        }
      }
    }
  },

  // Register an extension provider proxy object so Rust Core can invoke provider methods
  registerObjectProxy(type, id, provider, methods, metadata = {}) {
    const key = `${type}:${id}`;
    bridgeState.objectProxies.set(key, { provider, methods, metadata });

    if (ops.op_register_provider) {
      ops.op_register_provider(type, id, JSON.stringify(methods), JSON.stringify(metadata));
    }

    rpc.fireAndForget('extHost.registerProvider', {
      type,
      id,
      methods,
      selector: metadata.selector || '*',
      triggerCharacters: metadata.triggerCharacters || [],
    });

    return {
      dispose() {
        bridgeState.objectProxies.delete(key);
        if (type === 'TreeDataProvider' || type === 'TreeData') {
          const prefix = `${id}:`;
          for (const elemKey of Array.from(bridgeState.treeElements.keys())) {
            if (elemKey.startsWith(prefix)) {
              bridgeState.treeElements.delete(elemKey);
            }
          }
          bridgeState.treeElementReverse.delete(id);
        }
        if (ops.op_unregister_provider) {
          ops.op_unregister_provider(type, id);
        }
        rpc.fireAndForget('extHost.unregisterProvider', { type, id });
      }
    };
  },

  registerCommandHandler(command, handler) {
    const token = bridgeState.nextCommandToken++;
    bridgeState.commandHandlers.set(command, { token, handler });
    return token;
  },

  unregisterCommandHandler(command, tokenOrHandler) {
    const entry = bridgeState.commandHandlers.get(command);
    if (!entry) {
      return false;
    }
    if (typeof tokenOrHandler === 'number') {
      if (entry && typeof entry === 'object' && entry.token === tokenOrHandler) {
        bridgeState.commandHandlers.delete(command);
        return true;
      }
      return false;
    }
    const existingHandler = typeof entry === 'function' ? entry : entry.handler;
    if (!tokenOrHandler || existingHandler === tokenOrHandler) {
      bridgeState.commandHandlers.delete(command);
      return true;
    }
    return false;
  },

  hasCommandHandler(command) {
    return bridgeState.commandHandlers.has(command);
  },

  getCommandHandler(command) {
    const entry = bridgeState.commandHandlers.get(command);
    if (!entry) {
      return undefined;
    }
    return typeof entry === 'function' ? entry : entry.handler;
  },

  listCommandHandlers() {
    return Array.from(bridgeState.commandHandlers.keys());
  },

  registerInputValidator(clientId, fn) {
    if (clientId && typeof fn === 'function') {
      bridgeState.inputValidators.set(clientId, fn);
    }
  },

  unregisterInputValidator(clientId) {
    if (clientId) {
      bridgeState.inputValidators.delete(clientId);
    }
  },

  resolveTreeElementHandle(providerId, rawElement) {
    return lookupTreeElementHandle(providerId, rawElement);
  },

  createWebviewPanelProxy(id, viewType, title, options = {}) {
    let _html = '';
    let _disposed = false;
    const messageEvent = rpc.createEvent(`webview.onDidReceiveMessage.${id}`);
    const disposeEvent = rpc.createEvent(`webview.onDidDispose.${id}`);

    const webview = {
      get html() {
        return _html;
      },
      set html(value) {
        if (_disposed) {
          return;
        }
        _html = String(value ?? '');
        rpc.fireAndForget('window.setWebviewHtml', { id, html: _html });
      },
      postMessage(message) {
        if (_disposed) {
          return Promise.resolve(false);
        }
        return rpc.call('window.postMessageToWebview', { id, message }).catch((err) => {
          if (err && err.code === 'ERR_RPC_CLOSED') {
            return false;
          }
          throw err;
        });
      },
      onDidReceiveMessage: messageEvent,
      options: options || {},
      cspSource: 'rustcode-webview:',
      asWebviewUri(uri) {
        return uri;
      },
    };

    return {
      id,
      viewType,
      title,
      webview,
      reveal(viewColumn) {
        if (_disposed) {
          return;
        }
        rpc.fireAndForget('window.revealWebviewPanel', { id, viewColumn });
      },
      dispose() {
        if (_disposed) {
          return;
        }
        _disposed = true;
        rpc.fireAndForget('window.disposeWebviewPanel', { id });
        rpc.dispatchNotification(`webview.onDidDispose.${id}`, {});
        bridgeState.eventSubscribers.delete(`webview.onDidReceiveMessage.${id}`);
        bridgeState.eventSubscribers.delete(`webview.onDidDispose.${id}`);
      },
      onDidDispose: disposeEvent,
    };
  },

  // Handle incoming request dispatched from Rust Core
  async handleIncomingRequest(method, params = {}) {
    if (method === 'extHost.ping') {
      return {
        pong: true,
        pid: typeof process !== 'undefined' ? process.pid : 0,
        echo: params || {},
      };
    }
    if (method === 'languages.provideDocumentFormattingEdits' || method === 'DocumentFormatting.provideDocumentFormattingEdits') {
      const { providerId, uri, options } = params;
      return await this.invokeProvider('DocumentFormatting', providerId, 'provideDocumentFormattingEdits', uri, options);
    }
    if (method === 'languages.provideCodeActions' || method === 'CodeAction.provideCodeActions') {
      const { providerId, uri, range, context } = params;
      return await this.invokeProvider('CodeAction', providerId, 'provideCodeActions', uri, range, context);
    }
    if (method === 'languages.provideCompletionItems') {
      const { providerId, uri, position, triggerCharacter } = params;
      return await this.invokeProvider('Completion', providerId, 'provideCompletionItems', uri, position, triggerCharacter);
    }
    if (method === 'languages.provideHover') {
      const { providerId, uri, position } = params;
      return await this.invokeProvider('Hover', providerId, 'provideHover', uri, position);
    }
    if (method === 'languages.provideDefinition') {
      const { providerId, uri, position } = params;
      return await this.invokeProvider('Definition', providerId, 'provideDefinition', uri, position);
    }
    if (method === 'notebooks.deserializeNotebook' || method === 'NotebookSerializer.deserializeNotebook') {
      const { providerId, notebookType, content, token } = params;
      const id = providerId || notebookType;
      let bytes;
      if (content instanceof Uint8Array) {
        bytes = content;
      } else if (Array.isArray(content)) {
        bytes = Uint8Array.from(content);
      } else if (typeof content === 'string') {
        bytes = typeof TextEncoder !== 'undefined'
          ? new TextEncoder().encode(content)
          : Uint8Array.from(Array.from(content).map((c) => c.charCodeAt(0) & 0xff));
      } else {
        bytes = new Uint8Array(0);
      }
      return await this.invokeProvider(
        'NotebookSerializer',
        id,
        'deserializeNotebook',
        bytes,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'notebooks.serializeNotebook' || method === 'NotebookSerializer.serializeNotebook') {
      const { providerId, notebookType, data, token } = params;
      const id = providerId || notebookType;
      const res = await this.invokeProvider(
        'NotebookSerializer',
        id,
        'serializeNotebook',
        data,
        token || { isCancellationRequested: false }
      );
      return res instanceof Uint8Array ? Array.from(res) : res;
    }
    if (method === 'notebooks.executeCells' || method === 'NotebookController.execute') {
      const { controllerId, providerId, cells, notebook } = params;
      return await this.invokeProvider(
        'NotebookController',
        controllerId || providerId,
        'execute',
        cells || [],
        notebook || {}
      );
    }
    if (method === 'notebooks.interruptExecution' || method === 'NotebookController.interrupt') {
      const { controllerId, providerId, notebook } = params;
      return await this.invokeProvider(
        'NotebookController',
        controllerId || providerId,
        'interrupt',
        notebook || {}
      );
    }
    if (method === 'scm.provideOriginalResource' || method === 'SourceControl.provideOriginalResource') {
      const { providerId, sourceControlId, uri, token } = params;
      return await this.invokeProvider(
        'SourceControl',
        providerId || sourceControlId,
        'provideOriginalResource',
        uri,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'scm.acceptInput' || method === 'SourceControl.acceptInput') {
      const { providerId, sourceControlId, value } = params;
      return await this.invokeProvider(
        'SourceControl',
        providerId || sourceControlId,
        'acceptInput',
        value
      );
    }
    if (method === 'debug.provideDebugConfigurations' || method === 'DebugConfiguration.provideDebugConfigurations') {
      const { providerId, debugType, folder, token } = params;
      return await this.invokeProvider(
        'DebugConfiguration',
        providerId || debugType,
        'provideDebugConfigurations',
        folder,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'debug.resolveDebugConfiguration' || method === 'DebugConfiguration.resolveDebugConfiguration') {
      const { providerId, debugType, folder, debugConfiguration, configuration, token } = params;
      return await this.invokeProvider(
        'DebugConfiguration',
        providerId || debugType,
        'resolveDebugConfiguration',
        folder,
        debugConfiguration || configuration,
        token || { isCancellationRequested: false }
      );
    }
    if (
      method === 'debug.resolveDebugConfigurationWithSubstitutedVariables' ||
      method === 'DebugConfiguration.resolveDebugConfigurationWithSubstitutedVariables'
    ) {
      const { providerId, debugType, folder, debugConfiguration, configuration, token } = params;
      return await this.invokeProvider(
        'DebugConfiguration',
        providerId || debugType,
        'resolveDebugConfigurationWithSubstitutedVariables',
        folder,
        debugConfiguration || configuration,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'debug.createDebugAdapterDescriptor' || method === 'DebugAdapterFactory.createDebugAdapterDescriptor') {
      const { providerId, debugType, session, executable } = params;
      return await this.invokeProvider(
        'DebugAdapterFactory',
        providerId || debugType,
        'createDebugAdapterDescriptor',
        session,
        executable
      );
    }
    if (method === 'tasks.provideTasks' || method === 'Task.provideTasks') {
      const { providerId, taskType, token } = params;
      return await this.invokeProvider(
        'Task',
        providerId || taskType,
        'provideTasks',
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'tasks.resolveTask' || method === 'Task.resolveTask') {
      const { providerId, taskType, task, token } = params;
      return await this.invokeProvider(
        'Task',
        providerId || taskType,
        'resolveTask',
        task,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'tests.resolveChildren' || method === 'TestController.resolveChildren') {
      const { controllerId, providerId, itemId } = params;
      return await this.invokeProvider(
        'TestController',
        controllerId || providerId,
        'resolveChildren',
        itemId
      );
    }
    if (method === 'tests.refreshTests' || method === 'TestController.refreshTests') {
      const { controllerId, providerId, token } = params;
      return await this.invokeProvider(
        'TestController',
        controllerId || providerId,
        'refreshTests',
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'tests.runTests' || method === 'TestController.runTests') {
      const { controllerId, providerId, request, token } = params;
      return await this.invokeProvider(
        'TestController',
        controllerId || providerId,
        'runTests',
        request || {},
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'authentication.getSessions' || method === 'Authentication.getSessions') {
      const { providerId, scopes, options } = params;
      return await this.invokeProvider(
        'Authentication',
        providerId,
        'getSessions',
        scopes || [],
        options || {}
      );
    }
    if (method === 'authentication.createSession' || method === 'Authentication.createSession') {
      const { providerId, scopes, options } = params;
      return await this.invokeProvider(
        'Authentication',
        providerId,
        'createSession',
        scopes || [],
        options || {}
      );
    }
    if (method === 'authentication.removeSession' || method === 'Authentication.removeSession') {
      const { providerId, sessionId } = params;
      return await this.invokeProvider(
        'Authentication',
        providerId,
        'removeSession',
        sessionId
      );
    }
    if (method === 'window.openCustomDocument' || method === 'CustomEditor.openCustomDocument') {
      const { providerId, viewType, uri, openContext, token } = params;
      const id = providerId || viewType;
      const parsedUri = parseCustomEditorUri(uri);
      const doc = await this.invokeProvider(
        'CustomEditor',
        id,
        'openCustomDocument',
        parsedUri,
        openContext || {},
        token || { isCancellationRequested: false }
      );
      bridgeState.customDocuments.set(`${id}:${parsedUri.toString()}`, doc);
      return doc || { uri: parsedUri.toString(), opened: true };
    }
    if (
      method === 'window.resolveCustomEditor' ||
      method === 'CustomEditor.resolveCustomEditor' ||
      method === 'CustomEditor.resolveCustomTextEditor'
    ) {
      const {
        providerId,
        viewType,
        uri,
        panelId,
        title,
        options,
        text,
        version,
        languageId,
        token,
      } = params;
      const id = providerId || viewType;
      const proxy = findRegisteredProvider('CustomEditor', id);
      if (!proxy || !proxy.provider) {
        throw new Error(`Provider not found: CustomEditor:${id}`);
      }
      const parsedUri = parseCustomEditorUri(uri);
      const docKey = `${id}:${parsedUri.toString()}`;
      let doc = bridgeState.customDocuments.get(docKey);
      if (!doc) {
        doc = {
          uri: parsedUri,
          fileName: parsedUri.fsPath || String(uri || ''),
          languageId: languageId || 'plaintext',
          version: typeof version === 'number' ? version : 1,
          isDirty: false,
          getText() {
            return typeof text === 'string' ? text : '';
          },
        };
      }
      const resolvedPanelId = panelId || `custom_editor_${bridgeState.nextId++}`;
      const resolvedViewType = viewType || id;
      const resolvedTitle = title || parsedUri.fsPath || resolvedViewType;
      const resolvedOptions =
        options || (proxy.metadata && proxy.metadata.webviewOptions) || {};
      const webviewPanel = this.createWebviewPanelProxy(
        resolvedPanelId,
        resolvedViewType,
        resolvedTitle,
        resolvedOptions
      );

      let methodName = 'resolveCustomEditor';
      if (method === 'CustomEditor.resolveCustomTextEditor') {
        methodName = 'resolveCustomTextEditor';
      } else if (method === 'CustomEditor.resolveCustomEditor') {
        methodName = 'resolveCustomEditor';
      } else if (
        bridgeState.customDocuments.has(docKey) &&
        typeof proxy.provider.resolveCustomEditor === 'function'
      ) {
        methodName = 'resolveCustomEditor';
      } else if (typeof proxy.provider.resolveCustomTextEditor === 'function') {
        methodName = 'resolveCustomTextEditor';
      }

      await this.invokeProvider(
        'CustomEditor',
        id,
        methodName,
        doc,
        webviewPanel,
        token || { isCancellationRequested: false }
      );
      return {
        resolved: true,
        panelId: webviewPanel.id,
        viewType: resolvedViewType,
        html: webviewPanel.webview.html,
      };
    }
    if (method === 'window.saveCustomDocument' || method === 'CustomEditor.saveCustomDocument') {
      const { providerId, viewType, uri, token } = params;
      const id = providerId || viewType;
      const parsedUri = parseCustomEditorUri(uri);
      const doc = bridgeState.customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
      return await this.invokeProvider(
        'CustomEditor',
        id,
        'saveCustomDocument',
        doc,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'window.saveCustomDocumentAs' || method === 'CustomEditor.saveCustomDocumentAs') {
      const { providerId, viewType, uri, destination, targetResource, token } = params;
      const id = providerId || viewType;
      const parsedUri = parseCustomEditorUri(uri);
      const destUri = parseCustomEditorUri(destination || targetResource);
      const doc = bridgeState.customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
      return await this.invokeProvider(
        'CustomEditor',
        id,
        'saveCustomDocumentAs',
        doc,
        destUri,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'window.revertCustomDocument' || method === 'CustomEditor.revertCustomDocument') {
      const { providerId, viewType, uri, token } = params;
      const id = providerId || viewType;
      const parsedUri = parseCustomEditorUri(uri);
      const doc = bridgeState.customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
      return await this.invokeProvider(
        'CustomEditor',
        id,
        'revertCustomDocument',
        doc,
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'window.backupCustomDocument' || method === 'CustomEditor.backupCustomDocument') {
      const { providerId, viewType, uri, context, token } = params;
      const id = providerId || viewType;
      const parsedUri = parseCustomEditorUri(uri);
      const doc = bridgeState.customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
      return await this.invokeProvider(
        'CustomEditor',
        id,
        'backupCustomDocument',
        doc,
        context || {},
        token || { isCancellationRequested: false }
      );
    }
    if (method === 'window.closeCustomDocument' || method === 'CustomEditor.closeCustomDocument') {
      const { providerId, viewType, uri } = params;
      const id = providerId || viewType;
      const parsedUri = parseCustomEditorUri(uri);
      const docKey = `${id}:${parsedUri.toString()}`;
      const doc = bridgeState.customDocuments.get(docKey);
      bridgeState.customDocuments.delete(docKey);
      if (doc && typeof doc.dispose === 'function') {
        doc.dispose();
      }
      return { closed: true, uri: parsedUri.toString() };
    }
    if (
      method === 'treeView.getChildren' ||
      method === 'TreeDataProvider.getChildren' ||
      method === 'TreeData.getChildren'
    ) {
      const id = params.providerId || params.viewId || params.id;
      const proxy = findRegisteredProvider('TreeDataProvider', id) || findRegisteredProvider('TreeData', id);
      if (!proxy || !proxy.provider) {
        throw new Error(`Provider not found: TreeDataProvider:${id}`);
      }
      if (typeof proxy.provider.getChildren !== 'function') {
        throw new Error(`Provider TreeDataProvider:${id} does not implement method getChildren`);
      }
      let element = params.element;
      if (element === null || element === undefined) {
        element = undefined;
      } else if (typeof element === 'string') {
        const cached = bridgeState.treeElements.get(`${id}:${element}`);
        if (cached !== undefined) {
          element = cached;
        }
      }
      const children = await proxy.provider.getChildren(element);
      if (!Array.isArray(children)) {
        return [];
      }
      const normalized = [];
      for (const child of children) {
        const treeItem = typeof proxy.provider.getTreeItem === 'function'
          ? await proxy.provider.getTreeItem(child)
          : child;
        normalized.push(normalizeTreeItem(id, child, treeItem));
      }
      return normalized;
    }
    if (
      method === 'treeView.getTreeItem' ||
      method === 'TreeDataProvider.getTreeItem' ||
      method === 'TreeData.getTreeItem'
    ) {
      const id = params.providerId || params.viewId || params.id;
      const proxy = findRegisteredProvider('TreeDataProvider', id) || findRegisteredProvider('TreeData', id);
      if (!proxy || !proxy.provider) {
        throw new Error(`Provider not found: TreeDataProvider:${id}`);
      }
      if (typeof proxy.provider.getTreeItem !== 'function') {
        throw new Error(`Provider TreeDataProvider:${id} does not implement method getTreeItem`);
      }
      let element = params.element;
      if (typeof element === 'string') {
        const cached = bridgeState.treeElements.get(`${id}:${element}`);
        if (cached !== undefined) {
          element = cached;
        }
      }
      const treeItem = await proxy.provider.getTreeItem(element);
      return normalizeTreeItem(id, element, treeItem);
    }
    if (
      method === 'treeView.resolveTreeItem' ||
      method === 'TreeDataProvider.resolveTreeItem' ||
      method === 'TreeData.resolveTreeItem'
    ) {
      const id = params.providerId || params.viewId || params.id;
      const proxy = findRegisteredProvider('TreeDataProvider', id) || findRegisteredProvider('TreeData', id);
      if (!proxy || !proxy.provider) {
        throw new Error(`Provider not found: TreeDataProvider:${id}`);
      }
      let element = params.element;
      if (typeof element === 'string') {
        const cached = bridgeState.treeElements.get(`${id}:${element}`);
        if (cached !== undefined) {
          element = cached;
        }
      }
      const item = params.item || (typeof proxy.provider.getTreeItem === 'function'
        ? await proxy.provider.getTreeItem(element)
        : element);
      if (typeof proxy.provider.resolveTreeItem === 'function') {
        const resolved = await proxy.provider.resolveTreeItem(item, element, params.token || { isCancellationRequested: false });
        return normalizeTreeItem(id, element, resolved || item);
      }
      return normalizeTreeItem(id, element, item);
    }
    if (method === 'window.validateInput') {
      const { clientId, value } = params || {};
      const validator = clientId ? bridgeState.inputValidators.get(clientId) : null;
      if (typeof validator !== 'function') {
        return null;
      }
      const res = await validator(value);
      return res === undefined ? null : res;
    }
    if (method === 'extensions.activate') {
      const envApi = require('./vscode-api/env');
      const targetId = params && (params.id || params.extensionId);
      if (!targetId || typeof targetId !== 'string') {
        const err = new Error('Missing extension id');
        err.code = 'INVALID_PARAMS';
        throw err;
      }
      if (typeof envApi._activateExtensionFromHost === 'function') {
        return await envApi._activateExtensionFromHost(params);
      }
      return { activated: false, id: targetId };
    }
    if (method === 'commands.executeCommand') {
      const { command, args } = params || {};
      const entry = command ? bridgeState.commandHandlers.get(command) : undefined;
      const handler = typeof entry === 'function' ? entry : (entry && entry.handler);
      if (typeof handler !== 'function') {
        const err = new Error(`Command not found in extension host: ${command}`);
        err.code = 'COMMAND_NOT_FOUND';
        throw err;
      }
      const argList = Array.isArray(args)
        ? args
        : (args !== undefined && args !== null ? [args] : []);
      return await handler(...argList);
    }
    throw new Error(`Unknown incoming method: ${method}`);
  },

  // Invoke a method on a registered provider proxy (called from Rust Core)
  async invokeProvider(type, id, method, ...args) {
    const key = `${type}:${id}`;
    const proxy = findRegisteredProvider(type, id);
    if (!proxy || !proxy.provider) {
      throw new Error(`Provider not found: ${key}`);
    }
    if (typeof proxy.provider[method] !== 'function') {
      throw new Error(`Provider ${key} does not implement method ${method}`);
    }
    return await proxy.provider[method](...args);
  }
};

module.exports = rpc;

if (typeof require === 'function' && typeof module !== 'undefined' && require.main === module) {
  rpc.installVscodeModuleHook();
  require('./vscode-api/index');
  rpc.startStdioTransport();
  const cliScript = typeof process !== 'undefined' && Array.isArray(process.argv)
    ? process.argv.slice(2).find((arg) => arg && !arg.startsWith('-'))
    : null;
  if (cliScript) {
    require(require('path').resolve(cliScript));
  }
}
