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
const {
  scm,
  SourceControlInputBoxValidationType,
  SourceControlInputBox,
  SourceControlResourceGroup,
  SourceControl,
} = require('./scm');
const {
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
} = require('./debug');
const {
  tasks,
  TaskScope,
  TaskRevealKind,
  TaskPanelKind,
  TaskGroup,
  ProcessExecution,
  ShellExecution,
  CustomExecution,
  Task,
} = require('./tasks');
const {
  tests,
  TestRunProfileKind,
  TestMessage,
  TestTag,
  TestRunRequest,
  TestItem,
  TestItemCollection,
  TestRun,
  TestRunProfile,
  TestController,
} = require('./tests');
const { authentication } = require('./authentication');

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
  SourceControlInputBoxValidationType,
  SourceControlInputBox,
  SourceControlResourceGroup,
  SourceControl,
  DebugConsoleMode,
  DebugConfigurationProviderTriggerKind,
  Breakpoint,
  SourceBreakpoint,
  FunctionBreakpoint,
  DebugAdapterExecutable,
  DebugAdapterServer,
  DebugAdapterNamedPipeServer,
  DebugAdapterInlineImplementation,
  TaskScope,
  TaskRevealKind,
  TaskPanelKind,
  TaskGroup,
  ProcessExecution,
  ShellExecution,
  CustomExecution,
  Task,
  TestRunProfileKind,
  TestMessage,
  TestTag,
  TestRunRequest,
  TestItem,
  TestItemCollection,
  TestRun,
  TestRunProfile,
  TestController,

  // Namespaces
  commands,
  window,
  workspace,
  languages,
  notebooks,
  scm,
  debug,
  tasks,
  tests,
  authentication,
  env,
  extensions,
  version: '1.95.0',
};

module.exports = vscode;
