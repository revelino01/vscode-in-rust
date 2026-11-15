// Node.js path shim (posix style)

const path = {
  sep: '/',
  delimiter: ':',

  join(...parts) {
    return parts
      .filter(Boolean)
      .join('/')
      .replace(/\/+/g, '/');
  },

  resolve(...parts) {
    return this.join(...parts);
  },

  dirname(p) {
    const idx = p.lastIndexOf('/');
    if (idx === -1) return '.';
    if (idx === 0) return '/';
    return p.slice(0, idx);
  },

  basename(p, ext) {
    let base = p.split('/').filter(Boolean).pop() || '';
    if (ext && base.endsWith(ext)) {
      base = base.slice(0, -ext.length);
    }
    return base;
  },

  extname(p) {
    const base = this.basename(p);
    const idx = base.lastIndexOf('.');
    if (idx <= 0) return '';
    return base.slice(idx);
  },

  isAbsolute(p) {
    return p.startsWith('/');
  }
};

module.exports = path;
