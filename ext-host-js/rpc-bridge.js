// MessagePack / JSON RPC bridge between the Extension Host JavaScript runtime and RustCore.

const { Uri } = require('./vscode-api/types');

const globalDeno = typeof Deno !== 'undefined' ? Deno : null;
const ops = (globalDeno && globalDeno.core && globalDeno.core.ops) || {};

let nextId = 1;
const objectProxies = new Map();
const eventSubscribers = new Map();
const customDocuments = new Map();

function findRegisteredProvider(type, id) {
  const key = `${type}:${id}`;
  let proxy = objectProxies.get(key);
  if (!proxy) {
    for (const [k, p] of objectProxies.entries()) {
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
        rpc.call('window.setWebviewHtml', { id, html: _html });
      },
      postMessage(message) {
        if (_disposed) {
          return Promise.resolve(false);
        }
        return rpc.call('window.postMessageToWebview', { id, message });
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
        rpc.call('window.revealWebviewPanel', { id, viewColumn });
      },
      dispose() {
        if (_disposed) {
          return;
        }
        _disposed = true;
        rpc.call('window.disposeWebviewPanel', { id });
        rpc.dispatchNotification(`webview.onDidDispose.${id}`, {});
        eventSubscribers.delete(`webview.onDidReceiveMessage.${id}`);
        eventSubscribers.delete(`webview.onDidDispose.${id}`);
      },
      onDidDispose: disposeEvent,
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
      customDocuments.set(`${id}:${parsedUri.toString()}`, doc);
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
      let doc = customDocuments.get(docKey);
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
      const resolvedPanelId = panelId || `custom_editor_${nextId++}`;
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
        customDocuments.has(docKey) &&
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
      const doc = customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
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
      const doc = customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
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
      const doc = customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
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
      const doc = customDocuments.get(`${id}:${parsedUri.toString()}`) || { uri: parsedUri };
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
      const doc = customDocuments.get(docKey);
      customDocuments.delete(docKey);
      if (doc && typeof doc.dispose === 'function') {
        doc.dispose();
      }
      return { closed: true, uri: parsedUri.toString() };
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
