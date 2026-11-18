// vscode.tests API implementation

const rpc = require('../rpc-bridge');
const { Disposable, CancellationTokenSource, EventEmitter } = require('./types');

const TestRunProfileKind = {
  Run: 1,
  Debug: 2,
  Coverage: 3,
};

function _uriToString(uri) {
  if (!uri) return undefined;
  return typeof uri.toString === 'function' ? uri.toString() : String(uri);
}

class TestMessage {
  constructor(message) {
    this.message = message;
    this.expectedOutput = undefined;
    this.actualOutput = undefined;
    this.location = undefined;
    this.contextValue = undefined;
  }

  static diff(message, expected, actual) {
    const msg = new TestMessage(message);
    msg.expectedOutput = expected;
    msg.actualOutput = actual;
    return msg;
  }
}

class TestTag {
  constructor(id) {
    this.id = id;
  }
}

class TestRunRequest {
  constructor(include, exclude, profile, continuous = false) {
    this.include = include;
    this.exclude = exclude;
    this.profile = profile;
    this.continuous = Boolean(continuous);
  }

  toJSON() {
    return {
      include: Array.isArray(this.include)
        ? this.include.map((item) => (item && item.id ? item.id : item))
        : undefined,
      exclude: Array.isArray(this.exclude)
        ? this.exclude.map((item) => (item && item.id ? item.id : item))
        : undefined,
      profile: this.profile && this.profile.id ? this.profile.id : undefined,
      continuous: this.continuous,
    };
  }
}

class TestItemCollection {
  constructor(controllerId, parentItem) {
    this._controllerId = controllerId;
    this._parentItem = parentItem || undefined;
    this._items = new Map();
  }

  get size() {
    return this._items.size;
  }

  _sync() {
    if (!this._controllerId) return;
    rpc.call('tests.updateItems', {
      controllerId: this._controllerId,
      parentId: this._parentItem ? this._parentItem.id : undefined,
      items: this.toJSON(),
    });
  }

  replace(items) {
    this._items.clear();
    for (const item of items || []) {
      if (item && item.id) {
        if (this._parentItem) {
          item.parent = this._parentItem;
        }
        this._items.set(item.id, item);
      }
    }
    this._sync();
  }

  forEach(callback, thisArg) {
    for (const item of this._items.values()) {
      callback.call(thisArg, item, this);
    }
  }

  add(item) {
    if (!item || !item.id) return;
    if (this._parentItem) {
      item.parent = this._parentItem;
    }
    this._items.set(item.id, item);
    this._sync();
  }

  delete(id) {
    const removed = this._items.delete(id);
    if (removed) {
      this._sync();
    }
  }

  get(id) {
    return this._items.get(id);
  }

  [Symbol.iterator]() {
    return this._items.entries();
  }

  toJSON() {
    return Array.from(this._items.values()).map((item) =>
      typeof item.toJSON === 'function' ? item.toJSON() : item
    );
  }
}

class TestItem {
  constructor(id, label, uri, controllerId) {
    this.id = id;
    this.label = label;
    this.uri = uri;
    this._controllerId = controllerId;
    this.children = new TestItemCollection(controllerId, this);
    this.parent = undefined;
    this.tags = [];
    this.canResolveChildren = false;
    this.busy = false;
    this.description = undefined;
    this.sortText = undefined;
    this.range = undefined;
    this.error = undefined;
  }

  toJSON() {
    return {
      id: this.id,
      label: this.label,
      uri: _uriToString(this.uri),
      children: this.children ? this.children.toJSON() : [],
      tags: this.tags.map((t) => (t && t.id ? t.id : t)),
      canResolveChildren: this.canResolveChildren,
      busy: this.busy,
      description: this.description,
      sortText: this.sortText,
      range: this.range,
      error: this.error,
    };
  }
}

function _normalizeMessages(message) {
  if (!message) return [];
  const list = Array.isArray(message) ? message : [message];
  return list.map((m) => ({
    message: typeof m === 'string' ? m : m && m.message ? String(m.message) : '',
    expectedOutput: m && m.expectedOutput,
    actualOutput: m && m.actualOutput,
    location: m && m.location,
  }));
}

let nextRunId = 1;
let nextProfileId = 1;

class TestRun {
  constructor(controllerId, name, persist = true) {
    this._id = `test_run_${nextRunId++}`;
    this._controllerId = controllerId;
    this.name = name;
    this.isPersisted = persist !== false;
    this._cts = new CancellationTokenSource();
    this.token = this._cts.token;
    this._onDidDispose = new EventEmitter();
    this.onDidDispose = this._onDidDispose.event;
    this._ended = false;
  }

  _updateState(test, state, extra = {}) {
    if (this._ended) return;
    rpc.call('tests.updateTestState', {
      runId: this._id,
      controllerId: this._controllerId,
      testId: test && test.id ? test.id : String(test || ''),
      state,
      ...extra,
    });
  }

  enqueued(test) {
    this._updateState(test, 'enqueued');
  }

  started(test) {
    this._updateState(test, 'started');
  }

  skipped(test) {
    this._updateState(test, 'skipped');
  }

  passed(test, duration) {
    this._updateState(test, 'passed', { duration });
  }

  failed(test, message, duration) {
    this._updateState(test, 'failed', {
      messages: _normalizeMessages(message),
      duration,
    });
  }

