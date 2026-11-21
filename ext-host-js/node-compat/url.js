'use strict';

// Pure-JS Node.js url / WHATWG URL compatibility shim (no dependency on Node built-in 'url').

const pathCompat = require('./path');

const isWindowsHost =
  (typeof process !== 'undefined' && process.platform === 'win32') ||
  (typeof Deno !== 'undefined' && Deno.build && Deno.build.os === 'windows');

const SPECIAL_PORTS = {
  http: '80',
  https: '443',
  ws: '80',
  wss: '443',
  ftp: '21',
  file: '',
};

const SPECIAL_SCHEMES = new Set(Object.keys(SPECIAL_SCHEMES_MAP()));
function SPECIAL_SCHEMES_MAP() {
  return SPECIAL_PORTS;
}

// ============================================================================
// Pure-JS Punycode (RFC 3492) for domainToASCII / domainToUnicode
// ============================================================================

const PUNY_BASE = 36;
const PUNY_TMIN = 1;
const PUNY_TMAX = 26;
const PUNY_SKEW = 38;
const PUNY_DAMP = 700;
const PUNY_INITIAL_BIAS = 72;
const PUNY_INITIAL_N = 128;
const PUNY_MAX_INT = 0x7fffffff;

function punyAdapt(delta, numPoints, firstTime) {
  let d = firstTime ? Math.floor(delta / PUNY_DAMP) : delta >> 1;
  d += Math.floor(d / numPoints);
  let k = 0;
  while (d > ((PUNY_BASE - PUNY_TMIN) * PUNY_TMAX) >> 1) {
    d = Math.floor(d / (PUNY_BASE - PUNY_TMIN));
    k += PUNY_BASE;
  }
  return Math.floor(k + ((PUNY_BASE - PUNY_TMIN + 1) * d) / (d + PUNY_SKEW));
}

function encodeDigit(d) {
  return d + 22 + 75 * (d < 26 ? 1 : 0);
}

function decodeDigit(cp) {
  if (cp - 48 < 10) return cp - 22;
  if (cp - 65 < 26) return cp - 65;
  if (cp - 97 < 26) return cp - 97;
  return PUNY_BASE;
}

function ucs2Decode(str) {
  const out = [];
  for (let i = 0; i < str.length; i++) {
    const val = str.charCodeAt(i);
    if (val >= 0xd800 && val <= 0xdbff && i + 1 < str.length) {
      const extra = str.charCodeAt(i + 1);
      if ((extra & 0xfc00) === 0xdc00) {
        out.push(((val & 0x3ff) << 10) + (extra & 0x3ff) + 0x10000);
        i++;
      } else {
        out.push(val);
      }
    } else {
      out.push(val);
    }
  }
  return out;
}

function punycodeEncode(input) {
  const codePoints = ucs2Decode(input);
  const out = [];
  let basicLength = 0;
  for (let i = 0; i < codePoints.length; i++) {
    const cp = codePoints[i];
    if (cp < 0x80) {
      out.push(String.fromCharCode(cp));
      basicLength++;
    }
  }
  let handledCount = basicLength;
  if (basicLength > 0) {
    out.push('-');
  }
  let n = PUNY_INITIAL_N;
  let delta = 0;
  let bias = PUNY_INITIAL_BIAS;

  while (handledCount < codePoints.length) {
    let m = PUNY_MAX_INT;
    for (let i = 0; i < codePoints.length; i++) {
      const cp = codePoints[i];
      if (cp >= n && cp < m) {
        m = cp;
      }
    }
    const handledPlusOne = handledCount + 1;
    if (m - n > Math.floor((PUNY_MAX_INT - delta) / handledPlusOne)) {
      throw new RangeError('Punycode overflow');
    }
    delta += (m - n) * handledPlusOne;
    n = m;

    for (let i = 0; i < codePoints.length; i++) {
      const cp = codePoints[i];
      if (cp < n && ++delta > PUNY_MAX_INT) {
        throw new RangeError('Punycode overflow');
      }
      if (cp === n) {
        let q = delta;
        for (let k = PUNY_BASE; ; k += PUNY_BASE) {
          const t = k <= bias ? PUNY_TMIN : k >= bias + PUNY_TMAX ? PUNY_TMAX : k - bias;
          if (q < t) break;
          const qMinusT = q - t;
          const baseMinusT = PUNY_BASE - t;
          out.push(String.fromCharCode(encodeDigit(t + (qMinusT % baseMinusT))));
          q = Math.floor(qMinusT / baseMinusT);
        }
        out.push(String.fromCharCode(encodeDigit(q)));
        bias = punyAdapt(delta, handledPlusOne, handledCount === basicLength);
        delta = 0;
        handledCount++;
      }
    }
    delta++;
    n++;
  }
  return out.join('');
}

