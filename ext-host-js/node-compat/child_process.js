'use strict';

// Pure-JS + RPC-backed Node.js child_process compatibility shim (no dependency on Node built-in 'child_process').

const rpc = require('../rpc-bridge');
const EventEmitter = require('./events');
const { Buffer } = require('./buffer');

const customPromisifySymbol = Symbol.for('nodejs.util.promisify.custom');

function parsePrefixedCode(raw) {
  if (typeof raw !== 'string') {
    return null;
  }
  const idx = raw.indexOf(': ');
  if (idx <= 0) {
    return null;
  }
  const prefix = raw.slice(0, idx);
  for (let i = 0; i < prefix.length; i++) {
    const ch = prefix.charCodeAt(i);
    const ok = (ch >= 65 && ch <= 90) || (ch >= 48 && ch <= 57) || ch === 95;
    if (!ok) {
      return null;
    }
  }
  return { code: prefix, message: raw.slice(idx + 2) };
}

function toRpcError(errPayload) {
  if (errPayload instanceof Error) {
    return errPayload;
  }
  if (!errPayload) {
    return new Error('Process RPC error');
  }
  if (typeof errPayload === 'string') {
    const parsed = parsePrefixedCode(errPayload);
    if (parsed) {
      const err = new Error(parsed.message);
      err.code = parsed.code;
      return err;
    }
    return new Error(errPayload);
  }
  let message = errPayload.message || 'Process RPC error';
  let code = errPayload.code || null;
  if (!code && typeof errPayload === 'object' && typeof errPayload.error === 'string') {
    const parsed = parsePrefixedCode(errPayload.error);
    if (parsed) {
      code = parsed.code;
      message = parsed.message;
    }
  }
  const err = new Error(message);
  if (code) {
    err.code = code;
  }
  return err;
}

function assertSyncSupported(methodName) {
  if (
    typeof rpc.isLiveStdioMode === 'function' &&
    rpc.isLiveStdioMode() &&
    !(typeof rpc.hasSyncTransport === 'function' && rpc.hasSyncTransport())
  ) {
    const err = new Error(
      `ENOSYS: ${methodName} is unsupported over async Node.js stdio transport; use async child_process APIs instead`
    );
    err.code = 'ENOSYS';
    throw err;
  }
}

class ChildStream extends EventEmitter {
  constructor(readable) {
    super();
    this.readable = readable;
    this.writable = !readable;
    this._encoding = null;
    this._destroyed = false;
  }

  setEncoding(encoding) {
    this._encoding = encoding || 'utf8';
    return this;
  }

  pause() {
    return this;
  }

  resume() {
    return this;
  }

  write() {
    return !this._destroyed;
  }

  end() {
    if (!this._destroyed) {
      this._destroyed = true;
      this.emit('finish');
    }
    return this;
  }

  destroy() {
    if (!this._destroyed) {
      this._destroyed = true;
      this.emit('close');
    }
    return this;
  }
}

class ChildProcess extends EventEmitter {
  constructor(command, args) {
    super();
    this.spawnfile = command || '';
    this.spawnargs = [this.spawnfile, ...(Array.isArray(args) ? args : [])];
    this.stdin = new ChildStream(false);
    this.stdout = new ChildStream(true);
    this.stderr = new ChildStream(true);
    this.stdio = [this.stdin, this.stdout, this.stderr];
    this.pid = undefined;
    this.killed = false;
    this.connected = false;
    this.exitCode = null;
    this.signalCode = null;
  }

  kill(signal = 'SIGTERM') {
    this.killed = true;
    void signal;
    return true;
  }
}

function toStreamChunk(raw, streamEncoding) {
  if (raw === undefined || raw === null) {
    return null;
  }
  let buf;
  if (Buffer.isBuffer(raw)) {
    buf = raw;
  } else if (raw instanceof Uint8Array || Array.isArray(raw)) {
    buf = Buffer.from(raw);
  } else {
    const str = String(raw);
    if (str.length === 0) {
      return null;
    }
    buf = Buffer.from(str, 'utf8');
  }
  if (buf.length === 0) {
    return null;
  }
  if (streamEncoding && streamEncoding !== 'buffer') {
    return buf.toString(streamEncoding);
  }
  return buf;
}

