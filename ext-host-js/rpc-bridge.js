// MessagePack / JSON RPC bridge between the Extension Host JavaScript runtime and RustCore.

const globalDeno = typeof Deno !== 'undefined' ? Deno : null;
const ops = (globalDeno && globalDeno.core && globalDeno.core.ops) || {};

let nextId = 1;
const objectProxies = new Map();
const eventSubscribers = new Map();

const rpc = {
  // Asynchronous RPC call to Rust Core
  async call(method, params = {}) {
    if (ops.op_rpc_call) {
      const res = await ops.op_rpc_call(method, JSON.stringify(params));
      return typeof res === 'string' ? JSON.parse(res) : res;
    }
    // Headless / fallback mock mode
    return null;
  },

  // Synchronous RPC call for property getters that must return immediately
  callSync(method, params = {}) {
    if (ops.op_rpc_call_sync) {
      const res = ops.op_rpc_call_sync(method, JSON.stringify(params));
      return typeof res === 'string' ? JSON.parse(res) : res;
    }
    return null;
  },

  // Subscribable event helper
  createEvent(channel) {
    return function onEvent(listener, thisArgs, disposables) {
      const id = `${channel}_${nextId++}`;
      const callback = thisArgs ? listener.bind(thisArgs) : listener;
      
      let subs = eventSubscribers.get(channel);
      if (!subs) {
        subs = new Map();
        eventSubscribers.set(channel, subs);
      }
      subs.set(id, callback);

      const disposable = {
        dispose() {
          const s = eventSubscribers.get(channel);
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
    const subs = eventSubscribers.get(channel);
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
    objectProxies.set(key, { provider, methods, metadata });

    if (ops.op_register_provider) {
      ops.op_register_provider(type, id, JSON.stringify(methods), JSON.stringify(metadata));
    }

    rpc.call('extHost.registerProvider', {
      type,
      id,
      methods,
      selector: metadata.selector || '*',
      triggerCharacters: metadata.triggerCharacters || [],
    });

    return {
      dispose() {
        objectProxies.delete(key);
        if (ops.op_unregister_provider) {
          ops.op_unregister_provider(type, id);
        }
        rpc.call('extHost.unregisterProvider', { type, id });
      }
    };
  },

  // Handle incoming request dispatched from Rust Core
  async handleIncomingRequest(method, params = {}) {
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
    throw new Error(`Unknown incoming method: ${method}`);
  },

  // Invoke a method on a registered provider proxy (called from Rust Core)
  async invokeProvider(type, id, method, ...args) {
    let key = `${type}:${id}`;
    let proxy = objectProxies.get(key);
    if (!proxy) {
      for (const [k, p] of objectProxies.entries()) {
        if (k.endsWith(`:${id}`) || k === id) {
          proxy = p;
          break;
        }
      }
    }
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