function punycodeDecode(input) {
  const output = [];
  const basicEnd = input.lastIndexOf('-');
  let i = 0;
  let n = PUNY_INITIAL_N;
  let bias = PUNY_INITIAL_BIAS;

  if (basicEnd > 0) {
    for (let j = 0; j < basicEnd; j++) {
      const c = input.charCodeAt(j);
      if (c >= 0x80) {
        throw new RangeError('Invalid non-ASCII character in Punycode');
      }
      output.push(c);
    }
  }
  let idx = basicEnd > 0 ? basicEnd + 1 : 0;
  while (idx < input.length) {
    const oldi = i;
    let w = 1;
    for (let k = PUNY_BASE; ; k += PUNY_BASE) {
      if (idx >= input.length) {
        throw new RangeError('Invalid Punycode input');
      }
      const digit = decodeDigit(input.charCodeAt(idx++));
      if (digit >= PUNY_BASE) {
        throw new RangeError('Invalid Punycode digit');
      }
      if (digit > Math.floor((PUNY_MAX_INT - i) / w)) {
        throw new RangeError('Punycode overflow');
      }
      i += digit * w;
      const t = k <= bias ? PUNY_TMIN : k >= bias + PUNY_TMAX ? PUNY_TMAX : k - bias;
      if (digit < t) break;
      const baseMinusT = PUNY_BASE - t;
      if (w > Math.floor(PUNY_MAX_INT / baseMinusT)) {
        throw new RangeError('Punycode overflow');
      }
      w *= baseMinusT;
    }
    const outLen = output.length + 1;
    bias = punyAdapt(i - oldi, outLen, oldi === 0);
    if (Math.floor(i / outLen) > PUNY_MAX_INT - n) {
      throw new RangeError('Punycode overflow');
    }
    n += Math.floor(i / outLen);
    i %= outLen;
    output.splice(i++, 0, n);
  }
  return String.fromCodePoint(...output);
}

function domainToASCII(domain) {
  if (typeof domain !== 'string') {
    domain = String(domain);
  }
  if (!domain) {
    return '';
  }
  try {
    const labels = domain.toLowerCase().split('.');
    return labels
      .map((label) => {
        if (!label) return '';
        for (let i = 0; i < label.length; i++) {
          if (label.charCodeAt(i) >= 0x80) {
            return 'xn--' + punycodeEncode(label);
          }
        }
        return label;
      })
      .join('.');
  } catch (_) {
    return '';
  }
}

function domainToUnicode(domain) {
  if (typeof domain !== 'string') {
    domain = String(domain);
  }
  if (!domain) {
    return '';
  }
  try {
    const labels = domain.split('.');
    return labels
      .map((label) => {
        if (label.toLowerCase().startsWith('xn--')) {
          return punycodeDecode(label.slice(4));
        }
        return label;
      })
      .join('.');
  } catch (_) {
    return '';
  }
}

// ============================================================================
// Percent encoding / decoding helpers
// ============================================================================

function decodeURIComponentSafe(str) {
  if (typeof str !== 'string' || str.indexOf('%') === -1) {
    return str;
  }
  return str.replace(/%(?:[0-9A-Fa-f]{2})+/g, (seq) => {
    try {
      return decodeURIComponent(seq);
    } catch (_) {
      return seq;
    }
  });
}