function formatTextOutput(raw, encoding) {
  if (raw === undefined || raw === null) {
    return encoding === 'buffer' ? Buffer.alloc(0) : '';
  }
  if (encoding === 'buffer') {
    if (Buffer.isBuffer(raw)) return raw;
    if (raw instanceof Uint8Array || Array.isArray(raw)) return Buffer.from(raw);
    return Buffer.from(String(raw), 'utf8');
  }
  if (Buffer.isBuffer(raw)) {
    return raw.toString(encoding || 'utf8');
  }
  if (raw instanceof Uint8Array || Array.isArray(raw)) {
    return Buffer.from(raw).toString(encoding || 'utf8');
  }
  if (encoding && encoding !== 'utf8' && encoding !== 'utf-8') {
    return Buffer.from(String(raw), 'utf8').toString(encoding);
  }
  return String(raw);
}

function parseSpawnArgs(args, options) {
  let argList = [];
  let opts = {};
  if (Array.isArray(args)) {
    argList = args.map((a) => String(a));
    if (options && typeof options === 'object') {
      opts = options;
    }
  } else if (args && typeof args === 'object') {
    opts = args;
  } else if (options && typeof options === 'object') {
    opts = options;
  }
  return { argList, opts };
}

function parseExecArgs(options, callback) {
  let opts = {};
  let cb = null;
  if (typeof options === 'function') {
    cb = options;
  } else {
    if (options && typeof options === 'object') {
      opts = options;
    }
    if (typeof callback === 'function') {
      cb = callback;
    }
  }
  return { opts, cb };
}

function parseExecFileArgs(args, options, callback) {
  let argList = [];
  let opts = {};
  let cb = null;
  if (typeof args === 'function') {
    cb = args;
  } else if (Array.isArray(args)) {
    argList = args.map((a) => String(a));
    if (typeof options === 'function') {
      cb = options;
    } else {
      if (options && typeof options === 'object') {
        opts = options;
      }
      if (typeof callback === 'function') {
        cb = callback;
      }
    }
  } else if (args && typeof args === 'object') {
    opts = args;
    if (typeof options === 'function') {
      cb = options;
    }
  }
  return { argList, opts, cb };
}

function buildExecPayload(command, opts) {
  const payload = { command };
  if (opts.cwd !== undefined) payload.cwd = opts.cwd;
  if (opts.env !== undefined) payload.env = opts.env;
  if (opts.timeout !== undefined) payload.timeout = opts.timeout;
  if (opts.maxBuffer !== undefined) payload.maxBuffer = opts.maxBuffer;
  return payload;
}

function buildSpawnPayload(command, argList, opts) {
  const payload = { command, args: argList };
  if (opts.cwd !== undefined) payload.cwd = opts.cwd;
  if (opts.env !== undefined) payload.env = opts.env;
  if (opts.timeout !== undefined) payload.timeout = opts.timeout;
  if (opts.shell !== undefined) payload.shell = opts.shell;
  if (opts.input !== undefined) payload.input = opts.input;
  return payload;
}

function spawn(command, args, options) {
  const { argList, opts } = parseSpawnArgs(args, options);
  const child = new ChildProcess(typeof command === 'string' ? command : '', argList);

  Promise.resolve().then(async () => {
    if (typeof command !== 'string' || command.length === 0) {
      const err = new Error('EINVAL: command must be a non-empty string');
      err.code = 'EINVAL';
      child.emit('error', err);
      child.emit('close', null, null);
      return;
    }

    let res;
    try {
      res = await rpc.call('process.spawn', buildSpawnPayload(command, argList, opts));
    } catch (rpcErr) {
      const err = toRpcError(rpcErr);
      child.emit('error', err);
      child.emit('close', null, null);
      return;
    }

    if (res && typeof res === 'object' && res.error) {
      const err = toRpcError(res.error);
      child.emit('error', err);
      child.emit('close', null, null);
      return;
    }

    if (res === null || res === undefined) {
      child.pid = 0;
      child.exitCode = 0;
      child.signalCode = null;
      child.emit('spawn');
      child.stdout.emit('end');
      child.stderr.emit('end');
      child.stdout.emit('close');
      child.stderr.emit('close');
      child.emit('exit', 0, null);
      child.emit('close', 0, null);
      return;
    }

    child.pid = typeof res.pid === 'number' ? res.pid : 0;
    child.exitCode =
      res.exitCode !== undefined
        ? res.exitCode
        : (res.status !== undefined ? res.status : 0);
    child.signalCode = res.signal !== undefined ? res.signal : null;
    if (res.killed || res.timedOut) {
      child.killed = true;
    }

    child.emit('spawn');

    const outChunk = toStreamChunk(res.stdout, child.stdout._encoding);
    if (outChunk !== null) {
      child.stdout.emit('data', outChunk);
    }
    const errChunk = toStreamChunk(res.stderr, child.stderr._encoding);
    if (errChunk !== null) {
      child.stderr.emit('data', errChunk);
    }

    child.stdout.emit('end');
    child.stderr.emit('end');
    child.stdout.emit('close');
    child.stderr.emit('close');
    child.emit('exit', child.exitCode, child.signalCode);
    child.emit('close', child.exitCode, child.signalCode);
  });

  return child;
}

