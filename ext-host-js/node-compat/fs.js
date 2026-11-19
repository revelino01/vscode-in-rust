// Node.js fs shim wrapping Rust ops / Deno ops

const rpc = require('../rpc-bridge');

function unwrapResult(res) {
  if (res && typeof res === 'object' && res.error) {
    const message = typeof res.error === 'string' ? res.error : (res.error.message || 'FS error');
    const err = new Error(message);
    if (res.error && typeof res.error === 'object' && res.error.code) {
      err.code = res.error.code;
    }
    throw err;
  }
  return res;
}

const fs = {
  promises: {
    async readFile(filePath, encoding = 'utf8') {
      const enc = encoding && typeof encoding === 'object' ? encoding.encoding : encoding;
      const res = unwrapResult(await rpc.call('fs.readFile', { path: filePath, encoding: enc }));
      if (enc === null && Array.isArray(res)) {
        return Uint8Array.from(res);
      }
      return res;
    },

    async writeFile(filePath, data) {
      const payloadData = Array.isArray(data)
        ? data
        : (data instanceof Uint8Array ? Array.from(data) : String(data));
      unwrapResult(await rpc.call('fs.writeFile', { path: filePath, data: payloadData }));
    },

    async readdir(dirPath) {
      return unwrapResult(await rpc.call('fs.readdir', { path: dirPath }));
    },

    async stat(targetPath) {
      const raw = unwrapResult(await rpc.call('fs.stat', { path: targetPath }));
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
    },

    async mkdir(dirPath, options = {}) {
      return unwrapResult(await rpc.call('fs.mkdir', { path: dirPath, options }));
    },

    async unlink(filePath) {
      return unwrapResult(await rpc.call('fs.unlink', { path: filePath }));
    }
  },

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
    fs.promises.readFile(filePath, enc)
      .then(res => cb(null, res))
      .catch(err => cb(err));
  },

  writeFile(filePath, data, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    fs.promises.writeFile(filePath, data)
      .then(() => cb(null))
      .catch(err => cb(err));
  }
};

module.exports = fs;