function encodeFormComponent(str) {
  return encodeURIComponent(String(str))
    .replace(/[!'()*~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase())
    .replace(/%20/g, '+');
}

function decodeFormComponent(str) {
  const plusReplaced = String(str).replace(/\+/g, ' ');
  return decodeURIComponentSafe(plusReplaced);
}

function encodePathSegment(seg) {
  // Encode characters disallowed/reserved in file URL path segments while keeping safe ASCII chars
  return encodeURIComponent(seg).replace(/[!'()*]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
}

function encodeUrlPathname(pathname) {
  if (!pathname) return '';
  let out = '';
  for (let i = 0; i < pathname.length; i++) {
    const ch = pathname[i];
    const code = pathname.charCodeAt(i);
    if (ch === '%' && i + 2 < pathname.length && /^[0-9A-Fa-f]{2}$/.test(pathname.slice(i + 1, i + 3))) {
      out += pathname.slice(i, i + 3);
      i += 2;
    } else if (code <= 0x20 || code >= 0x7f || ch === '"' || ch === '<' || ch === '>' || ch === '`' || ch === '#' || ch === '?' || ch === '{' || ch === '}') {
      // Encode single char or surrogate pair
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < pathname.length) {
        const low = pathname.charCodeAt(i + 1);
        if (low >= 0xdc00 && low <= 0xdfff) {
          out += encodeURIComponent(pathname.slice(i, i + 2));
          i++;
          continue;
        }
      }
      if (code >= 0x80) {
        try {
          out += encodeURIComponent(ch);
        } catch (_) {
          out += '%EF%BF%BD';
        }
      } else {
        out += '%' + code.toString(16).toUpperCase().padStart(2, '0');
      }
    } else {
      out += ch;
    }
  }
  return out;
}

function encodeUrlQueryOrHash(str, isQuery) {
  if (!str) return '';
  let out = '';
  for (let i = 0; i < str.length; i++) {
    const ch = str[i];
    const code = str.charCodeAt(i);
    if (ch === '%' && i + 2 < str.length && /^[0-9A-Fa-f]{2}$/.test(str.slice(i + 1, i + 3))) {
      out += str.slice(i, i + 3);
      i += 2;
    } else if (code < 0x20 || code >= 0x7f || ch === ' ' || ch === '"' || ch === '<' || ch === '>' || (isQuery && ch === '#') || (!isQuery && ch === '`')) {
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < str.length) {
        const low = str.charCodeAt(i + 1);
        if (low >= 0xdc00 && low <= 0xdfff) {
          out += encodeURIComponent(str.slice(i, i + 2));
          i++;
          continue;
        }
      }
      if (code >= 0x80) {
        try {
          out += encodeURIComponent(ch);
        } catch (_) {
          out += '%EF%BF%BD';
        }
      } else {
        out += '%' + code.toString(16).toUpperCase().padStart(2, '0');
      }
    } else {
      out += ch;
    }
  }
  return out;
}

// ============================================================================
// URLSearchParams
// ============================================================================

const SEARCH_PARAMS_OWNER = Symbol('URLSearchParamsOwner');

class URLSearchParams {
  constructor(init = '') {
    this._entries = [];
    this[SEARCH_PARAMS_OWNER] = null;

    if (init === undefined || init === null || init === '') {
      return;
    }
    if (init instanceof URLSearchParams) {
      for (const [k, v] of init._entries) {
        this._entries.push([k, v]);
      }
      return;
    }
    if (typeof init === 'string') {
      this._parseString(init);
      return;
    }
    if (Array.isArray(init)) {
      for (const pair of init) {
        if (!Array.isArray(pair) && !(pair && typeof pair[Symbol.iterator] === 'function')) {
          throw new TypeError('Iterator value is not an entry object');
        }
        const arr = Array.isArray(pair) ? pair : Array.from(pair);
        if (arr.length !== 2) {
          throw new TypeError('Sequence initializer must only contain pair elements');
        }
        this._entries.push([String(arr[0]), String(arr[1])]);
      }
      return;
    }
    if (typeof init === 'object') {
      if (typeof init[Symbol.iterator] === 'function') {
        for (const pair of init) {
          const arr = Array.from(pair);
          if (arr.length !== 2) {
            throw new TypeError('Sequence initializer must only contain pair elements');
          }
          this._entries.push([String(arr[0]), String(arr[1])]);
        }
      } else {
        for (const key of Object.keys(init)) {
          this._entries.push([String(key), String(init[key])]);
        }
      }
      return;
    }
    this._parseString(String(init));
  }

  _parseString(raw) {
    this._entries.length = 0;
    const str = raw.startsWith('?') ? raw.slice(1) : raw;
    if (!str) return;
    const parts = str.split('&');
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      if (!part) continue;
      const eq = part.indexOf('=');
      if (eq === -1) {
        this._entries.push([decodeFormComponent(part), '']);
      } else {
        this._entries.push([
          decodeFormComponent(part.slice(0, eq)),
          decodeFormComponent(part.slice(eq + 1)),
        ]);
      }
    }
  }

  _notifyOwner() {
    const owner = this[SEARCH_PARAMS_OWNER];
    if (owner) {
      const s = this.toString();
      owner._search = s ? '?' + s : '';
    }
  }

  get size() {
    return this._entries.length;
  }

  append(name, value) {
    this._entries.push([String(name), String(value)]);
    this._notifyOwner();
  }

  delete(name, value) {
    const key = String(name);
    const hasVal = value !== undefined;
    const targetVal = hasVal ? String(value) : undefined;
    this._entries = this._entries.filter(([k, v]) => {
      if (k !== key) return true;
      if (hasVal && v !== targetVal) return true;
      return false;
    });
    this._notifyOwner();
  }

  get(name) {
    const key = String(name);
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === key) {
        return this._entries[i][1];
      }
    }
    return null;
  }

  getAll(name) {
    const key = String(name);
    const out = [];
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === key) {
        out.push(this._entries[i][1]);
      }
    }
    return out;
  }

  has(name, value) {
    const key = String(name);
    const hasVal = value !== undefined;
    const targetVal = hasVal ? String(value) : undefined;
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === key && (!hasVal || this._entries[i][1] === targetVal)) {
        return true;
      }
    }
    return false;
  }

  set(name, value) {
    const key = String(name);
    const val = String(value);
    let replaced = false;
    const next = [];
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === key) {
        if (!replaced) {
          next.push([key, val]);
          replaced = true;
        }
      } else {
        next.push(this._entries[i]);
      }
    }
    if (!replaced) {
      next.push([key, val]);
    }
    this._entries = next;
    this._notifyOwner();
  }

  sort() {
    this._entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    this._notifyOwner();
  }

  keys() {
    return this._entries.map(([k]) => k).values();
  }

  values() {
    return this._entries.map(([, v]) => v).values();
  }

  entries() {
    return this._entries.map(([k, v]) => [k, v]).values();
  }

  [Symbol.iterator]() {
    return this.entries();
  }

  forEach(callback, thisArg) {
    for (const [k, v] of this._entries.slice()) {
      callback.call(thisArg, v, k, this);
    }
  }

  toString() {
    return this._entries
      .map(([k, v]) => `${encodeFormComponent(k)}=${encodeFormComponent(v)}`)
      .join('&');
  }
}

