// vscode.authentication API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

const authentication = {
  async getSession(providerId, scopes, options = {}) {
    return await rpc.call('authentication.getSession', {
      providerId,
      scopes: Array.isArray(scopes) ? scopes : [],
      options,
    });
  },

  async getAccounts(providerId) {
    const res = await rpc.call('authentication.getAccounts', { providerId });
    return Array.isArray(res) ? res : [];
  },

  registerAuthenticationProvider(id, label, provider, options = {}) {
    const supportsMultipleAccounts = Boolean(options && options.supportsMultipleAccounts);
    const proxyDisp = rpc.registerObjectProxy(
      'Authentication',
      id,
      provider,
      ['getSessions', 'createSession', 'removeSession'],
      { selector: id, id, label, supportsMultipleAccounts }
    );

    let sessionsSub = null;
    if (provider && typeof provider.onDidChangeSessions === 'function') {
      sessionsSub = provider.onDidChangeSessions((ev) => {
        rpc.call('authentication.didChangeSessions', {
          providerId: id,
          added: (ev && ev.added) || [],
          removed: (ev && ev.removed) || [],
          changed: (ev && ev.changed) || [],
        });
      });
    }

    rpc.call('authentication.registerProvider', {
      id,
      label,
      supportsMultipleAccounts,
    });

    return new Disposable(() => {
      if (sessionsSub && typeof sessionsSub.dispose === 'function') {
        sessionsSub.dispose();
      }
      proxyDisp.dispose();
      rpc.call('authentication.unregisterProvider', { id });
    });
  },

  onDidChangeSessions: rpc.createEvent('authentication.onDidChangeSessions'),
};

module.exports = {
  authentication,
};
