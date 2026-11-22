'use strict';

// Pure-JS Node.js util compatibility shim (no dependency on Node built-in 'util').

const { utf8ToBytes, utf8EncodeInto, bytesToUtf8 } = require('./buffer');

const customInspectSymbol = Symbol.for('nodejs.util.inspect.custom');
const customPromisifySymbol = Symbol.for('nodejs.util.promisify.custom');
const customPromisifyArgsSymbol = Symbol.for('nodejs.util.promisify.customArgs');

function getTag(val) {
  return Object.prototype.toString.call(val);
}

const types = {
  isAnyArrayBuffer(val) {
    const t = getTag(val);
    return t === '[object ArrayBuffer]' || t === '[object SharedArrayBuffer]';
  },
  isArrayBuffer(val) {
    return getTag(val) === '[object ArrayBuffer]';
  },
  isSharedArrayBuffer(val) {
    return getTag(val) === '[object SharedArrayBuffer]';
  },
  isArrayBufferView(val) {
    return ArrayBuffer.isView(val);
  },
  isTypedArray(val) {
    return ArrayBuffer.isView(val) && !(val instanceof DataView);
  },
  isUint8Array(val) {
    return getTag(val) === '[object Uint8Array]';
  },
  isUint8ClampedArray(val) {
    return getTag(val) === '[object Uint8ClampedArray]';
  },
  isInt8Array(val) {
    return getTag(val) === '[object Int8Array]';
  },
  isUint16Array(val) {
    return getTag(val) === '[object Uint16Array]';
  },
  isInt16Array(val) {
    return getTag(val) === '[object Int16Array]';
  },
  isUint32Array(val) {
    return getTag(val) === '[object Uint32Array]';
  },
  isInt32Array(val) {
    return getTag(val) === '[object Int32Array]';
  },
  isFloat32Array(val) {
    return getTag(val) === '[object Float32Array]';
  },
  isFloat64Array(val) {
    return getTag(val) === '[object Float64Array]';
  },
  isBigInt64Array(val) {
    return getTag(val) === '[object BigInt64Array]';
  },
  isBigUint64Array(val) {
    return getTag(val) === '[object BigUint64Array]';
  },
  isDataView(val) {
    return getTag(val) === '[object DataView]';
  },
  isDate(val) {
    return getTag(val) === '[object Date]';
  },
  isRegExp(val) {
    return getTag(val) === '[object RegExp]';
  },
  isPromise(val) {
    return getTag(val) === '[object Promise]';
  },
  isMap(val) {
    return getTag(val) === '[object Map]';
  },
  isSet(val) {
    return getTag(val) === '[object Set]';
  },
  isWeakMap(val) {
    return getTag(val) === '[object WeakMap]';
  },
  isWeakSet(val) {
    return getTag(val) === '[object WeakSet]';
  },
  isMapIterator(val) {
    return getTag(val) === '[object Map Iterator]';
  },
  isSetIterator(val) {
    return getTag(val) === '[object Set Iterator]';
  },
  isArgumentsObject(val) {
    return getTag(val) === '[object Arguments]';
  },
  isNativeError(val) {
    return val instanceof Error || getTag(val) === '[object Error]';
  },
  isBoxedPrimitive(val) {
    const t = getTag(val);
    return (
      typeof val === 'object' &&
      val !== null &&
      (t === '[object Number]' ||
        t === '[object String]' ||
        t === '[object Boolean]' ||
        t === '[object Symbol]' ||
        t === '[object BigInt]')
    );
  },
  isBooleanObject(val) {
    return typeof val === 'object' && val !== null && getTag(val) === '[object Boolean]';
  },
  isNumberObject(val) {
    return typeof val === 'object' && val !== null && getTag(val) === '[object Number]';
  },
  isStringObject(val) {
    return typeof val === 'object' && val !== null && getTag(val) === '[object String]';
  },
  isSymbolObject(val) {
    return typeof val === 'object' && val !== null && getTag(val) === '[object Symbol]';
  },
  isBigIntObject(val) {
    return typeof val === 'object' && val !== null && getTag(val) === '[object BigInt]';
  },
  isAsyncFunction(val) {
    return typeof val === 'function' && getTag(val) === '[object AsyncFunction]';
  },
  isGeneratorFunction(val) {
    return typeof val === 'function' && getTag(val) === '[object GeneratorFunction]';
  },
  isGeneratorObject(val) {
    return getTag(val) === '[object Generator]';
  },
  isProxy() {
    return false;
  },
};