function handleExecLikeResult(child, command, res, opts, cb) {
  if (res && typeof res === 'object' && res.error) {
    const err = toRpcError(res.error);
    err.cmd = command;
    err.killed = Boolean(res.killed);
    err.signal = res.signal !== undefined ? res.signal : null;
    err.stdout = formatTextOutput(res.stdout, opts.encoding);
    err.stderr = formatTextOutput(res.stderr, opts.encoding);
    child.emit('close', null, null);
    if (cb) {
      cb(err, err.stdout, err.stderr);
    } else {
      child.emit('error', err);
    }
    return;
  }

  const stdout = formatTextOutput(res ? res.stdout : '', opts.encoding);
  const stderr = formatTextOutput(res ? res.stderr : '', opts.encoding);
  const exitCode = res
    ? (res.exitCode !== undefined ? res.exitCode : (res.status !== undefined ? res.status : 0))
    : 0;
  const signal = res && res.signal !== undefined ? res.signal : null;
  const killed = Boolean(res && (res.killed || res.timedOut));

  child.pid = res && typeof res.pid === 'number' ? res.pid : 0;
  child.exitCode = exitCode;
  child.signalCode = signal;
  if (killed) {
    child.killed = true;
  }

  child.emit('spawn');
  const outChunk = toStreamChunk(res ? res.stdout : null, child.stdout._encoding);
  if (outChunk !== null) {
    child.stdout.emit('data', outChunk);
  }
  const errChunk = toStreamChunk(res ? res.stderr : null, child.stderr._encoding);
  if (errChunk !== null) {
    child.stderr.emit('data', errChunk);
  }
  child.stdout.emit('end');
  child.stderr.emit('end');
  child.stdout.emit('close');
  child.stderr.emit('close');
  child.emit('exit', exitCode, signal);
  child.emit('close', exitCode, signal);

  if ((exitCode !== 0 && exitCode !== null) || killed) {
    const errText = typeof stderr === 'string' && stderr ? `\n${stderr}` : '';
    const err = new Error(`Command failed: ${command}${errText}`);
    err.code = exitCode !== 0 && exitCode !== null ? exitCode : null;
    err.killed = killed;
    err.signal = signal;
    err.cmd = command;
    err.stdout = stdout;
    err.stderr = stderr;
    if (cb) {
      cb(err, stdout, stderr);
    }
    return;
  }

  if (cb) {
    cb(null, stdout, stderr);
  }
}

function exec(command, options, callback) {
  const { opts, cb } = parseExecArgs(options, callback);
  const child = new ChildProcess(typeof command === 'string' ? command : '', []);

  Promise.resolve().then(async () => {
    if (typeof command !== 'string' || command.length === 0) {
      const err = new Error('EINVAL: command must be a non-empty string');
      err.code = 'EINVAL';
      err.cmd = command;
      err.stdout = '';
      err.stderr = '';
      child.emit('close', null, null);
      if (cb) {
        cb(err, '', '');
      } else {
        child.emit('error', err);
      }
      return;
    }

    let res;
    try {
      res = await rpc.call('process.exec', buildExecPayload(command, opts));
    } catch (rpcErr) {
      const err = toRpcError(rpcErr);
      err.cmd = command;
      err.stdout = '';
      err.stderr = '';
      child.emit('close', null, null);
      if (cb) {
        cb(err, '', '');
      } else {
        child.emit('error', err);
      }
      return;
    }

    handleExecLikeResult(child, command, res, opts, cb);
  });

  return child;
}

exec[customPromisifySymbol] = function execPromisified(command, options) {
  return new Promise((resolve, reject) => {
    exec(command, options, (err, stdout, stderr) => {
      if (err) {
        reject(err);
      } else {
        resolve({ stdout, stderr });
      }
    });
  });
};