// ============================================================================
// Pure-JS WHATWG URL parser / serializer
// ============================================================================

function normalizeUrlPathSegments(rawPath, isSpecialFile) {
  const hasLeadingSlash = rawPath.startsWith('/');
  const segs = rawPath.split('/');
  const out = [];
  for (let i = 0; i < segs.length; i++) {
    const seg = segs[i];
    if (seg === '.' || seg === '%2e' || seg === '%2E') {
      if (i === segs.length - 1 && out.length > 0) {
        out.push('');
      }
      continue;
    }
    if (
      seg === '..' ||
      seg === '%2e%2e' ||
      seg === '%2e.' ||
      seg === '.%2e' ||
      seg === '%2E%2E' ||
      seg === '%2E.' ||
      seg === '.%2E'
    ) {
      const minLen = hasLeadingSlash ? 1 : 0;
      if (
        out.length > minLen &&
        !(isSpecialFile && hasLeadingSlash && out.length === 2 && /^[A-Za-z]:$/.test(out[1]))
      ) {
        out.pop();
      }
      if (i === segs.length - 1 && !(hasLeadingSlash && out.length === 1)) {
        out.push('');
      }
      continue;
    }
    out.push(seg);
  }
  const joined = out.join('/');
  return hasLeadingSlash && !joined.startsWith('/') ? '/' + joined : joined;
}