function safeJsonStringify(val) {
  const seen = new Set();
  return JSON.stringify(val, (_key, value) => {
    if (typeof value === 'bigint') {
      return value.toString();
    }
    if (typeof value === 'object' && value !== null) {
      if (seen.has(value)) {
        return '[Circular]';
      }
      seen.add(value);
    }
    return value;
  });
}

function formatKey(key) {
  if (typeof key === 'symbol') {
    return `[${String(key)}]`;
  }
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(key) ? key : JSON.stringify(key);
}

function inspectValue(val, opts, depth, ancestors) {
  if (val === null) return 'null';
  if (val === undefined) return 'undefined';
  const t = typeof val;
  if (t === 'boolean' || t === 'number') {
    return Object.is(val, -0) ? '-0' : String(val);
  }
  if (t === 'bigint') return `${val}n`;
  if (t === 'symbol') return String(val);
  if (t === 'string') {
    const maxLen = opts.maxStringLength;
    const str = maxLen >= 0 && val.length > maxLen
      ? val.slice(0, maxLen) + `... ${val.length - maxLen} more characters`
      : val;
    return `'${str.replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t')}'`;
  }
  if (t === 'function') {
    const name = val.name ? `: ${val.name}` : ' (anonymous)';
    const prefix = types.isAsyncFunction(val)
      ? 'AsyncFunction'
      : types.isGeneratorFunction(val)
        ? 'GeneratorFunction'
        : 'Function';
    return `[${prefix}${name}]`;
  }

  // Custom inspect hook
  if (typeof val[customInspectSymbol] === 'function') {
    try {
      const customOut = val[customInspectSymbol](opts.depth - depth, opts, inspect);
      if (typeof customOut === 'string') {
        return customOut;
      }
    } catch (_) {}
  }

  if (types.isDate(val)) {
    return Number.isNaN(val.getTime()) ? 'Invalid Date' : val.toISOString();
  }
  if (types.isRegExp(val)) {
    return val.toString();
  }
  if (types.isPromise(val)) {
    return 'Promise { <pending> }';
  }
  if (types.isWeakMap(val)) {
    return 'WeakMap { <items unknown> }';
  }
  if (types.isWeakSet(val)) {
    return 'WeakSet { <items unknown> }';
  }
  if (types.isNativeError(val)) {
    return val.stack || `${val.name}: ${val.message}`;
  }

  // Circular reference detection
  const circIdx = ancestors.indexOf(val);
  if (circIdx !== -1) {
    return `[Circular *${circIdx + 1}]`;
  }

  const maxDepth = opts.depth === null ? Infinity : opts.depth;
  if (depth > maxDepth) {
    if (Array.isArray(val)) return '[Array]';
    const ctorName = val.constructor && val.constructor.name ? val.constructor.name : 'Object';
    return `[${ctorName}]`;
  }

  const nextAncestors = ancestors.concat([val]);

  if (Array.isArray(val)) {
    if (val.length === 0) return '[]';
    const limit = Math.min(val.length, opts.maxArrayLength);
    const items = [];
    for (let i = 0; i < limit; i++) {
      if (Object.prototype.hasOwnProperty.call(val, i)) {
        items.push(inspectValue(val[i], opts, depth + 1, nextAncestors));
      } else {
        items.push('<empty item>');
      }
    }
    if (val.length > limit) {
      items.push(`... ${val.length - limit} more item${val.length - limit === 1 ? '' : 's'}`);
    }
    return `[ ${items.join(', ')} ]`;
  }

  if (types.isTypedArray(val)) {
    const name = val.constructor ? val.constructor.name : 'TypedArray';
    if (val.length === 0) return `${name}(0) []`;
    const limit = Math.min(val.length, opts.maxArrayLength);
    const items = [];
    for (let i = 0; i < limit; i++) {
      items.push(String(val[i]));
    }
    if (val.length > limit) {
      items.push(`... ${val.length - limit} more item${val.length - limit === 1 ? '' : 's'}`);
    }
    return `${name}(${val.length}) [ ${items.join(', ')} ]`;
  }

  if (types.isMap(val)) {
    if (val.size === 0) return 'Map(0) {}';
    const entries = [];
    let count = 0;
    for (const [k, v] of val.entries()) {
      if (count >= opts.maxArrayLength) {
        entries.push(`... ${val.size - count} more items`);
        break;
      }
      entries.push(`${inspectValue(k, opts, depth + 1, nextAncestors)} => ${inspectValue(v, opts, depth + 1, nextAncestors)}`);
      count++;
    }
    return `Map(${val.size}) { ${entries.join(', ')} }`;
  }

  if (types.isSet(val)) {
    if (val.size === 0) return 'Set(0) {}';
    const entries = [];
    let count = 0;
    for (const item of val.values()) {
      if (count >= opts.maxArrayLength) {
        entries.push(`... ${val.size - count} more items`);
        break;
      }
      entries.push(inspectValue(item, opts, depth + 1, nextAncestors));
      count++;
    }
    return `Set(${val.size}) { ${entries.join(', ')} }`;
  }

  const keys = opts.showHidden
    ? Reflect.ownKeys(val)
    : Object.keys(val).concat(
        Object.getOwnPropertySymbols(val).filter((s) => Object.prototype.propertyIsEnumerable.call(val, s))
      );
  if (opts.sorted) {
    keys.sort((a, b) => String(a).localeCompare(String(b)));
  }

  const ctorName = val.constructor && val.constructor.name && val.constructor.name !== 'Object'
    ? `${val.constructor.name} `
    : '';

  if (keys.length === 0) {
    return ctorName ? `${ctorName}{}` : '{}';
  }

  const pairs = keys.map((k) => {
    let propVal;
    try {
      propVal = val[k];
    } catch (e) {
      propVal = `[Thrown: ${e && e.message ? e.message : e}]`;
    }
    return `${formatKey(k)}: ${inspectValue(propVal, opts, depth + 1, nextAncestors)}`;
  });

  return `${ctorName}{ ${pairs.join(', ')} }`;
}

