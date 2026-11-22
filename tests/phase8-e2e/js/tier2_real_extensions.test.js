'use strict';

const assert = require('assert/strict');
const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const RPC_BRIDGE_PATH = path.join(REPO_ROOT, 'ext-host-js', 'rpc-bridge.js');
const ACTIVE_CHILDREN = new Set();

process.on('exit', () => {
  for (const child of ACTIVE_CHILDREN) {
    if (child.exitCode === null && child.signalCode === null) {
      try {
        child.kill('SIGKILL');
      } catch (_) {}
    }
  }
});

function createSuite(suiteName) {
  const tests = [];
  return {
    test(name, fn) {
      tests.push({ name, fn });
    },
    async run() {
      let filter = null;
      const argv = process.argv.slice(2);
      for (let i = 0; i < argv.length; i++) {
        if (argv[i] === '--filter' && i + 1 < argv.length) filter = argv[i + 1];
        else if (argv[i].startsWith('--filter=')) filter = argv[i].slice('--filter='.length);
      }
      const selected = filter ? tests.filter((t) => t.name.includes(filter)) : tests;
      process.stdout.write(`TAP version 13\n# Suite: ${suiteName}\n1..${selected.length}\n`);
      let passed = 0;
      let failed = 0;
      for (let i = 0; i < selected.length; i++) {
        const t = selected[i];
        const startedAt = Date.now();
        try {
          await t.fn();
          passed++;
          process.stdout.write(`ok ${i + 1} - ${t.name} (${Date.now() - startedAt}ms)\n`);
        } catch (err) {
          failed++;
          process.stdout.write(`not ok ${i + 1} - ${t.name} (${Date.now() - startedAt}ms)\n`);
          process.stdout.write(`  # ${err && err.stack ? err.stack.replace(/\n/g, '\n  # ') : String(err)}\n`);
        }
      }
      process.stdout.write(`# Summary (${suiteName}): ${passed}/${selected.length} passed, ${failed} failed\n`);
      if (failed > 0) process.exitCode = 1;
    },
  };
}

function encodeFrame(obj) {
  const body = Buffer.from(JSON.stringify(obj), 'utf8');
  const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, 'ascii');
  return Buffer.concat([header, body]);
}

