// vscode.commands API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

const localCommands = new Map();

const commands = {
  registerCommand(command, callback, thisArg) {
    const handler = thisArg ? callback.bind(thisArg) : callback;
    localCommands.set(command, handler);

    rpc.call('commands.registerCommand', { command });

    return new Disposable(() => {
      localCommands.remove ? localCommands.remove(command) : localCommands.delete(command);
      rpc.call('commands.unregisterCommand', { command });
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
    return await rpc.call('commands.getCommands', { filterInternal });
  }
};

module.exports = commands;