function inspect(object, options = {}) {
  const opts = {
    depth: 2,
    maxArrayLength: 100,
    maxStringLength: 10000,
    showHidden: false,
    sorted: false,
    ...(typeof options === 'boolean' ? { showHidden: options } : options),
  };
  return inspectValue(object, opts, 0, []);
}

inspect.custom = customInspectSymbol;
inspect.defaultOptions = {
  showHidden: false,
  depth: 2,
  colors: false,
  customInspect: true,
  showProxy: false,
  maxArrayLength: 100,
  maxStringLength: 10000,
  breakLength: 80,
  compact: 3,
  sorted: false,
  getters: false,
};

function formatWithOptions(inspectOptions, f, ...args) {
  const opts = inspectOptions && typeof inspectOptions === 'object' ? inspectOptions : {};
  if (typeof f !== 'string') {
    const all = arguments.length >= 2 ? [f, ...args] : [];
    return all.map((item) => (typeof item === 'string' ? item : inspect(item, opts))).join(' ');
  }

  let argIdx = 0;
  let out = '';
  for (let i = 0; i < f.length; i++) {
    const ch = f[i];
    if (ch === '%' && i + 1 < f.length) {
      const spec = f[i + 1];
      if (spec === '%') {
        out += '%';
        i++;
        continue;
      }
      if (argIdx < args.length) {
        const val = args[argIdx];
        if (spec === 's') {
          out += typeof val === 'symbol' ? String(val) : String(val);
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'd') {
          out += typeof val === 'symbol' ? 'NaN' : (typeof val === 'bigint' ? `${val}n` : String(Number(val)));
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'i') {
          out += typeof val === 'symbol' ? 'NaN' : (typeof val === 'bigint' ? `${val}n` : String(parseInt(val, 10)));
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'f') {
          out += typeof val === 'symbol' ? 'NaN' : String(parseFloat(val));
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'j') {
          try {
            out += safeJsonStringify(val);
          } catch (_) {
            out += '[Circular]';
          }
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'o') {
          out += inspect(val, { ...opts, showHidden: true, showProxy: true });
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'O') {
          out += inspect(val, opts);
          argIdx++;
          i++;
          continue;
        }
        if (spec === 'c') {
          argIdx++;
          i++;
          continue;
        }
      }
    }
    out += ch;
  }

  while (argIdx < args.length) {
    const extra = args[argIdx++];
    out += ' ' + (typeof extra === 'string' ? extra : inspect(extra, opts));
  }
  return out;
}

function format(f, ...args) {
  if (arguments.length === 0) {
    return '';
  }
  return formatWithOptions({}, f, ...args);
}