function execFile(file, args, options, callback) {
  const { argList, opts, cb } = parseExecFileArgs(args, options, callback);
  const child = new ChildProcess(typeof file === 'string' ? file : '', argList);

  Promise.resolve().then(async () => {
    if (typeof file !== 'string' || file.length === 0) {
      const err = new Error('EINVAL: file must be a non-empty string');
      err.code = 'EINVAL';
      err.cmd = file;
      err.stdout = '';
      err.stderr = '';
      child.emit('close', null, null);
      if (cb) {
        cb(err, '', '');
      } else {
        child.emit('error', err);
      }
      return;
    }

    let res;
    try {
      res = await rpc.call('process.spawn', buildSpawnPayload(file, argList, opts));
    } catch (rpcErr) {
      const err = toRpcError(rpcErr);
      err.cmd = file;
      err.stdout = '';
      err.stderr = '';
      child.emit('close', null, null);
      if (cb) {
        cb(err, '', '');
      } else {
        child.emit('error', err);
      }
      return;
    }

    handleExecLikeResult(child, file, res, opts, cb);
  });

  return child;
}

execFile[customPromisifySymbol] = function execFilePromisified(file, args, options) {
  return new Promise((resolve, reject) => {
    execFile(file, args, options, (err, stdout, stderr) => {
      if (err) {
        reject(err);
      } else {
        resolve({ stdout, stderr });
      }
    });
  });
};

function spawnSync(command, args, options) {
  assertSyncSupported('child_process.spawnSync');
  const { argList, opts } = parseSpawnArgs(args, options);
  if (typeof command !== 'string' || command.length === 0) {
    const err = new Error('EINVAL: command must be a non-empty string');
    err.code = 'EINVAL';
    return {
      pid: 0,
      output: [null, '', ''],
      stdout: '',
      stderr: '',
      status: null,
      signal: null,
      error: err,
    };
  }

  let res;
  try {
    res = rpc.callSync('process.spawnSync', buildSpawnPayload(command, argList, opts));
  } catch (rpcErr) {
    const err = toRpcError(rpcErr);
    return {
      pid: 0,
      output: [null, '', ''],
      stdout: '',
      stderr: '',
      status: null,
      signal: null,
      error: err,
    };
  }

  if (res === null || res === undefined) {
    const emptyOut = formatTextOutput('', opts.encoding);
    return {
      pid: 0,
      output: [null, emptyOut, emptyOut],
      stdout: emptyOut,
      stderr: emptyOut,
      status: null,
      signal: null,
      error: null,
    };
  }

  if (res && typeof res === 'object' && res.error) {
    const err = toRpcError(res.error);
    const stdout = formatTextOutput(res.stdout, opts.encoding);
    const stderr = formatTextOutput(res.stderr, opts.encoding);
    return {
      pid: typeof res.pid === 'number' ? res.pid : 0,
      output: [null, stdout, stderr],
      stdout,
      stderr,
      status: null,
      signal: res.signal !== undefined ? res.signal : null,
      error: err,
    };
  }

  const stdout = formatTextOutput(res.stdout, opts.encoding);
  const stderr = formatTextOutput(res.stderr, opts.encoding);
  const status =
    res.status !== undefined
      ? res.status
      : (res.exitCode !== undefined ? res.exitCode : 0);
  const signal = res.signal !== undefined ? res.signal : null;
  return {
    pid: typeof res.pid === 'number' ? res.pid : 0,
    output: [null, stdout, stderr],
    stdout,
    stderr,
    status,
    signal,
    error: null,
  };
}

function execSync(command, options) {
  assertSyncSupported('child_process.execSync');
  const opts = options && typeof options === 'object' ? options : {};
  if (typeof command !== 'string' || command.length === 0) {
    const err = new Error('EINVAL: command must be a non-empty string');
    err.code = 'EINVAL';
    throw err;
  }

  const res = rpc.callSync('process.exec', buildExecPayload(command, opts));
  if (res && typeof res === 'object' && res.error) {
    throw toRpcError(res.error);
  }
  if (res === null || res === undefined) {
    return opts.encoding && opts.encoding !== 'buffer' ? '' : Buffer.alloc(0);
  }

  const exitCode =
    res.exitCode !== undefined
      ? res.exitCode
      : (res.status !== undefined ? res.status : 0);
  const encoding = opts.encoding || 'buffer';
  const stdout = formatTextOutput(res.stdout, encoding);
  const stderr = formatTextOutput(res.stderr, encoding);

  if ((exitCode !== 0 && exitCode !== null) || res.killed || res.timedOut) {
    const err = new Error(`Command failed: ${command}`);
    err.status = exitCode;
    err.signal = res.signal !== undefined ? res.signal : null;
    err.output = [null, stdout, stderr];
    err.stdout = stdout;
    err.stderr = stderr;
    throw err;
  }

  return stdout;
}

module.exports = {
  ChildProcess,
  spawn,
  exec,
  execFile,
  spawnSync,
  execSync,
};
