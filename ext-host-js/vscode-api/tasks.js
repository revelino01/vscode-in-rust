// vscode.tasks API implementation

const rpc = require('../rpc-bridge');
const { Disposable } = require('./types');

const TaskScope = {
  Global: 1,
  Workspace: 2,
};

const TaskRevealKind = {
  Always: 1,
  Silent: 2,
  Never: 3,
};

const TaskPanelKind = {
  Shared: 1,
  Dedicated: 2,
  New: 3,
};

class TaskGroup {
  constructor(id, label) {
    this.id = id;
    this.label = label;
    this.isDefault = undefined;
  }
}

TaskGroup.Clean = new TaskGroup('clean', 'Clean');
TaskGroup.Build = new TaskGroup('build', 'Build');
TaskGroup.Test = new TaskGroup('test', 'Test');

class ProcessExecution {
  constructor(process, argsOrOptions, options) {
    this.process = process;
    if (Array.isArray(argsOrOptions)) {
      this.args = argsOrOptions;
      this.options = options;
    } else {
      this.args = [];
      this.options = argsOrOptions;
    }
  }

  toJSON() {
    return {
      type: 'process',
      process: this.process,
      args: this.args,
      options: this.options,
    };
  }
}

class ShellExecution {
  constructor(commandLineOrCommand, argsOrOptions, options) {
    if (typeof commandLineOrCommand === 'string' && !Array.isArray(argsOrOptions)) {
      this.commandLine = commandLineOrCommand;
      this.command = undefined;
      this.args = [];
      this.options = argsOrOptions;
    } else {
      this.commandLine = undefined;
      this.command = commandLineOrCommand;
      this.args = Array.isArray(argsOrOptions) ? argsOrOptions : [];
      this.options = options;
    }
  }

  toJSON() {
    return {
      type: 'shell',
      commandLine: this.commandLine,
      command: this.command,
      args: this.args,
      options: this.options,
    };
  }
}

class CustomExecution {
  constructor(callback) {
    this.callback = callback;
  }

  toJSON() {
    return { type: 'custom' };
  }
}

class Task {
  constructor(taskDefinition, scope, name, source, execution, problemMatchers) {
    this.definition = taskDefinition;
    this.scope = scope;
    this.name = name;
    this.source = source;
    this.execution = execution;
    this.problemMatchers = Array.isArray(problemMatchers)
      ? problemMatchers
      : problemMatchers
      ? [problemMatchers]
      : [];
    this.isBackground = false;
    this.group = undefined;
    this.presentationOptions = {};
    this.runOptions = {};
    this.detail = undefined;
  }

  toJSON() {
    return {
      definition: this.definition,
      scope: this.scope,
      name: this.name,
      source: this.source,
      execution: this.execution,
      problemMatchers: this.problemMatchers,
      isBackground: this.isBackground,
      group: this.group,
      presentationOptions: this.presentationOptions,
      runOptions: this.runOptions,
      detail: this.detail,
    };
  }
}

let nextTaskProviderId = 1;
let nextTaskExecutionId = 1;
const activeExecutions = [];

const tasks = {
  get taskExecutions() {
    return activeExecutions.slice();
  },

  registerTaskProvider(type, provider) {
    const id = `task_provider_${nextTaskProviderId++}`;
    const proxyDisp = rpc.registerObjectProxy(
      'Task',
      id,
      provider,
      ['provideTasks', 'resolveTask'],
      { selector: type, taskType: type }
    );
    rpc.call('tasks.registerProvider', { id, type });
    return new Disposable(() => {
      proxyDisp.dispose();
      rpc.call('tasks.unregisterProvider', { id, type });
    });
  },

  async fetchTasks(filter) {
    const res = await rpc.call('tasks.fetchTasks', { filter: filter || {} });
    return Array.isArray(res) ? res : [];
  },

  async executeTask(task) {
    const executionId = `task_exec_${nextTaskExecutionId++}`;
    await rpc.call('tasks.executeTask', { id: executionId, task });
    const exec = {
      id: executionId,
      task,
      terminate() {
        const idx = activeExecutions.indexOf(exec);
        if (idx !== -1) {
          activeExecutions.splice(idx, 1);
        }
        return rpc.call('tasks.terminateTask', {
          id: executionId,
          name: task && task.name,
          source: task && task.source,
        });
      },
    };
    activeExecutions.push(exec);
    return exec;
  },

  onDidStartTask: rpc.createEvent('tasks.onDidStartTask'),
  onDidEndTask: rpc.createEvent('tasks.onDidEndTask'),
  onDidStartTaskProcess: rpc.createEvent('tasks.onDidStartTaskProcess'),
  onDidEndTaskProcess: rpc.createEvent('tasks.onDidEndTaskProcess'),
};

module.exports = {
  tasks,
  TaskScope,
  TaskRevealKind,
  TaskPanelKind,
  TaskGroup,
  ProcessExecution,
  ShellExecution,
  CustomExecution,
  Task,
};