function promisify(original) {
  if (typeof original !== 'function') {
    throw new TypeError('The "original" argument must be of type Function.');
  }
  if (original[customPromisifySymbol]) {
    const fn = original[customPromisifySymbol];
    if (typeof fn !== 'function') {
      throw new TypeError('The "util.promisify.custom" property must be of type Function.');
    }
    return Object.defineProperty(fn, customPromisifySymbol, {
      value: fn,
      enumerable: false,
      writable: false,
      configurable: true,
    });
  }

  const argNames = original[customPromisifyArgsSymbol];

  function fn(...args) {
    return new Promise((resolve, reject) => {
      original.call(this, ...args, (err, ...values) => {
        if (err) {
          reject(err);
        } else if (Array.isArray(argNames)) {
          const obj = {};
          for (let i = 0; i < argNames.length; i++) {
            obj[argNames[i]] = values[i];
          }
          resolve(obj);
        } else {
          resolve(values[0]);
        }
      });
    });
  }

  Object.setPrototypeOf(fn, Object.getPrototypeOf(original));
  Object.defineProperty(fn, customPromisifySymbol, {
    value: fn,
    enumerable: false,
    writable: false,
    configurable: true,
  });
  return fn;
}

promisify.custom = customPromisifySymbol;
promisify.customPromisifyArgs = customPromisifyArgsSymbol;

function callbackify(original) {
  if (typeof original !== 'function') {
    throw new TypeError('The "original" argument must be of type Function.');
  }
  function callbackified(...args) {
    const maybeCb = args.pop();
    if (typeof maybeCb !== 'function') {
      throw new TypeError('The last argument must be of type Function.');
    }
    const invokeCb = (err, val) => {
      if (typeof queueMicrotask === 'function') {
        queueMicrotask(() => maybeCb(err, val));
      } else {
        Promise.resolve().then(() => maybeCb(err, val));
      }
    };
    Promise.resolve()
      .then(() => original.apply(this, args))
      .then(
        (res) => invokeCb(null, res),
        (reason) => {
          if (!reason) {
            const err = new Error('Promise was rejected with a falsy value');
            err.reason = reason;
            invokeCb(err);
          } else {
            invokeCb(reason);
          }
        }
      );
  }
  Object.setPrototypeOf(callbackified, Object.getPrototypeOf(original));
  return callbackified;
}

function deprecate(fn, msg, code) {
  if (typeof fn !== 'function') {
    throw new TypeError('The "fn" argument must be of type Function.');
  }
  if (typeof process !== 'undefined' && process.noDeprecation) {
    return fn;
  }
  let warned = false;
  function deprecated(...args) {
    if (!warned) {
      warned = true;
      const prefix = code ? `[${code}] DeprecationWarning: ` : 'DeprecationWarning: ';
      if (typeof console !== 'undefined' && typeof console.warn === 'function') {
        console.warn(prefix + msg);
      }
    }
    return fn.apply(this, args);
  }
  Object.setPrototypeOf(deprecated, Object.getPrototypeOf(fn));
  return deprecated;
}

function inherits(ctor, superCtor) {
  if (ctor === undefined || ctor === null) {
    throw new TypeError('The "ctor" argument must not be null or undefined.');
  }
  if (superCtor === undefined || superCtor === null) {
    throw new TypeError('The "superCtor" argument must not be null or undefined.');
  }
  if (superCtor.prototype === undefined) {
    throw new TypeError('The "superCtor.prototype" property must not be undefined.');
  }
  ctor.super_ = superCtor;
  Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
}

function matchBipartite(remA, remB, eqFn) {
  const n = remA.length;
  if (n !== remB.length) return false;
  if (n === 0) return true;
  const matchB = new Int32Array(n).fill(-1);
  const edgeCache = new Int8Array(n * n).fill(-1);
  function canMatch(i, j) {
    const idx = i * n + j;
    const cached = edgeCache[idx];
    if (cached !== -1) return cached === 1;
    const ok = eqFn(remA[i], remB[j]);
    edgeCache[idx] = ok ? 1 : 0;
    return ok;
  }
  function tryAugment(i, seen) {
    for (let j = 0; j < n; j++) {
      if (seen[j] || !canMatch(i, j)) continue;
      seen[j] = 1;
      if (matchB[j] === -1 || tryAugment(matchB[j], seen)) {
        matchB[j] = i;
        return true;
      }
    }
    return false;
  }
  for (let i = 0; i < n; i++) {
    const seen = new Uint8Array(n);
    if (!tryAugment(i, seen)) return false;
  }
  return true;
}

