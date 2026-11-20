// vscode.commands API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

const localCommands = new Map();
const localCommandTokens = new Map();

const commands = {
  registerCommand(command, callback, thisArg) {
    if (typeof command !== 'string' || !command.trim()) {
      throw new Error('registerCommand requires a non-empty command string');
    }
    const handler = thisArg !== undefined ? callback.bind(thisArg) : callback;
    localCommands.set(command, handler);
    const token = typeof rpc.registerCommandHandler === 'function'
      ? rpc.registerCommandHandler(command, handler)
      : undefined;
    if (token !== undefined) {
      localCommandTokens.set(command, token);
    }

    rpc.fireAndForget('commands.registerCommand', { command });

    return new Disposable(() => {
      const tokenMatches = token === undefined || localCommandTokens.get(command) === token;
      if (localCommands.get(command) === handler) {
        if (tokenMatches) {
          localCommands.delete(command);
          localCommandTokens.delete(command);
        }
      }
      const removedActive = typeof rpc.unregisterCommandHandler === 'function'
        ? rpc.unregisterCommandHandler(command, token !== undefined ? token : handler)
        : tokenMatches;
      if (removedActive) {
        rpc.fireAndForget('commands.unregisterCommand', { command });
      }
    });
  },

  async executeCommand(command, ...args) {
    // When shared rpc-bridge command registry is available, treat it as the sole
    // authoritative JS command registry (avoids stale module-local fallback after
    // active registration disposal across Windows drive-letter casing splits).
    const localHandler =
      typeof rpc.getCommandHandler === 'function'
        ? rpc.getCommandHandler(command)
        : localCommands.get(command);
    if (localHandler) {
      return await localHandler(...args);
    }
    // Route to Rust Core
    return await rpc.call('commands.executeCommand', { command, args });
  },

  async getCommands(filterInternal = false) {
    const filter = Boolean(filterInternal);
    const res = await rpc.call('commands.getCommands', { filterInternal: filter });
    const merged = new Set(Array.isArray(res) ? res : []);
    if (typeof rpc.listCommandHandlers === 'function') {
      for (const key of rpc.listCommandHandlers()) {
        merged.add(key);
      }
    } else {
      for (const key of localCommands.keys()) {
        merged.add(key);
      }
    }
    const list = Array.from(merged);
    const filtered = filter ? list.filter(c => !c.startsWith('_')) : list;
    filtered.sort();
    return filtered;
  }
};

module.exports = commands;