function parseHostPort(authorityRest, scheme) {
  if (!authorityRest) {
    return { hostname: '', port: '' };
  }
  let hostname = authorityRest;
  let port = '';
  if (authorityRest.startsWith('[')) {
    const closeIdx = authorityRest.indexOf(']');
    if (closeIdx === -1) {
      throw new TypeError('Invalid IPv6 host in URL');
    }
    hostname = authorityRest.slice(0, closeIdx + 1).toLowerCase();
    const after = authorityRest.slice(closeIdx + 1);
    if (after) {
      if (!after.startsWith(':')) {
        throw new TypeError('Invalid host/port in URL');
      }
      port = after.slice(1);
    }
  } else {
    const colonIdx = authorityRest.lastIndexOf(':');
    if (colonIdx !== -1) {
      hostname = authorityRest.slice(0, colonIdx);
      port = authorityRest.slice(colonIdx + 1);
    }
    if (hostname) {
      if (/[\s/?#\\@]/.test(hostname)) {
        throw new TypeError(`Invalid hostname in URL: ${hostname}`);
      }
      const asciiHost = domainToASCII(hostname);
      if (!asciiHost && hostname) {
        throw new TypeError(`Invalid hostname in URL: ${hostname}`);
      }
      hostname = asciiHost;
    }
  }
  if (port !== '') {
    if (!/^\d+$/.test(port)) {
      throw new TypeError(`Invalid port in URL: ${port}`);
    }
    const portNum = Number(port);
    if (portNum > 65535) {
      throw new TypeError(`Port out of range in URL: ${port}`);
    }
    port = String(portNum);
    if (SPECIAL_PORTS[scheme] && port === SPECIAL_PORTS[scheme]) {
      port = '';
    }
  }
  return { hostname, port };
}

function parseRawUrl(rawInput, baseInput) {
  let input = String(rawInput).replace(/^[\u0000-\u0020]+|[\u0000-\u0020]+$/g, '');
  input = input.replace(/[\t\r\n]/g, '');

  let baseParsed = null;
  if (baseInput !== undefined) {
    if (baseInput instanceof URL) {
      baseParsed = baseInput._snapshot();
    } else {
      baseParsed = parseRawUrl(String(baseInput), undefined);
    }
  }

  // Extract fragment
  let hash = '';
  const hashIdx = input.indexOf('#');
  if (hashIdx !== -1) {
    hash = '#' + encodeUrlQueryOrHash(input.slice(hashIdx + 1), false);
    input = input.slice(0, hashIdx);
    if (hash === '#') hash = '#';
  }

  // Extract query
  let search = '';
  const queryIdx = input.indexOf('?');
  if (queryIdx !== -1) {
    search = '?' + encodeUrlQueryOrHash(input.slice(queryIdx + 1), true);
    input = input.slice(0, queryIdx);
  }

  // Check scheme
  const schemeMatch = input.match(/^([A-Za-z][A-Za-z0-9+.-]*):(.*)$/);
  if (!schemeMatch) {
    if (!baseParsed) {
      throw new TypeError(`Invalid URL: ${rawInput}`);
    }
    // Relative resolution against baseParsed
    const res = { ...baseParsed };
    if (input === '') {
      if (search !== '') {
        res.search = search;
        res.hash = hash;
      } else if (hash !== '') {
        res.hash = hash;
      }
      return res;
    }
    const isSpecial = SPECIAL_SCHEMES.has(res.scheme);
    if (isSpecial) {
      input = input.replace(/\\/g, '/');
    }
    if (input.startsWith('//')) {
      const reconstructed = `${res.scheme}:${input}${search}${hash}`;
      return parseRawUrl(reconstructed, undefined);
    }
    if (input.startsWith('/')) {
      res.pathname = normalizeUrlPathSegments(encodeUrlPathname(input), res.scheme === 'file');
    } else {
      const basePath = res.pathname || '/';
      const lastSlash = basePath.lastIndexOf('/');
      const dir = lastSlash !== -1 ? basePath.slice(0, lastSlash + 1) : '/';
      res.pathname = normalizeUrlPathSegments(encodeUrlPathname(dir + input), res.scheme === 'file');
    }
    res.search = search;
    res.hash = hash;
    return res;
  }

  const scheme = schemeMatch[1].toLowerCase();
  let rest = schemeMatch[2];
  const isSpecial = SPECIAL_SCHEMES.has(scheme);

  // If relative with same scheme as special base and not starting with '//'
  if (isSpecial) {
    rest = rest.replace(/\\/g, '/');
  }

  let hasSlashes = false;
  let username = '';
  let password = '';
  let hostname = '';
  let port = '';
  let pathname = '';

  if (rest.startsWith('//')) {
    hasSlashes = true;
    const afterSlashes = rest.slice(2);
    const slashIdx = afterSlashes.indexOf('/');
    const authority = slashIdx === -1 ? afterSlashes : afterSlashes.slice(0, slashIdx);
    const rawPath = slashIdx === -1 ? (isSpecial ? '/' : '') : afterSlashes.slice(slashIdx);

    if (authority) {
      const atIdx = authority.lastIndexOf('@');
      let hostPortStr = authority;
      if (atIdx !== -1) {
        const userinfo = authority.slice(0, atIdx);
        hostPortStr = authority.slice(atIdx + 1);
        const colonIdx = userinfo.indexOf(':');
        if (colonIdx !== -1) {
          username = encodeURIComponent(decodeURIComponentSafe(userinfo.slice(0, colonIdx)));
          password = encodeURIComponent(decodeURIComponentSafe(userinfo.slice(colonIdx + 1)));
        } else {
          username = encodeURIComponent(decodeURIComponentSafe(userinfo));
        }
      }
      const parsedHost = parseHostPort(hostPortStr, scheme);
      hostname = parsedHost.hostname;
      port = parsedHost.port;
    }

    if (isSpecial && scheme !== 'file' && !hostname) {
      throw new TypeError(`Invalid URL: missing host for ${scheme}:`);
    }
    if (scheme === 'file' && hostname === 'localhost') {
      hostname = '';
    }

    if (rawPath) {
      pathname = normalizeUrlPathSegments(encodeUrlPathname(rawPath), scheme === 'file');
      if (!pathname.startsWith('/') && (hasSlashes || isSpecial)) {
        pathname = '/' + pathname;
      }
    } else {
      pathname = isSpecial ? '/' : '';
    }
  } else {
    if (isSpecial && scheme !== 'file') {
      throw new TypeError(`Invalid URL: missing '//' for ${scheme}:`);
    }
    if (scheme === 'file') {
      hasSlashes = true;
      let normInput = rest.startsWith('/') ? rest : '/' + rest;
      pathname = normalizeUrlPathSegments(encodeUrlPathname(normInput), true);
    } else {
      pathname = encodeUrlPathname(rest);
    }
  }

  return {
    scheme,
    hasSlashes,
    username,
    password,
    hostname,
    port,
    pathname,
    search,
    hash,
  };
}

class URL {
  constructor(input, base) {
    if (arguments.length === 0) {
      throw new TypeError('Failed to construct "URL": 1 argument required, but only 0 present.');
    }
    const parsed = parseRawUrl(input, base);
    this._scheme = parsed.scheme;
    this._hasSlashes = parsed.hasSlashes;
    this._username = parsed.username;
    this._password = parsed.password;
    this._hostname = parsed.hostname;
    this._port = parsed.port;
    this._pathname = parsed.pathname;
    this._search = parsed.search;
    this._hash = parsed.hash;
    this._searchParams = new URLSearchParams(this._search);
    this[SEARCH_PARAMS_OWNER] = null;
    this._searchParams[SEARCH_PARAMS_OWNER] = this;
  }

  static canParse(input, base) {
    try {
      new URL(input, base);
      return true;
    } catch (_) {
      return false;
    }
  }

  _snapshot() {
    return {
      scheme: this._scheme,
      hasSlashes: this._hasSlashes,
      username: this._username,
      password: this._password,
      hostname: this._hostname,
      port: this._port,
      pathname: this._pathname,
      search: this._search,
      hash: this._hash,
    };
  }

  get protocol() {
    return this._scheme + ':';
  }
  set protocol(value) {
    const clean = String(value).replace(/:.*$/, '').toLowerCase();
    if (/^[a-z][a-z0-9+.-]*$/.test(clean)) {
      this._scheme = clean;
    }
  }

  get username() {
    return this._username;
  }
  set username(value) {
    this._username = encodeURIComponent(String(value));
  }

  get password() {
    return this._password;
  }
  set password(value) {
    this._password = encodeURIComponent(String(value));
  }

  get hostname() {
    return this._hostname;
  }
  set hostname(value) {
    const parsed = parseHostPort(String(value), this._scheme);
    this._hostname = parsed.hostname;
  }

  get port() {
    return this._port;
  }
  set port(value) {
    const s = String(value);
    if (s === '') {
      this._port = '';
      return;
    }
    if (/^\d+$/.test(s) && Number(s) <= 65535) {
      const p = String(Number(s));
      this._port = SPECIAL_PORTS[this._scheme] === p ? '' : p;
    }
  }

  get host() {
    if (!this._hostname) return '';
    return this._port ? `${this._hostname}:${this._port}` : this._hostname;
  }
  set host(value) {
    const parsed = parseHostPort(String(value), this._scheme);
    this._hostname = parsed.hostname;
    this._port = parsed.port;
  }

  get origin() {
    if (this._scheme === 'http' || this._scheme === 'https' || this._scheme === 'ws' || this._scheme === 'wss' || this._scheme === 'ftp') {
      return `${this._scheme}://${this.host}`;
    }
    return 'null';
  }

  get pathname() {
    return this._pathname;
  }
  set pathname(value) {
    const str = String(value);
    this._pathname = this._hasSlashes
      ? normalizeUrlPathSegments(encodeUrlPathname(str.startsWith('/') ? str : '/' + str), this._scheme === 'file')
      : encodeUrlPathname(str);
  }

  get search() {
    return this._search === '?' ? '' : this._search;
  }
  set search(value) {
    const s = String(value);
    if (!s || s === '?') {
      this._search = '';
      this._searchParams._parseString('');
    } else {
      const withQ = s.startsWith('?') ? s : '?' + s;
      this._search = '?' + encodeUrlQueryOrHash(withQ.slice(1), true);
      this._searchParams._parseString(this._search);
    }
  }

  get searchParams() {
    return this._searchParams;
  }

  get hash() {
    return this._hash === '#' ? '' : this._hash;
  }
  set hash(value) {
    const s = String(value);
    if (!s || s === '#') {
      this._hash = '';
    } else {
      const clean = s.startsWith('#') ? s.slice(1) : s;
      this._hash = '#' + encodeUrlQueryOrHash(clean, false);
    }
  }

  get href() {
    let out = this._scheme + ':';
    if (this._hasSlashes) {
      out += '//';
      if (this._username || this._password) {
        out += this._username;
        if (this._password) {
          out += ':' + this._password;
        }
        out += '@';
      }
      out += this.host;
    }
    out += this._pathname;
    if (this._search && this._search !== '?') {
      out += this._search;
    }
    if (this._hash && this._hash !== '#') {
      out += this._hash;
    }
    return out;
  }

  set href(value) {
    const next = new URL(value);
    this._scheme = next._scheme;
    this._hasSlashes = next._hasSlashes;
    this._username = next._username;
    this._password = next._password;
    this._hostname = next._hostname;
    this._port = next._port;
    this._pathname = next._pathname;
    this._search = next._search;
    this._hash = next._hash;
    this._searchParams._parseString(this._search);
  }

  toString() {
    return this.href;
  }

  toJSON() {
    return this.href;
  }
}

// ============================================================================
// Legacy url.parse / url.format / url.resolve
// ============================================================================

function parseLegacyQuery(qs) {
  const out = Object.create(null);
  if (!qs) return out;
  const parts = qs.split('&');
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (!part) continue;
    const eq = part.indexOf('=');
    const k = eq === -1 ? decodeFormComponent(part) : decodeFormComponent(part.slice(0, eq));
    const v = eq === -1 ? '' : decodeFormComponent(part.slice(eq + 1));
    if (Object.prototype.hasOwnProperty.call(out, k)) {
      if (Array.isArray(out[k])) {
        out[k].push(v);
      } else {
        out[k] = [out[k], v];
      }
    } else {
      out[k] = v;
    }
  }
  return out;
}