function innerDeepStrictEqual(a, b, memos) {
  if (Object.is(a, b)) {
    return true;
  }
  if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') {
    return false;
  }
  if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) {
    return false;
  }

  const tagA = getTag(a);
  const tagB = getTag(b);
  if (tagA !== tagB) {
    return false;
  }

  if (tagA === '[object Date]') {
    if (a.getTime() !== b.getTime()) return false;
  }
  if (tagA === '[object RegExp]') {
    if (a.source !== b.source || a.flags !== b.flags || a.lastIndex !== b.lastIndex) return false;
  }
  if (types.isBoxedPrimitive(a)) {
    if (!Object.is(a.valueOf(), b.valueOf())) return false;
  }
  if (ArrayBuffer.isView(a)) {
    if (a.byteLength !== b.byteLength) return false;
    if (tagA === '[object Float32Array]' || tagA === '[object Float64Array]') {
      if (a.length !== b.length) return false;
      for (let i = 0; i < a.length; i++) {
        if (!Object.is(a[i], b[i])) return false;
      }
    } else {
      const ua = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
      const ub = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
      for (let i = 0; i < ua.length; i++) {
        if (ua[i] !== ub[i]) return false;
      }
    }
  }
  if (tagA === '[object ArrayBuffer]' || tagA === '[object SharedArrayBuffer]') {
    if (a.byteLength !== b.byteLength) return false;
    const ua = new Uint8Array(a);
    const ub = new Uint8Array(b);
    for (let i = 0; i < ua.length; i++) {
      if (ua[i] !== ub[i]) return false;
    }
  }

  // Active-path 1-to-1 circular pair memoization
  if (memos.val1.has(a)) {
    if (memos.val2.has(b)) {
      return memos.val1.get(a) === memos.val2.get(b);
    }
    return false;
  }
  if (memos.val2.has(b)) {
    return false;
  }
  const pos = memos.position++;
  memos.val1.set(a, pos);
  memos.val2.set(b, pos);

  try {
    if (Array.isArray(a)) {
      if (a.length !== b.length) return false;
      for (let i = 0; i < a.length; i++) {
        const hasA = Object.prototype.hasOwnProperty.call(a, i);
        const hasB = Object.prototype.hasOwnProperty.call(b, i);
        if (hasA !== hasB) return false;
        if (hasA && !innerDeepStrictEqual(a[i], b[i], memos)) {
          return false;
        }
      }
    }

    if (tagA === '[object Set]') {
      if (a.size !== b.size) return false;
      const remA = [];
      const remB = [];
      for (const av of a.values()) {
        if (av === null || (typeof av !== 'object' && typeof av !== 'function')) {
          if (!b.has(av)) return false;
        } else {
          remA.push(av);
        }
      }
      for (const bv of b.values()) {
        if (bv !== null && (typeof bv === 'object' || typeof bv === 'function')) {
          remB.push(bv);
        }
      }
      if (!matchBipartite(remA, remB, (x, y) => innerDeepStrictEqual(x, y, memos))) {
        return false;
      }
    }

    if (tagA === '[object Map]') {
      if (a.size !== b.size) return false;
      const remA = [];
      const remB = [];
      for (const [ak, av] of a.entries()) {
        if (ak === null || (typeof ak !== 'object' && typeof ak !== 'function')) {
          if (!b.has(ak)) return false;
          if (!innerDeepStrictEqual(av, b.get(ak), memos)) return false;
        } else {
          remA.push([ak, av]);
        }
      }
      for (const [bk, bv] of b.entries()) {
        if (bk !== null && (typeof bk === 'object' || typeof bk === 'function')) {
          remB.push([bk, bv]);
        }
      }
      if (
        !matchBipartite(
          remA,
          remB,
          ([ak, av], [bk, bv]) =>
            innerDeepStrictEqual(ak, bk, memos) && innerDeepStrictEqual(av, bv, memos)
        )
      ) {
        return false;
      }
    }

    if (types.isNativeError(a)) {
      if (a.name !== b.name || a.message !== b.message) return false;
      const hasCauseA = 'cause' in a;
      const hasCauseB = 'cause' in b;
      if (hasCauseA !== hasCauseB) return false;
      if (hasCauseA && !innerDeepStrictEqual(a.cause, b.cause, memos)) return false;
    }

    const keysA = Object.keys(a);
    const keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    for (let i = 0; i < keysA.length; i++) {
      const k = keysA[i];
      if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
      if (!innerDeepStrictEqual(a[k], b[k], memos)) return false;
    }

    const symsA = Object.getOwnPropertySymbols(a).filter((s) =>
      Object.prototype.propertyIsEnumerable.call(a, s)
    );
    const symsB = Object.getOwnPropertySymbols(b).filter((s) =>
      Object.prototype.propertyIsEnumerable.call(b, s)
    );
    if (symsA.length !== symsB.length) return false;
    for (let i = 0; i < symsA.length; i++) {
      const s = symsA[i];
      if (!Object.prototype.propertyIsEnumerable.call(b, s)) return false;
      if (!innerDeepStrictEqual(a[s], b[s], memos)) return false;
    }

    return true;
  } finally {
    memos.val1.delete(a);
    memos.val2.delete(b);
  }
}