  errored(test, message, duration) {
    this._updateState(test, 'errored', {
      messages: _normalizeMessages(message),
      duration,
    });
  }

  appendOutput(output, location, test) {
    if (this._ended) return;
    rpc.call('tests.appendRunOutput', {
      runId: this._id,
      controllerId: this._controllerId,
      output: output !== undefined && output !== null ? String(output) : '',
      location,
      testId: test && test.id ? test.id : undefined,
    });
  }

  end() {
    if (this._ended) return;
    this._ended = true;
    rpc.call('tests.endTestRun', {
      runId: this._id,
      controllerId: this._controllerId,
    });
    this._onDidDispose.fire();
    this._onDidDispose.dispose();
  }
}

class TestRunProfile {
  constructor(controllerId, id, label, kind, runHandler, isDefault, tag, supportsContinuousRun, onDispose) {
    this._controllerId = controllerId;
    this.id = id;
    this.label = label;
    this.kind = kind;
    this.runHandler = runHandler;
    this.isDefault = Boolean(isDefault);
    this.tag = tag;
    this.supportsContinuousRun = Boolean(supportsContinuousRun);
    this.configureHandler = undefined;
    this._disposed = false;
    this._onDispose = onDispose || null;
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    if (typeof this._onDispose === 'function') {
      this._onDispose();
      this._onDispose = null;
    }
    rpc.call('tests.disposeRunProfile', {
      controllerId: this._controllerId,
      id: this.id,
    });
  }
}

function _findTestItem(collection, targetId) {
  if (!collection || !targetId) return undefined;
  const direct = collection.get(targetId);
  if (direct) return direct;
  for (const [, item] of collection) {
    const found = _findTestItem(item.children, targetId);
    if (found) return found;
  }
  return undefined;
}

class TestController {
  constructor(id, label) {
    this.id = id;
    this.label = label;
    this.items = new TestItemCollection(id, null);
    this.resolveHandler = undefined;
    this.refreshHandler = undefined;
    this._profiles = new Map();
    this._disposed = false;
    this._onDispose = null;
  }

  createTestItem(id, label, uri) {
    return new TestItem(id, label, uri, this.id);
  }

  createRunProfile(label, kind, runHandler, isDefault = false, tag, supportsContinuousRun = false) {
    const profileId = `profile_${nextProfileId++}`;
    const profile = new TestRunProfile(
      this.id,
      profileId,
      label,
      kind,
      runHandler,
      isDefault,
      tag,
      supportsContinuousRun,
      () => {
        this._profiles.delete(profileId);
      }
    );
    this._profiles.set(profileId, profile);
    rpc.call('tests.createRunProfile', {
      controllerId: this.id,
      id: profileId,
      label,
      kind,
      isDefault: Boolean(isDefault),
      tag: tag && tag.id ? tag.id : undefined,
      supportsContinuousRun: Boolean(supportsContinuousRun),
    });
    return profile;
  }

  createTestRun(request, name, persist = true) {
    const run = new TestRun(this.id, name, persist);
    rpc.call('tests.createTestRun', {
      runId: run._id,
      controllerId: this.id,
      name,
      persist: persist !== false,
      request: request && typeof request.toJSON === 'function' ? request.toJSON() : request,
    });
    return run;
  }

  invalidateTestResults(items) {
    const ids = Array.isArray(items)
      ? items.map((i) => (i && i.id ? i.id : i))
      : items && items.id
      ? [items.id]
      : undefined;
    rpc.call('tests.invalidateTestResults', {
      controllerId: this.id,
      testIds: ids,
    });
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;
    for (const profile of Array.from(this._profiles.values())) {
      profile.dispose();
    }
    this._profiles.clear();
    if (typeof this._onDispose === 'function') {
      this._onDispose();
      this._onDispose = null;
    }
    rpc.call('tests.disposeController', { id: this.id });
  }
}

const tests = {
  createTestController(id, label) {
    const controller = new TestController(id, label);
    const proxyDisp = rpc.registerObjectProxy(
      'TestController',
      id,
      {
        resolveChildren: (itemId) => {
          if (typeof controller.resolveHandler === 'function') {
            const item = itemId ? _findTestItem(controller.items, itemId) : undefined;
            return controller.resolveHandler(item);
          }
          return undefined;
        },
        refreshTests: (token) => {
          if (typeof controller.refreshHandler === 'function') {
            return controller.refreshHandler(token);
          }
          return undefined;
        },
        runTests: (request, token) => {
          const req = request || {};
          let profile = req.profileId ? controller._profiles.get(req.profileId) : undefined;
          if (!profile) {
            for (const p of controller._profiles.values()) {
              if (p.isDefault) {
                profile = p;
                break;
              }
            }
          }
          if (!profile) {
            profile = controller._profiles.values().next().value;
          }
          if (profile && typeof profile.runHandler === 'function') {
            return profile.runHandler(req, token);
          }
          return undefined;
        },
      },
      ['resolveChildren', 'refreshTests', 'runTests'],
      { selector: id, id, label }
    );

    rpc.call('tests.createController', { id, label });
    controller._onDispose = () => {
      proxyDisp.dispose();
    };
    return controller;
  },
};

module.exports = {
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
};
