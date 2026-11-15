// Node.js fs shim wrapping Rust ops / Deno ops

const rpc = require('../rpc-bridge');

const fs = {
  promises: {
    async readFile(filePath, encoding = 'utf8') {
      return await rpc.call('fs.readFile', { path: filePath, encoding });
    },

    async writeFile(filePath, data) {
      return await rpc.call('fs.writeFile', { path: filePath, data: String(data) });
    },

    async readdir(dirPath) {
      return await rpc.call('fs.readdir', { path: dirPath });
    },

    async stat(targetPath) {
      return await rpc.call('fs.stat', { path: targetPath });
    },

    async mkdir(dirPath, options = {}) {
      return await rpc.call('fs.mkdir', { path: dirPath, options });
    },

    async unlink(filePath) {
      return await rpc.call('fs.unlink', { path: filePath });
    }
  },

  readFile(filePath, options, callback) {
    const cb = typeof options === 'function' ? options : callback;
    const enc = typeof options === 'string' ? options : (options && options.encoding) || 'utf8';
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
