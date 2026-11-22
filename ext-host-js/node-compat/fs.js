// Node.js fs shim wrapping Rust ops / Deno ops / live stdio RPC

const rpc = require('../rpc-bridge');
const { base64ToBytes, bytesToBase64, encodeString, decodeBytes } = require('./buffer');

const constants = {
  F_OK: 0,
  X_OK: 1,
  W_OK: 2,
  R_OK: 4,
  COPYFILE_EXCL: 1,
  COPYFILE_FICLONE: 2,
  COPYFILE_FICLONE_FORCE: 4,
};

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

function unwrapResult(res) {
  if (res && typeof res === 'object' && res.error) {
    let message = typeof res.error === 'string' ? res.error : (res.error.message || 'FS error');
    let code = (res.error && typeof res.error === 'object' && res.error.code) ? res.error.code : null;
    if (!code && typeof res.error === 'string') {
      const parsed = parsePrefixedCode(res.error);
      if (parsed) {
        code = parsed.code;
        message = parsed.message;
      }
    }
    const err = new Error(message);
    if (code) {
      err.code = code;
    }
    throw err;
  }
  return res;
}

class Dirent {
  constructor(raw) {
    const obj = raw && typeof raw === 'object' ? raw : { name: String(raw || '') };
    this.name = obj.name !== undefined && obj.name !== null ? String(obj.name) : '';
    this._isFile = Boolean(obj.isFile);
    this._isDirectory = Boolean(obj.isDirectory);
    this._isSymbolicLink = Boolean(obj.isSymbolicLink);
  }

  isFile() {
    return this._isFile;
  }

  isDirectory() {
    return this._isDirectory;
  }

  isSymbolicLink() {
    return this._isSymbolicLink;
  }

  isBlockDevice() {
    return false;
  }

  isCharacterDevice() {
    return false;
  }

  isFIFO() {
    return false;
  }

  isSocket() {
    return false;
  }
}

function wrapStat(raw) {
  if (!raw || typeof raw !== 'object') {
    return raw;
  }
  const isFileVal = Boolean(raw.isFile);
  const isDirVal = Boolean(raw.isDirectory);
  const isSymVal = Boolean(raw.isSymbolicLink);
  return {
    ...raw,
    size: raw.size || 0,
    mtimeMs: raw.mtimeMs || 0,
    isFile: () => isFileVal,
    isDirectory: () => isDirVal,
    isSymbolicLink: () => isSymVal,
  };
}

function normalizeEncodingOption(options, defaultEncoding = 'utf8') {
  if (options === null) {
    return null;
  }
  if (options === undefined) {
    return defaultEncoding;
  }
  if (typeof options === 'string') {
    return normalizeEncodingName(options);
  }
  if (typeof options === 'object') {
    if (options.encoding === null) {
      return null;
    }
    if (options.encoding !== undefined) {
      return normalizeEncodingName(options.encoding);
    }
    return defaultEncoding;
  }
  return defaultEncoding;
}

function normalizeEncodingName(enc) {
  if (enc === null) {
    return null;
  }
  if (typeof enc !== 'string') {
    const err = new Error('Unknown encoding: ' + String(enc));
    err.code = 'EINVAL';
    throw err;
  }
  const lower = enc.toLowerCase();
  switch (lower) {
    case 'utf8':
    case 'utf-8':
      return 'utf8';
    case 'ascii':
      return 'ascii';
    case 'latin1':
    case 'binary':
      return 'latin1';
    case 'base64':
      return 'base64';
    case 'base64url':
      return 'base64url';
    case 'hex':
      return 'hex';
    case 'utf16le':
    case 'utf-16le':
    case 'ucs2':
    case 'ucs-2':
      return 'utf16le';
    case 'buffer':
    case 'bytes':
      return null;
    default: {
      const err = new Error('Unknown encoding: ' + enc);
      err.code = 'EINVAL';
      throw err;
    }
  }
}