function spawnExtHostClient() {
  const child = spawn(process.execPath, [RPC_BRIDGE_PATH], {
    cwd: REPO_ROOT,
    env: { ...process.env, RUSTCODE_RPC_FORMAT: 'content-length' },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  ACTIVE_CHILDREN.add(child);

  let buf = Buffer.alloc(0);
  let nextReqId = 1;
  const pending = new Map();
  const outboundRequests = [];
  const notifications = [];
  let onHostRequest = null;

  child.once('exit', (code, signal) => {
    ACTIVE_CHILDREN.delete(child);
    for (const { reject, timer } of pending.values()) {
      clearTimeout(timer);
      reject(new Error(`Extension host exited early (code=${code}, signal=${signal})`));
    }
    pending.clear();
  });

  child.stdout.on('data', (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    while (true) {
      const headerEnd = buf.indexOf('\r\n\r\n');
      if (headerEnd === -1) break;
      const header = buf.slice(0, headerEnd).toString('ascii');
      const m = /content-length:\s*(\d+)/i.exec(header);
      if (!m) {
        buf = buf.slice(headerEnd + 4);
        continue;
      }
      const len = parseInt(m[1], 10);
      const bodyStart = headerEnd + 4;
      if (buf.length < bodyStart + len) break;
      const raw = buf.slice(bodyStart, bodyStart + len).toString('utf8');
      buf = buf.slice(bodyStart + len);
      const msg = JSON.parse(raw);

      if (msg.type === 'response' && pending.has(msg.id)) {
        const { resolve, reject, timer } = pending.get(msg.id);
        clearTimeout(timer);
        pending.delete(msg.id);
        if (msg.error) reject(new Error(typeof msg.error === 'string' ? msg.error : msg.error.message));
        else resolve(msg.result);
      } else if (msg.type === 'request') {
        outboundRequests.push(msg);
        Promise.resolve(onHostRequest ? onHostRequest(msg.method, msg.params || {}) : null)
          .then((result) => {
            if (!child.killed && child.stdin.writable) {
              child.stdin.write(encodeFrame({ type: 'response', id: msg.id, result }));
            }
          })
          .catch((err) => {
            if (!child.killed && child.stdin.writable) {
              child.stdin.write(encodeFrame({ type: 'response', id: msg.id, error: err.message }));
            }
          });
      } else if (msg.type === 'notification') {
        notifications.push(msg);
      }
    }
  });

  function sendRequest(method, params = {}, timeoutMs = 5000) {
    const id = nextReqId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (pending.delete(id)) {
          reject(new Error(`RPC request '${method}' timed out after ${timeoutMs}ms`));
        }
      }, timeoutMs);
      if (typeof timer.unref === 'function') timer.unref();
      pending.set(id, { resolve, reject, timer });
      child.stdin.write(encodeFrame({ type: 'request', id, method, params }));
    });
  }

  function sendNotification(channel, data) {
    child.stdin.write(encodeFrame({ type: 'notification', channel, data }));
  }

  function close() {
    return new Promise((resolve) => {
      if (child.exitCode !== null || child.signalCode !== null) {
        ACTIVE_CHILDREN.delete(child);
        return resolve(child.exitCode);
      }
      let settled = false;
      const finish = (code) => {
        if (settled) return;
        settled = true;
        clearTimeout(killTimer);
        clearTimeout(giveUpTimer);
        ACTIVE_CHILDREN.delete(child);
        resolve(code);
      };
      child.once('exit', (code) => finish(code));
      try {
        child.stdin.end();
      } catch (_) {}
      const killTimer = setTimeout(() => {
        if (child.exitCode === null && child.signalCode === null) {
          try {
            child.kill('SIGKILL');
          } catch (_) {}
        }
      }, 800);
      if (typeof killTimer.unref === 'function') killTimer.unref();
      const giveUpTimer = setTimeout(() => finish(child.exitCode), 2500);
      if (typeof giveUpTimer.unref === 'function') giveUpTimer.unref();
    });
  }

  return {
    child,
    outboundRequests,
    notifications,
    setHostHandler(fn) {
      onHostRequest = fn;
    },
    sendRequest,
    sendNotification,
    close,
  };
}

const suite = createSuite('tier2_real_extensions');

