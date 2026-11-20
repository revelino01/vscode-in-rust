// Node.js path shim (cross-platform OS-aware POSIX & Windows path support)

const isWindowsHost =
  (typeof process !== 'undefined' && process.platform === 'win32') ||
  (typeof Deno !== 'undefined' && Deno.build && Deno.build.os === 'windows');

function createPathFlavor(isWin32) {
  const sep = isWin32 ? '\\' : '/';
  const delimiter = isWin32 ? ';' : ':';

  function isWinSep(ch) {
    return ch === '\\' || ch === '/';
  }

  function isDriveLetter(ch) {
    return (ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z');
  }

  function findWinRootEnd(p) {
    if (!p || p.length === 0) {
      return 0;
    }
    if (isWinSep(p[0])) {
      let rootEnd = 1;
      if (p.length > 1 && isWinSep(p[1])) {
        let j = 2;
        let last = j;
        while (j < p.length && !isWinSep(p[j])) {
          j++;
        }
        if (j < p.length && j !== last) {
          last = j;
          while (j < p.length && isWinSep(p[j])) {
            j++;
          }
          if (j < p.length && j !== last) {
            last = j;
            while (j < p.length && !isWinSep(p[j])) {
              j++;
            }
            if (j === p.length) {
              return j;
            }
            if (j !== last) {
              return j + 1;
            }
          }
        }
      }
      return rootEnd;
    }
    if (p.length >= 2 && isDriveLetter(p[0]) && p[1] === ':') {
      return p.length > 2 && isWinSep(p[2]) ? 3 : 2;
    }
    return 0;
  }

  function splitWinDevice(p) {
    // UNC: \\server\share or //server/share
    if (p.length >= 2 && isWinSep(p[0]) && isWinSep(p[1])) {
      const norm = p.replace(/\//g, '\\');
      const rest = norm.slice(2);
      const firstSep = rest.indexOf('\\');
      if (firstSep > 0) {
        const afterServer = rest.slice(firstSep + 1);
        const secondSep = afterServer.indexOf('\\');
        if (secondSep > 0) {
          const rootEnd = 2 + firstSep + 1 + secondSep;
          return {
            device: norm.slice(0, rootEnd),
            isAbs: true,
            tail: norm.slice(rootEnd),
          };
        }
        return {
          device: norm,
          isAbs: true,
          tail: '',
        };
      }
    }
    // Drive letter: C:\... or C:...
    if (p.length >= 2 && /^[A-Za-z]:/.test(p)) {
      const device = p.slice(0, 2);
      const after = p.slice(2);
      const isAbs = after.startsWith('\\') || after.startsWith('/');
      return {
        device,
        isAbs,
        tail: after,
      };
    }
    const isAbs = p.startsWith('\\') || p.startsWith('/');
    return { device: '', isAbs, tail: p };
  }

  function normalizeSegments(segments, allowAboveRoot) {
    const res = [];
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      if (!seg || seg === '.') {
        continue;
      }
      if (seg === '..') {
        if (res.length > 0 && res[res.length - 1] !== '..') {
          res.pop();
        } else if (allowAboveRoot) {
          res.push('..');
        }
      } else {
        res.push(seg);
      }
    }
    return res;
  }

  const impl = {
    sep,
    delimiter,

    isAbsolute(p) {
      if (typeof p !== 'string' || p.length === 0) {
        return false;
      }
      if (!isWin32) {
        return p.startsWith('/');
      }
      if (p.startsWith('\\') || p.startsWith('/')) {
        return true;
      }
      return /^[A-Za-z]:[\\/]/.test(p);
    },

    normalize(p) {
      if (!p || typeof p !== 'string') {
        return '.';
      }
      if (!isWin32) {
        const isAbs = p.startsWith('/');
        const trailing = p.length > 1 && p.endsWith('/');
        const segs = normalizeSegments(p.split('/'), !isAbs);
        let out = (isAbs ? '/' : '') + segs.join('/');
        if (!out) {
          out = isAbs ? '/' : '.';
        }
        if (trailing && !out.endsWith('/')) {
          out += '/';
        }
        return out;
      }

      const { device, isAbs, tail } = splitWinDevice(p);
      const normTail = tail.replace(/\//g, '\\');
      const trailing = normTail.length > 1 && normTail.endsWith('\\');
      const segs = normalizeSegments(normTail.split('\\'), !isAbs);
      const isUnc = device.startsWith('\\\\');

      let root = device;
      if (isUnc) {
        root = device + '\\';
        const body = segs.join('\\');
        let out = body ? root + body : root;
        if (trailing && !out.endsWith('\\')) {
          out += '\\';
        }
        return out;
      }

      const prefix = device + (isAbs ? '\\' : '');
      let out = prefix + segs.join('\\');
      if (!out) {
        out = isAbs ? '\\' : '.';
      } else if (device && !isAbs && segs.length === 0) {
        out = device + '.';
      }
      if (trailing && !out.endsWith('\\')) {
        out += '\\';
      }
      return out;
    },

    join(...parts) {
      const filtered = parts.filter((part) => typeof part === 'string' && part.length > 0);
      if (filtered.length === 0) {
        return '.';
      }
      return impl.normalize(filtered.join(sep));
    },

    resolve(...parts) {
      if (!isWin32) {
        let resolvedPath = '';
        let resolvedAbsolute = false;
        for (let i = parts.length - 1; i >= -1 && !resolvedAbsolute; i--) {
          let part;
          if (i >= 0) {
            part = parts[i];
          } else {
            part = (!isWindowsHost && typeof process !== 'undefined' && typeof process.cwd === 'function')
              ? process.cwd()
              : '/';
          }
          if (!part || typeof part !== 'string') {
            continue;
          }
          resolvedPath = resolvedPath ? `${part}/${resolvedPath}` : part;
          resolvedAbsolute = part.startsWith('/');
        }
        const segs = normalizeSegments(resolvedPath.split('/'), !resolvedAbsolute);
        return '/' + segs.join('/');
      }

      let resolvedDevice = '';
      let resolvedTail = '';
      let resolvedAbsolute = false;

      for (let i = parts.length - 1; i >= -1; i--) {
        let pathArg;
        if (i >= 0) {
          pathArg = parts[i];
        } else if (!resolvedDevice) {
          pathArg = (isWindowsHost && typeof process !== 'undefined' && typeof process.cwd === 'function')
            ? process.cwd()
            : 'C:\\';
        } else {
          let fallback = null;
          if (isWindowsHost && typeof process !== 'undefined') {
            if (process.env) {
              fallback = process.env['=' + resolvedDevice] || process.env['=' + resolvedDevice.toUpperCase()] || null;
            }
            if (!fallback && typeof process.cwd === 'function') {
              const cwd = process.cwd();
              const cwdDev = splitWinDevice(cwd).device;
              if (cwdDev && cwdDev.toLowerCase() === resolvedDevice.toLowerCase()) {
                fallback = cwd;
              }
            }
          }
          pathArg = fallback || (resolvedDevice + '\\');
        }
        if (!pathArg || typeof pathArg !== 'string') {
          continue;
        }
        const { device, isAbs, tail } = splitWinDevice(pathArg);
        if (device && resolvedDevice && device.toLowerCase() !== resolvedDevice.toLowerCase()) {
          continue;
        }
        if (!resolvedDevice && device) {
          resolvedDevice = device;
        }
        if (!resolvedAbsolute) {
          resolvedTail = resolvedTail ? `${tail}\\${resolvedTail}` : tail;
          resolvedAbsolute = isAbs;
        }
        if (resolvedDevice && resolvedAbsolute) {
          break;
        }
      }

      const normTail = resolvedTail.replace(/\//g, '\\');
      const segs = normalizeSegments(normTail.split('\\'), !resolvedAbsolute);
      if (resolvedDevice.startsWith('\\\\')) {
        return segs.length > 0 ? `${resolvedDevice}\\${segs.join('\\')}` : `${resolvedDevice}\\`;
      }
      if (resolvedDevice) {
        return `${resolvedDevice}\\${segs.join('\\')}`;
      }
      return `\\${segs.join('\\')}`;
    },

    dirname(p) {
      if (!p || typeof p !== 'string') {
        return '.';
      }
      const rootEnd = isWin32 ? findWinRootEnd(p) : (p.startsWith('/') ? 1 : 0);
      let end = -1;
      let matchedSlash = true;
      for (let i = p.length - 1; i >= rootEnd; --i) {
        const isSep = isWin32 ? isWinSep(p[i]) : p[i] === '/';
        if (isSep) {
          if (!matchedSlash) {
            end = i;
            break;
          }
        } else {
          matchedSlash = false;
        }
      }
      if (end === -1) {
        return rootEnd === 0 ? '.' : p.slice(0, rootEnd);
      }
      return p.slice(0, end);
    },

    basename(p, ext) {
      if (!p || typeof p !== 'string') {
        return '';
      }
      let start = 0;
      if (isWin32 && p.length >= 2 && isDriveLetter(p[0]) && p[1] === ':') {
        start = 2;
      }
      let end = -1;
      let firstNonSlash = -1;
      for (let i = p.length - 1; i >= start; --i) {
        const isSep = isWin32 ? isWinSep(p[i]) : p[i] === '/';
        if (isSep) {
          if (firstNonSlash !== -1) {
            end = i + 1;
            break;
          }
        } else if (firstNonSlash === -1) {
          firstNonSlash = i + 1;
        }
      }
      if (firstNonSlash === -1) {
        return '';
      }
      let base = end === -1 ? p.slice(start, firstNonSlash) : p.slice(end, firstNonSlash);
      if (ext && base.endsWith(ext)) {
        base = base.slice(0, -ext.length);
      }
      return base;
    },

    extname(p) {
      const base = impl.basename(p);
      if (base === '.' || base === '..') {
        return '';
      }
      const idx = base.lastIndexOf('.');
      if (idx <= 0) {
        return '';
      }
      return base.slice(idx);
    },

    relative(from, to) {
      if (from === to) {
        return '';
      }
      const fromResolved = impl.resolve(from);
      const toResolved = impl.resolve(to);
      if (
        (isWin32 && fromResolved.toLowerCase() === toResolved.toLowerCase()) ||
        fromResolved === toResolved
      ) {
        return '';
      }

      if (isWin32) {
        if (fromResolved.startsWith('\\\\') && toResolved.startsWith('\\\\')) {
          const fUnc = fromResolved.slice(2).split('\\').filter(Boolean);
          const tUnc = toResolved.slice(2).split('\\').filter(Boolean);
          if (fUnc.length > 0 && tUnc.length > 0 && fUnc[0].toLowerCase() === tUnc[0].toLowerCase()) {
            const fParts = fUnc.slice(1);
            const tParts = tUnc.slice(1);
            let common = 0;
            const limit = Math.min(fParts.length, tParts.length);
            while (common < limit && fParts[common].toLowerCase() === tParts[common].toLowerCase()) {
              common++;
            }
            const up = [];
            for (let i = common; i < fParts.length; i++) {
              up.push('..');
            }
            return up.concat(tParts.slice(common)).join('\\');
          }
          return toResolved;
        }
        const fDev = splitWinDevice(fromResolved);
        const tDev = splitWinDevice(toResolved);
        if (fDev.device.toLowerCase() !== tDev.device.toLowerCase()) {
          return toResolved;
        }
        const fParts = fDev.tail.split('\\').filter(Boolean);
        const tParts = tDev.tail.split('\\').filter(Boolean);
        let common = 0;
        const limit = Math.min(fParts.length, tParts.length);
        while (common < limit && fParts[common].toLowerCase() === tParts[common].toLowerCase()) {
          common++;
        }
        const up = [];
        for (let i = common; i < fParts.length; i++) {
          up.push('..');
        }
        return up.concat(tParts.slice(common)).join('\\');
      }

      const fParts = fromResolved.split('/').filter(Boolean);
      const tParts = toResolved.split('/').filter(Boolean);
      let common = 0;
      const limit = Math.min(fParts.length, tParts.length);
      while (common < limit && fParts[common] === tParts[common]) {
        common++;
      }
      const up = [];
      for (let i = common; i < fParts.length; i++) {
        up.push('..');
      }
      return up.concat(tParts.slice(common)).join('/');
    },

    parse(p) {
      if (typeof p !== 'string' || p.length === 0) {
        return { root: '', dir: '', base: '', ext: '', name: '' };
      }
      const rootEnd = isWin32 ? findWinRootEnd(p) : (p.startsWith('/') ? 1 : 0);
      const root = rootEnd > 0 ? p.slice(0, rootEnd) : '';

      let end = -1;
      let start = -1;
      for (let i = p.length - 1; i >= rootEnd; --i) {
        const isSep = isWin32 ? isWinSep(p[i]) : p[i] === '/';
        if (isSep) {
          if (end !== -1) {
            start = i + 1;
            break;
          }
        } else if (end === -1) {
          end = i + 1;
        }
      }

      if (end === -1) {
        return { root, dir: root, base: '', ext: '', name: '' };
      }
      if (start === -1) {
        start = rootEnd;
      }

      const base = p.slice(start, end);
      const dir = start > rootEnd ? p.slice(0, start - 1) : root;
      let ext = '';
      let name = base;
      if (base !== '.' && base !== '..') {
        const dotIdx = base.lastIndexOf('.');
        if (dotIdx > 0) {
          ext = base.slice(dotIdx);
          name = base.slice(0, dotIdx);
        }
      }
      return { root, dir, base, ext, name };
    },

    format(pathObject) {
      if (!pathObject || typeof pathObject !== 'object') {
        return '';
      }
      const dir = pathObject.dir || pathObject.root || '';
      const base = pathObject.base || `${pathObject.name || ''}${pathObject.ext || ''}`;
      if (!dir) {
        return base;
      }
      if (dir === pathObject.root || dir.endsWith(sep) || (isWin32 && dir.endsWith('/'))) {
        return `${dir}${base}`;
      }
      return `${dir}${sep}${base}`;
    },
  };

  return impl;
}

const posix = createPathFlavor(false);
const win32 = createPathFlavor(true);

posix.posix = posix;
posix.win32 = win32;
win32.posix = posix;
win32.win32 = win32;

const active = isWindowsHost ? win32 : posix;
module.exports = active;