class Url {
  constructor() {
    this.protocol = null;
    this.slashes = null;
    this.auth = null;
    this.host = null;
    this.port = null;
    this.hostname = null;
    this.hash = null;
    this.search = null;
    this.query = null;
    this.pathname = null;
    this.path = null;
    this.href = null;
  }
}

function parse(urlString, parseQueryString = false, slashesDenoteHost = false) {
  if (urlString && typeof urlString === 'object' && urlString instanceof Url) {
    return urlString;
  }
  if (typeof urlString !== 'string') {
    throw new TypeError(`Parameter "url" must be a string, not ${typeof urlString}`);
  }
  const obj = new Url();
  let rest = urlString.trim();

  const hashIdx = rest.indexOf('#');
  if (hashIdx !== -1) {
    obj.hash = rest.slice(hashIdx);
    rest = rest.slice(0, hashIdx);
  }

  const queryIdx = rest.indexOf('?');
  if (queryIdx !== -1) {
    const rawQuery = rest.slice(queryIdx + 1);
    obj.search = '?' + rawQuery;
    obj.query = parseQueryString ? parseLegacyQuery(rawQuery) : rawQuery;
    rest = rest.slice(0, queryIdx);
  } else {
    obj.query = parseQueryString ? Object.create(null) : null;
  }

  const protoMatch = rest.match(/^([A-Za-z][A-Za-z0-9+.-]*):(.*)$/);
  if (protoMatch) {
    obj.protocol = protoMatch[1].toLowerCase() + ':';
    rest = protoMatch[2];
  }

  if (rest.startsWith('//') && (obj.protocol || slashesDenoteHost)) {
    obj.slashes = true;
    rest = rest.slice(2);
    const slashIdx = rest.indexOf('/');
    const authHost = slashIdx === -1 ? rest : rest.slice(0, slashIdx);
    rest = slashIdx === -1 ? '/' : rest.slice(slashIdx);

    const atIdx = authHost.lastIndexOf('@');
    let hostPart = authHost;
    if (atIdx !== -1) {
      obj.auth = decodeURIComponentSafe(authHost.slice(0, atIdx));
      hostPart = authHost.slice(atIdx + 1);
    }
    obj.host = hostPart.toLowerCase();
    if (hostPart.startsWith('[')) {
      const closeIdx = hostPart.indexOf(']');
      if (closeIdx !== -1) {
        obj.hostname = hostPart.slice(1, closeIdx).toLowerCase();
        if (hostPart[closeIdx + 1] === ':') {
          obj.port = hostPart.slice(closeIdx + 2) || null;
        }
      } else {
        obj.hostname = hostPart.toLowerCase();
      }
    } else {
      const colonIdx = hostPart.lastIndexOf(':');
      if (colonIdx !== -1) {
        obj.hostname = hostPart.slice(0, colonIdx).toLowerCase();
        obj.port = hostPart.slice(colonIdx + 1) || null;
      } else {
        obj.hostname = hostPart.toLowerCase();
      }
    }
  } else if (obj.protocol && rest.startsWith('//')) {
    obj.slashes = true;
  }

  obj.pathname = rest || (obj.slashes ? '/' : null);
  if (obj.pathname !== null || obj.search !== null) {
    obj.path = (obj.pathname || '') + (obj.search || '');
  }
  obj.href = format(obj);
  return obj;
}