suite.test('[M2-20, M4-08] Live Node ext-host process activates extension, runs require(vscode), and roundtrips Webview postMessage', async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'phase8_js_exthost_'));
  let host = null;
  try {
    fs.writeFileSync(
      path.join(tmpDir, 'package.json'),
      JSON.stringify({
        name: 'live-webview-ext',
        publisher: 'Phase8',
        version: '1.0.0',
        main: './extension.js',
        activationEvents: ['*'],
      })
    );
    fs.writeFileSync(
      path.join(tmpDir, 'extension.js'),
      `const vscode = require('vscode');
exports.activate = (context) => {
  const received = [];
  const panel = vscode.window.createWebviewPanel('p8.view', 'P8 Panel', 1, { enableScripts: true });
  panel.webview.onDidReceiveMessage((m) => received.push(m));
  panel.webview.postMessage({ fromExt: true });
  context.subscriptions.push(
    panel,
    vscode.commands.registerCommand('p8.getWebviewMsgs', () => received)
  );
  return { activated: true };
};
`
    );

    host = spawnExtHostClient();
    const postedToWebview = [];

    host.setHostHandler((method, params) => {
      if (method === 'extensions.getAll') {
        return [{
          id: 'Phase8.live-webview-ext',
          extensionPath: tmpDir,
          isActive: false,
          packageJSON: { main: './extension.js' },
        }];
      }
      if (method === 'workspace.getWorkspaceFolders') return [];
      if (method === 'env.getEnvironment') return { appRoot: REPO_ROOT, shell: 'sh', uriScheme: 'rustcode', language: 'en' };
      if (method === 'window.postMessageToWebview') {
        postedToWebview.push(params);
        return true;
      }
      return null;
    });

    const actRes = await host.sendRequest('extensions.activate', {
      id: 'Phase8.live-webview-ext',
      extensionPath: tmpDir,
      main: './extension.js',
    });
    assert.equal(actRes.activated, true);

    for (let i = 0; i < 20; i++) {
      const hasCreate = host.notifications.some((n) => n.channel === 'window.createWebviewPanel');
      if (hasCreate && postedToWebview.length > 0) break;
      await new Promise((r) => setTimeout(r, 20));
    }

    const createNotif = host.notifications.find((n) => n.channel === 'window.createWebviewPanel');
    assert.ok(createNotif && createNotif.data && createNotif.data.id, 'expected window.createWebviewPanel notification');
    const panelId = createNotif.data.id;
    assert.equal(createNotif.data.viewType, 'p8.view');
    assert.equal(postedToWebview.length, 1);
    assert.equal(postedToWebview[0].id, panelId);
    assert.deepEqual(postedToWebview[0].message, { fromExt: true });

    host.sendNotification(`webview.onDidReceiveMessage.${panelId}`, {
      fromWebview: 'hello',
    });
    await new Promise((r) => setTimeout(r, 40));

    const msgs = await host.sendRequest('commands.executeCommand', {
      command: 'p8.getWebviewMsgs',
      args: [],
    });
    assert.deepEqual(msgs, [{ fromWebview: 'hello' }]);
  } finally {
    if (host) await host.close();
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

suite.test('[M2-20, ADV-04] Live Node ext-host unexpected termination is detected cleanly and allows fresh respawn', async () => {
  const host1 = spawnExtHostClient();
  try {
    const exitPromise = new Promise((resolve) => {
      host1.child.once('exit', (code, signal) => resolve({ code, signal }));
    });
    host1.child.kill('SIGTERM');
    const exitInfo = await exitPromise;
    assert.ok(exitInfo.code !== null || exitInfo.signal !== null);
  } finally {
    await host1.close();
  }

  // Respawn fresh host process and verify ping + registerCommand/executeCommand RPC flow
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'phase8_js_respawn_'));
  let host2 = null;
  try {
    fs.writeFileSync(
      path.join(tmpDir, 'package.json'),
      JSON.stringify({
        name: 'respawn-cmd-ext',
        publisher: 'Phase8',
        version: '1.0.0',
        main: './extension.js',
        activationEvents: ['*'],
      })
    );
    fs.writeFileSync(
      path.join(tmpDir, 'extension.js'),
      `const vscode = require('vscode');
exports.activate = (context) => {
  context.subscriptions.push(
    vscode.commands.registerCommand('p8.respawnEcho', (arg) => ({ echoed: arg }))
  );
  return { ready: true };
};
`
    );

    host2 = spawnExtHostClient();
    host2.setHostHandler((method) => {
      if (method === 'extensions.getAll') {
        return [{
          id: 'Phase8.respawn-cmd-ext',
          extensionPath: tmpDir,
          isActive: false,
          packageJSON: { main: './extension.js' },
        }];
      }
      if (method === 'workspace.getWorkspaceFolders') return [];
      if (method === 'env.getEnvironment') return { appRoot: REPO_ROOT, shell: 'sh', uriScheme: 'rustcode', language: 'en' };
      return null;
    });

    const ping = await host2.sendRequest('extHost.ping', { probe: 'respawn' });
    assert.equal(ping.pong, true);

    const act = await host2.sendRequest('extensions.activate', {
      id: 'Phase8.respawn-cmd-ext',
      extensionPath: tmpDir,
      main: './extension.js',
    });
    assert.equal(act.activated, true);

    for (let i = 0; i < 20; i++) {
      if (
        host2.notifications.some(
          (n) => n.channel === 'commands.registerCommand' && n.data && n.data.command === 'p8.respawnEcho'
        )
      ) {
        break;
      }
      await new Promise((r) => setTimeout(r, 20));
    }
    assert.ok(
      host2.notifications.some(
        (n) => n.channel === 'commands.registerCommand' && n.data && n.data.command === 'p8.respawnEcho'
      ),
      'expected commands.registerCommand notification'
    );

    const execRes = await host2.sendRequest('commands.executeCommand', {
      command: 'p8.respawnEcho',
      args: ['ok'],
    });
    assert.deepEqual(execRes, { echoed: 'ok' });
  } finally {
    if (host2) await host2.close();
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

suite.run();
