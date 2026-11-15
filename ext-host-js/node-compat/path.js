// Node.js path shim (posix style)

const path = {
  sep: '/',
  delimiter: ':',

  normalize(p) {
    if (!p || typeof p !== 'string') return '.';
    const isAbs = p.startsWith('/');
    const trailingSlash = p.endsWith('/') && p.length > 1;
    const segments = p.split('/').filter(Boolean);
    const resolved = [];

    for (const seg of segments) {
      if (seg === '.') continue;
      if (seg === '..') {
        if (resolved.length > 0 && resolved[resolved.length - 1] !== '..') {
          resolved.pop();
        } else if (!isAbs) {
          resolved.push('..');
        }
      } else {
        resolved.push(seg);
      }
    }

    let res = (isAbs ? '/' : '') + resolved.join('/');
    if (!res) res = isAbs ? '/' : '.';
    if (trailingSlash && !res.endsWith('/')) res += '/';
    return res;
  },

  join(...parts) {
    const raw = parts.filter(Boolean).join('/');
    return this.normalize(raw);
  },

  resolve(...parts) {
    let resolvedPath = '';
    let resolvedAbsolute = false;

    for (let i = parts.length - 1; i >= 0 && !resolvedAbsolute; i--) {
      const part = parts[i];
      if (!part) continue;
      resolvedPath = `${part}/${resolvedPath}`;
      resolvedAbsolute = part.startsWith('/');
    }

    if (!resolvedAbsolute) {
      resolvedPath = `/${resolvedPath}`;
    }

    return this.normalize(resolvedPath);
  },

  dirname(p) {
    const norm = this.normalize(p);
    const idx = norm.lastIndexOf('/');
    if (idx === -1) return '.';
    if (idx === 0) return '/';
    return norm.slice(0, idx);
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
    return typeof p === 'string' && p.startsWith('/');
  }
};

module.exports = path;
