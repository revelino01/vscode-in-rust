// Main entry point for `require('vscode')` shim

const types = require('./types');
const commands = require('./commands');
const window = require('./window');
const workspace = require('./workspace');
const languages = require('./languages');
const { env, extensions } = require('./env');
const {
  notebooks,
  NotebookCellKind,
  NotebookCellData,
  NotebookData,
  NotebookCellOutput,
  NotebookCellOutputItem,
  NotebookRange,
  NotebookEdit,
  NotebookController,
  NotebookCellExecution,
} = require('./notebooks');

const vscode = {
  // Types
  ...types,
  NotebookCellKind,
  NotebookCellData,
  NotebookData,
  NotebookCellOutput,
  NotebookCellOutputItem,
  NotebookRange,
  NotebookEdit,
  NotebookController,
  NotebookCellExecution,

  // Namespaces
  commands,
  window,
  workspace,
  languages,
  notebooks,
  env,
  extensions,
  version: '1.95.0',
};

module.exports = vscode;