function format(urlObject) {
  if (typeof urlObject === 'string') {
    return parse(urlObject).href;
  }
  if (urlObject instanceof URL) {
    return urlObject.href;
  }
  if (!urlObject || typeof urlObject !== 'object') {
    throw new TypeError('Parameter "urlObject" must be an object or string');
  }

  let protocol = urlObject.protocol || '';
  if (protocol && !protocol.endsWith(':')) {
    protocol += ':';
  }
  const auth = urlObject.auth ? encodeURIComponent(urlObject.auth).replace(/%3A/gi, ':') + '@' : '';
  let host = urlObject.host || '';
  if (!host && urlObject.hostname) {
    host = urlObject.hostname.includes(':') && !urlObject.hostname.startsWith('[')
      ? `[${urlObject.hostname}]`
      : urlObject.hostname;
    if (urlObject.port) {
      host += ':' + urlObject.port;
    }
  }

  let pathname = urlObject.pathname || '';
  let search = urlObject.search || '';
  if (!search && urlObject.query && typeof urlObject.query === 'object') {
    const qs = new URLSearchParams(urlObject.query).toString();
    if (qs) search = '?' + qs;
  } else if (search && !search.startsWith('?')) {
    search = '?' + search;
  }

  let hash = urlObject.hash || '';
  if (hash && !hash.startsWith('#')) {
    hash = '#' + hash;
  }

  const slashes = Boolean(
    urlObject.slashes ||
    (protocol && (SPECIAL_SCHEMES.has(protocol.slice(0, -1).toLowerCase()) || host))
  );

  let out = protocol;
  if (slashes) {
    out += '//' + auth + host;
    if (pathname && !pathname.startsWith('/')) {
      pathname = '/' + pathname;
    }
  }
  out += pathname + search + hash;
  return out;
}

