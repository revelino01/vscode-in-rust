// Main entry point for `require('vscode')` shim

const types = require('./types');
const commands = require('./commands');
const window = require('./window');
const workspace = require('./workspace');
const languages = require('./languages');
const { env, extensions } = require('./env');

const vscode = {
  // Types
  ...types,

  // Namespaces
  commands,
  window,
  workspace,
  languages,
  env,
  extensions,
  version: '1.95.0',
};

module.exports = vscode;
