// vscode.commands API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

const localCommands = new Map();

const commands = {
  registerCommand(command, callback, thisArg) {
    const handler = thisArg !== undefined ? callback.bind(thisArg) : callback;
    localCommands.set(command, handler);
    if (typeof rpc.registerCommandHandler === 'function') {
      rpc.registerCommandHandler(command, handler);
    }

    rpc.call('commands.registerCommand', { command });

    return new Disposable(() => {
      if (localCommands.get(command) === handler) {
        localCommands.delete(command);
        if (typeof rpc.unregisterCommandHandler === 'function') {
          rpc.unregisterCommandHandler(command, handler);
        }
        rpc.call('commands.unregisterCommand', { command });
      }
    });
  },

  async executeCommand(command, ...args) {
    // Check local JS commands first
    if (localCommands.has(command)) {
      return await localCommands.get(command)(...args);
    }
    // Route to Rust Core
    return await rpc.call('commands.executeCommand', { command, args });
  },

  async getCommands(filterInternal = false) {
    const res = await rpc.call('commands.getCommands', { filterInternal });
    if (Array.isArray(res)) {
      return res;
    }
    const keys = Array.from(localCommands.keys());
    const filtered = filterInternal ? keys.filter(c => !c.startsWith('_')) : keys;
    filtered.sort();
    return filtered;
  }
};

module.exports = commands;