function isDeepStrictEqual(val1, val2) {
  return innerDeepStrictEqual(val1, val2, {
    val1: new WeakMap(),
    val2: new WeakMap(),
    position: 0,
  });
}

// ============================================================================
// Pure-JS TextEncoder & TextDecoder
// ============================================================================

class TextEncoder {
  get encoding() {
    return 'utf-8';
  }

  encode(input = '') {
    return new Uint8Array(utf8ToBytes(String(input)));
  }

  encodeInto(source, destination) {
    if (!(destination instanceof Uint8Array)) {
      throw new TypeError('The "destination" argument must be an instance of Uint8Array.');
    }
    return utf8EncodeInto(String(source), destination, 0, destination.length);
  }
}

class TextDecoder {
  constructor(label = 'utf-8', options = {}) {
    const normalized = String(label).trim().toLowerCase();
    if (normalized === 'utf-8' || normalized === 'utf8' || normalized === 'unicode-1-1-utf-8') {
      this._encoding = 'utf-8';
    } else if (normalized === 'ascii' || normalized === 'us-ascii') {
      this._encoding = 'ascii';
    } else if (normalized === 'latin1' || normalized === 'iso-8859-1' || normalized === 'windows-1252') {
      this._encoding = 'latin1';
    } else if (normalized === 'utf-16le' || normalized === 'utf-16') {
      this._encoding = 'utf-16le';
    } else {
      throw new RangeError(`The encoding label provided ('${label}') is invalid.`);
    }
    this._fatal = Boolean(options && options.fatal);
    this._ignoreBOM = Boolean(options && options.ignoreBOM);
  }

  get encoding() {
    return this._encoding;
  }
  get fatal() {
    return this._fatal;
  }
  get ignoreBOM() {
    return this._ignoreBOM;
  }

  decode(input) {
    if (input === undefined) {
      return '';
    }
    let bytes;
    if (input instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && input instanceof SharedArrayBuffer)) {
      bytes = new Uint8Array(input);
    } else if (ArrayBuffer.isView(input)) {
      bytes = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    } else {
      throw new TypeError('The provided value is not of type "(ArrayBuffer or ArrayBufferView)"');
    }

    if (this._encoding === 'ascii') {
      let out = '';
      for (let i = 0; i < bytes.length; i++) {
        out += String.fromCharCode(bytes[i] & 0x7f);
      }
      return out;
    }
    if (this._encoding === 'latin1') {
      let out = '';
      for (let i = 0; i < bytes.length; i++) {
        out += String.fromCharCode(bytes[i]);
      }
      return out;
    }
    if (this._encoding === 'utf-16le') {
      let start = 0;
      if (!this._ignoreBOM && bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xfe) {
        start = 2;
      }
      let out = '';
      for (let i = start; i + 1 < bytes.length; i += 2) {
        out += String.fromCharCode(bytes[i] | (bytes[i + 1] << 8));
      }
      return out;
    }

    return bytesToUtf8(bytes, 0, bytes.length, {
      fatal: this._fatal,
      ignoreBOM: this._ignoreBOM,
    });
  }
}

module.exports = {
  format,
  formatWithOptions,
  inspect,
  promisify,
  callbackify,
  deprecate,
  inherits,
  isDeepStrictEqual,
  TextEncoder,
  TextDecoder,
  types,
};