function resolve(from, to) {
  try {
    return new URL(to, from).href;
  } catch (_) {
    const dummyBase = 'resolve://';
    const base = new URL(from.startsWith('/') ? from : '/' + from, dummyBase);
    const resolved = new URL(to, base);
    if (resolved.href.startsWith(dummyBase)) {
      return resolved.href.slice(dummyBase.length);
    }
    return resolved.href;
  }
}

// ============================================================================
// pathToFileURL / fileURLToPath
// ============================================================================

function pathToFileURL(filepath, options = {}) {
  if (typeof filepath !== 'string') {
    throw new TypeError('The "path" argument must be of type string.');
  }
  const isWindows = options && options.windows !== undefined
    ? Boolean(options.windows)
    : isWindowsHost;

  const flavor = isWindows ? pathCompat.win32 : pathCompat.posix;
  let resolved = flavor.resolve(filepath);

  // Preserve trailing separator if original path had one
  const hadTrailingSep = isWindows
    ? (filepath.endsWith('\\') || filepath.endsWith('/')) && resolved.length > 3
    : filepath.endsWith('/') && resolved.length > 1;
  if (hadTrailingSep && !resolved.endsWith(flavor.sep)) {
    resolved += flavor.sep;
  }

  if (isWindows) {
    // Handle UNC \\server\share\...
    if (resolved.startsWith('\\\\')) {
      const rest = resolved.slice(2);
      const parts = rest.split('\\');
      const server = parts[0] || '';
      const encodedTail = parts.slice(1).map(encodePathSegment).join('/');
      return new URL(`file://${domainToASCII(server) || server}/${encodedTail}`);
    }
    // Drive letter C:\...
    const slashNorm = resolved.replace(/\\/g, '/');
    const parts = slashNorm.split('/');
    // parts[0] is e.g. 'C:'
    const encodedParts = [parts[0]];
    for (let i = 1; i < parts.length; i++) {
      encodedParts.push(encodePathSegment(parts[i]));
    }
    return new URL('file:///' + encodedParts.join('/'));
  }

  const parts = resolved.split('/');
  const encodedParts = parts.map((seg, idx) => (idx === 0 ? '' : encodePathSegment(seg)));
  return new URL('file://' + encodedParts.join('/'));
}

function fileURLToPath(urlInput, options = {}) {
  let urlObj;
  if (typeof urlInput === 'string') {
    urlObj = new URL(urlInput);
  } else if (urlInput && typeof urlInput === 'object' && typeof urlInput.href === 'string') {
    urlObj = urlInput instanceof URL ? urlInput : new URL(urlInput.href);
  } else {
    throw new TypeError('The "path" argument must be of type string or an instance of URL.');
  }

  if (urlObj.protocol !== 'file:') {
    const err = new TypeError('The URL must be of scheme file');
    err.code = 'ERR_INVALID_URL_SCHEME';
    throw err;
  }

  const isWindows = options && options.windows !== undefined
    ? Boolean(options.windows)
    : isWindowsHost;

  const rawPathname = urlObj.pathname;
  if (/%2f/i.test(rawPathname) || (isWindows && /%5c/i.test(rawPathname))) {
    const err = new TypeError('File URL path must not include encoded / or \\ characters');
    err.code = 'ERR_INVALID_FILE_URL_PATH';
    throw err;
  }

  const decodedPath = decodeURIComponentSafe(rawPathname);

  if (isWindows) {
    const host = urlObj.hostname;
    if (host && host.toLowerCase() !== 'localhost') {
      return `\\\\${domainToUnicode(host) || host}${decodedPath.replace(/\//g, '\\')}`;
    }
    if (!/^\/[A-Za-z]:(?:\/|$)/.test(decodedPath)) {
      const err = new TypeError('File URL path must be absolute on Windows');
      err.code = 'ERR_INVALID_FILE_URL_PATH';
      throw err;
    }
    return decodedPath.slice(1).replace(/\//g, '\\');
  }

  if (urlObj.hostname && urlObj.hostname.toLowerCase() !== 'localhost') {
    const err = new TypeError('File URL host must be "localhost" or empty on POSIX');
    err.code = 'ERR_INVALID_FILE_URL_HOST';
    throw err;
  }

  return decodedPath;
}

module.exports = {
  URL,
  URLSearchParams,
  Url,
  parse,
  format,
  resolve,
  pathToFileURL,
  fileURLToPath,
  domainToASCII,
  domainToUnicode,
};