function encodeWriteData(data, options) {
  if (data instanceof Uint8Array) {
    return Array.from(data);
  }
  if (Array.isArray(data)) {
    return data;
  }
  if (ArrayBuffer.isView(data)) {
    return Array.from(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  }
  const str = String(data);
  const enc = normalizeEncodingOption(options, 'utf8') || 'utf8';
  if (enc === 'utf8') {
    return str;
  }
  if (typeof Buffer === 'function') {
    return Array.from(Buffer.from(str, enc));
  }
  if (enc === 'base64' || enc === 'base64url') {
    return base64ToBytes(str);
  }
  return encodeString(str, enc);
}

function decodeReadResult(res, enc) {
  if (res === null || res === undefined) {
    return res;
  }
  if (enc === null) {
    if (Array.isArray(res)) {
      return Uint8Array.from(res);
    }
    if (res instanceof Uint8Array) {
      return res;
    }
    return res;
  }
  if (enc === 'utf8') {
    return res;
  }
  const bytes = Array.isArray(res) ? res : (res instanceof Uint8Array ? Array.from(res) : null);
  if (!bytes) {
    return res;
  }
  if (typeof Buffer === 'function') {
    return Buffer.from(bytes).toString(enc);
  }
  if (enc === 'base64') {
    return bytesToBase64(bytes, 0, bytes.length, false);
  }
  if (enc === 'base64url') {
    return bytesToBase64(bytes, 0, bytes.length, true);
  }
  return decodeBytes(bytes, enc, 0, bytes.length);
}

function normalizeMkdirOptions(options) {
  if (typeof options === 'number') {
    return { recursive: false, mode: options };
  }
  if (typeof options === 'boolean') {
    return { recursive: options };
  }
  if (options && typeof options === 'object') {
    return { ...options, recursive: Boolean(options.recursive) };
  }
  return { recursive: false };
}

function normalizeReaddirArgs(dirPath, options) {
  const withFileTypes = Boolean(options && typeof options === 'object' && options.withFileTypes);
  return {
    withFileTypes,
    payload: withFileTypes ? { path: dirPath, withFileTypes: true } : { path: dirPath },
  };
}

function shapeReaddirResult(res, withFileTypes) {
  if (withFileTypes && Array.isArray(res)) {
    return res.map((entry) => (entry instanceof Dirent ? entry : new Dirent(entry)));
  }
  return res;
}

function shapeMkdirResult(res, recursive) {
  if (recursive && res && typeof res === 'object' && typeof res.firstCreated === 'string') {
    return res.firstCreated;
  }
  return undefined;
}

function assertSyncSupported(methodName) {
  if (
    typeof rpc.isLiveStdioMode === 'function' &&
    rpc.isLiveStdioMode() &&
    !(typeof rpc.hasSyncTransport === 'function' && rpc.hasSyncTransport())
  ) {
    const err = new Error(
      `ENOSYS: ${methodName} is unsupported over async Node.js stdio transport; use fs.promises.* instead`
    );
    err.code = 'ENOSYS';
    throw err;
  }
}

function invokeCallback(cb, promise) {
  if (typeof cb !== 'function') {
    promise.catch(() => {});
    return;
  }
  promise.then(
    (res) => cb(null, res),
    (err) => cb(err)
  );
}

const promises = {
  constants,

  async readFile(filePath, options = 'utf8') {
    const enc = normalizeEncodingOption(options, 'utf8');
    const rpcEnc = enc === null ? null : (enc === 'utf8' ? 'utf8' : null);
    const res = unwrapResult(await rpc.call('fs.readFile', { path: filePath, encoding: rpcEnc }));
    return decodeReadResult(res, enc);
  },

  async writeFile(filePath, data, options) {
    const payloadData = encodeWriteData(data, options);
    unwrapResult(await rpc.call('fs.writeFile', { path: filePath, data: payloadData }));
  },

  async readdir(dirPath, options) {
    const { withFileTypes, payload } = normalizeReaddirArgs(dirPath, options);
    const res = unwrapResult(await rpc.call('fs.readdir', payload));
    return shapeReaddirResult(res, withFileTypes);
  },

  async stat(targetPath) {
    const raw = unwrapResult(await rpc.call('fs.stat', { path: targetPath }));
    return wrapStat(raw);
  },

  async mkdir(dirPath, options = {}) {
    const norm = normalizeMkdirOptions(options);
    const res = unwrapResult(await rpc.call('fs.mkdir', { path: dirPath, options: norm }));
    return shapeMkdirResult(res, norm.recursive);
  },

  async unlink(filePath) {
    unwrapResult(await rpc.call('fs.unlink', { path: filePath }));
  },

  async access(targetPath, mode = constants.F_OK) {
    unwrapResult(await rpc.call('fs.access', { path: targetPath, mode: Number(mode) || 0 }));
  },

  async rm(targetPath, options = {}) {
    const opts = options && typeof options === 'object' ? options : {};
    unwrapResult(
      await rpc.call('fs.rm', {
        path: targetPath,
        options: {
          recursive: Boolean(opts.recursive),
          force: Boolean(opts.force),
        },
      })
    );
  },

  async rename(oldPath, newPath) {
    unwrapResult(await rpc.call('fs.rename', { oldPath, newPath }));
  },

  async copyFile(src, dest, mode = 0) {
    unwrapResult(await rpc.call('fs.copyFile', { src, dest, mode: Number(mode) || 0 }));
  },
};

const fs = {
  constants,
  Dirent,
  promises,

  readFile(filePath, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    let enc = 'utf8';
    if (options === null) {
      enc = null;
    } else if (typeof options === 'string') {
      enc = options;
    } else if (options && typeof options === 'object' && options.encoding !== undefined) {
      enc = options.encoding;
    }
    invokeCallback(cb, promises.readFile(filePath, enc));
  },

  writeFile(filePath, data, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    const opts = typeof options === 'function' ? undefined : options;
    invokeCallback(cb, promises.writeFile(filePath, data, opts));
  },

  readdir(dirPath, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    const opts = typeof options === 'function' ? undefined : options;
    invokeCallback(cb, promises.readdir(dirPath, opts));
  },

  stat(targetPath, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    invokeCallback(cb, promises.stat(targetPath));
  },

  mkdir(dirPath, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    const opts = typeof options === 'function' ? {} : options;
    invokeCallback(cb, promises.mkdir(dirPath, opts));
  },

  unlink(filePath, callback) {
    invokeCallback(callback, promises.unlink(filePath));
  },

  access(targetPath, mode, callback) {
    const cb = typeof mode === 'function' ? mode : callback;
    const resolvedMode = typeof mode === 'function' ? constants.F_OK : mode;
    invokeCallback(cb, promises.access(targetPath, resolvedMode));
  },

  rm(targetPath, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    const opts = typeof options === 'function' ? {} : options;
    invokeCallback(cb, promises.rm(targetPath, opts));
  },

  rename(oldPath, newPath, callback) {
    invokeCallback(callback, promises.rename(oldPath, newPath));
  },

  copyFile(src, dest, mode, callback) {
    const cb = typeof mode === 'function' ? mode : callback;
    const resolvedMode = typeof mode === 'function' ? 0 : mode;
    invokeCallback(cb, promises.copyFile(src, dest, resolvedMode));
  },

  existsSync(targetPath) {
    if (
      typeof rpc.isLiveStdioMode === 'function' &&
      rpc.isLiveStdioMode() &&
      !(typeof rpc.hasSyncTransport === 'function' && rpc.hasSyncTransport())
    ) {
      return false;
    }
    try {
      const res = unwrapResult(rpc.callSync('fs.stat', { path: targetPath }));
      return Boolean(res && typeof res === 'object');
    } catch (_) {
      return false;
    }
  },

  readFileSync(filePath, options = 'utf8') {
    assertSyncSupported('fs.readFileSync');
    const enc = normalizeEncodingOption(options, 'utf8');
    const rpcEnc = enc === null ? null : (enc === 'utf8' ? 'utf8' : null);
    const res = unwrapResult(rpc.callSync('fs.readFile', { path: filePath, encoding: rpcEnc }));
    return decodeReadResult(res, enc);
  },

  writeFileSync(filePath, data, options) {
    assertSyncSupported('fs.writeFileSync');
    const payloadData = encodeWriteData(data, options);
    unwrapResult(rpc.callSync('fs.writeFile', { path: filePath, data: payloadData }));
  },

  readdirSync(dirPath, options) {
    assertSyncSupported('fs.readdirSync');
    const { withFileTypes, payload } = normalizeReaddirArgs(dirPath, options);
    const res = unwrapResult(rpc.callSync('fs.readdir', payload));
    return shapeReaddirResult(res, withFileTypes);
  },

  statSync(targetPath) {
    assertSyncSupported('fs.statSync');
    const raw = unwrapResult(rpc.callSync('fs.stat', { path: targetPath }));
    return wrapStat(raw);
  },

  mkdirSync(dirPath, options = {}) {
    assertSyncSupported('fs.mkdirSync');
    const norm = normalizeMkdirOptions(options);
    const res = unwrapResult(rpc.callSync('fs.mkdir', { path: dirPath, options: norm }));
    return shapeMkdirResult(res, norm.recursive);
  },

  unlinkSync(filePath) {
    assertSyncSupported('fs.unlinkSync');
    unwrapResult(rpc.callSync('fs.unlink', { path: filePath }));
  },

  accessSync(targetPath, mode = constants.F_OK) {
    assertSyncSupported('fs.accessSync');
    unwrapResult(rpc.callSync('fs.access', { path: targetPath, mode: Number(mode) || 0 }));
  },

  rmSync(targetPath, options = {}) {
    assertSyncSupported('fs.rmSync');
    const opts = options && typeof options === 'object' ? options : {};
    unwrapResult(
      rpc.callSync('fs.rm', {
        path: targetPath,
        options: {
          recursive: Boolean(opts.recursive),
          force: Boolean(opts.force),
        },
      })
    );
  },

  renameSync(oldPath, newPath) {
    assertSyncSupported('fs.renameSync');
    unwrapResult(rpc.callSync('fs.rename', { oldPath, newPath }));
  },

  copyFileSync(src, dest, mode = 0) {
    assertSyncSupported('fs.copyFileSync');
    unwrapResult(rpc.callSync('fs.copyFile', { src, dest, mode: Number(mode) || 0 }));
  },
};

module.exports = fs;
