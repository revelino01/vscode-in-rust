// Node.js path shim (cross-platform POSIX & Windows path support)

const path = {
  sep: '/',
  delimiter: ':',

  normalize(p) {
    if (!p || typeof p !== 'string') return '.';
    const slash = p.replace(/\\/g, '/');
    let prefix = '';
    let rest = slash;

    const uncMatch = rest.match(/^(\/\/[^/]+\/[^/]+)(.*)$/);
    const driveMatch = !uncMatch && rest.match(/^([A-Za-z]:)(.*)$/);
    if (uncMatch) {
      prefix = uncMatch[1];
      rest = uncMatch[2] || '/';
    } else if (driveMatch) {
      prefix = driveMatch[1];
      rest = driveMatch[2];
    }

    const isAbs = Boolean(uncMatch) || rest.startsWith('/');
    const trailingSlash = rest.endsWith('/') && rest.length > 1;
    const segments = rest.split('/').filter(Boolean);
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

    let res = prefix + (isAbs ? '/' : '') + resolved.join('/');
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
      resolvedPath = resolvedPath ? `${part}/${resolvedPath}` : part;
      resolvedAbsolute = this.isAbsolute(part);
    }

    if (!resolvedAbsolute) {
      resolvedPath = `/${resolvedPath}`;
    }

    return this.normalize(resolvedPath);
  },

  dirname(p) {
    const norm = this.normalize(p);
    if (/^[A-Za-z]:\/?$/.test(norm)) {
      return norm.endsWith('/') ? norm : `${norm}/`;
    }
    const trimmed = norm.length > 1 && norm.endsWith('/') ? norm.slice(0, -1) : norm;
    const idx = trimmed.lastIndexOf('/');
    if (idx === -1) return '.';
    if (idx === 0) return '/';
    if (idx === 2 && /^[A-Za-z]:/.test(trimmed)) return trimmed.slice(0, 3);
    return trimmed.slice(0, idx);
  },

  basename(p, ext) {
    if (!p || typeof p !== 'string') return '';
    const cleaned = p.replace(/[\\/]+$/, '');
    const parts = cleaned.split(/[\\/]/).filter(Boolean);
    let base = parts.pop() || '';
    if (/^[A-Za-z]:$/.test(base)) {
      return '';
    }
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
    return (
      typeof p === 'string' &&
      (p.startsWith('/') || p.startsWith('\\') || /^[A-Za-z]:[\\/]/.test(p))
    );
  }
};

module.exports = path;
