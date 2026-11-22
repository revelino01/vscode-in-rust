'use strict';

// Pure-JS Node.js Buffer compatibility shim (no dependency on Node built-in 'buffer').

const kMaxLength = 0x7fffffff;
const INSPECT_MAX_BYTES = 50;
const constants = {
  MAX_LENGTH: kMaxLength,
  MAX_STRING_LENGTH: 536870888,
};

const BASE64_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const BASE64URL_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
const BASE64_REV = new Int16Array(128).fill(-1);
for (let i = 0; i < BASE64_CHARS.length; i++) {
  BASE64_REV[BASE64_CHARS.charCodeAt(i)] = i;
}
BASE64_REV[45] = 62; // '-'
BASE64_REV[95] = 63; // '_'

function normalizeEncoding(encoding) {
  if (encoding === undefined || encoding === null || encoding === '') {
    return 'utf8';
  }
  if (typeof encoding !== 'string') {
    throw new TypeError(`Unknown encoding: ${String(encoding)}`);
  }
  switch (encoding.toLowerCase()) {
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
    default:
      throw new TypeError(`Unknown encoding: ${encoding}`);
  }
}

function isEncoding(encoding) {
  if (typeof encoding !== 'string') {
    return false;
  }
  switch (encoding.toLowerCase()) {
    case 'utf8':
    case 'utf-8':
    case 'ascii':
    case 'latin1':
    case 'binary':
    case 'base64':
    case 'base64url':
    case 'hex':
    case 'utf16le':
    case 'utf-16le':
    case 'ucs2':
    case 'ucs-2':
      return true;
    default:
      return false;
  }
}

function utf8ToBytes(str) {
  const bytes = [];
  for (let i = 0; i < str.length; i++) {
    let code = str.charCodeAt(i);
    if (code < 0x80) {
      bytes.push(code);
    } else if (code < 0x800) {
      bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
    } else if (code >= 0xd800 && code <= 0xdbff) {
      if (i + 1 < str.length) {
        const low = str.charCodeAt(i + 1);
        if (low >= 0xdc00 && low <= 0xdfff) {
          const cp = 0x10000 + (((code - 0xd800) << 10) | (low - 0xdc00));
          bytes.push(
            0xf0 | (cp >> 18),
            0x80 | ((cp >> 12) & 0x3f),
            0x80 | ((cp >> 6) & 0x3f),
            0x80 | (cp & 0x3f)
          );
          i++;
          continue;
        }
      }
      // Lone high surrogate -> U+FFFD
      bytes.push(0xef, 0xbf, 0xbd);
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      // Lone low surrogate -> U+FFFD
      bytes.push(0xef, 0xbf, 0xbd);
    } else {
      bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    }
  }
  return bytes;
}

function utf8ByteLength(str) {
  let len = 0;
  for (let i = 0; i < str.length; i++) {
    const code = str.charCodeAt(i);
    if (code < 0x80) {
      len += 1;
    } else if (code < 0x800) {
      len += 2;
    } else if (code >= 0xd800 && code <= 0xdbff) {
      if (i + 1 < str.length) {
        const low = str.charCodeAt(i + 1);
        if (low >= 0xdc00 && low <= 0xdfff) {
          len += 4;
          i++;
          continue;
        }
      }
      len += 3;
    } else {
      len += 3;
    }
  }
  return len;
}

function utf8EncodeInto(str, destination, offset = 0, maxBytes = destination.length - offset) {
  let read = 0;
  let written = 0;
  for (let i = 0; i < str.length; i++) {
    const code = str.charCodeAt(i);
    let charBytes;
    let units = 1;
    if (code < 0x80) {
      charBytes = [code];
    } else if (code < 0x800) {
      charBytes = [0xc0 | (code >> 6), 0x80 | (code & 0x3f)];
    } else if (code >= 0xd800 && code <= 0xdbff && i + 1 < str.length) {
      const low = str.charCodeAt(i + 1);
      if (low >= 0xdc00 && low <= 0xdfff) {
        const cp = 0x10000 + (((code - 0xd800) << 10) | (low - 0xdc00));
        charBytes = [
          0xf0 | (cp >> 18),
          0x80 | ((cp >> 12) & 0x3f),
          0x80 | ((cp >> 6) & 0x3f),
          0x80 | (cp & 0x3f),
        ];
        units = 2;
      } else {
        charBytes = [0xef, 0xbf, 0xbd];
      }
    } else if (code >= 0xd800 && code <= 0xdfff) {
      charBytes = [0xef, 0xbf, 0xbd];
    } else {
      charBytes = [0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)];
    }
    if (written + charBytes.length > maxBytes) {
      break;
    }
    for (let j = 0; j < charBytes.length; j++) {
      destination[offset + written++] = charBytes[j];
    }
    read += units;
    if (units === 2) {
      i++;
    }
  }
  return { read, written };
}

function bytesToUtf8(buf, start = 0, end = buf.length, options = {}) {
  const fatal = Boolean(options && options.fatal);
  const ignoreBOM = options && options.ignoreBOM !== undefined ? Boolean(options.ignoreBOM) : true;
  const chunks = [];
  let i = start;
  if (!ignoreBOM && end - i >= 3 && buf[i] === 0xef && buf[i + 1] === 0xbb && buf[i + 2] === 0xbf) {
    i += 3;
  }
  while (i < end) {
    const b0 = buf[i];
    if (b0 < 0x80) {
      chunks.push(String.fromCharCode(b0));
      i += 1;
    } else if (b0 >= 0xc2 && b0 <= 0xdf) {
      if (i + 1 < end) {
        const b1 = buf[i + 1];
        if (b1 >= 0x80 && b1 <= 0xbf) {
          const cp = ((b0 & 0x1f) << 6) | (b1 & 0x3f);
          chunks.push(String.fromCharCode(cp));
          i += 2;
          continue;
        }
      }
      if (fatal) {
        throw new TypeError('The encoded data was not valid.');
      }
      chunks.push('\ufffd');
      i += 1;
    } else if (b0 >= 0xe0 && b0 <= 0xef) {
      const lower = b0 === 0xe0 ? 0xa0 : 0x80;
      const upper = b0 === 0xed ? 0x9f : 0xbf;
      if (i + 1 < end) {
        const b1 = buf[i + 1];
        if (b1 >= lower && b1 <= upper) {
          if (i + 2 < end) {
            const b2 = buf[i + 2];
            if (b2 >= 0x80 && b2 <= 0xbf) {
              const cp = ((b0 & 0x0f) << 12) | ((b1 & 0x3f) << 6) | (b2 & 0x3f);
              chunks.push(String.fromCharCode(cp));
              i += 3;
              continue;
            }
          }
          // Maximal subpart is [b0, b1]
          if (fatal) {
            throw new TypeError('The encoded data was not valid.');
          }
          chunks.push('\ufffd');
          i += 2;
          continue;
        }
      }
      if (fatal) {
        throw new TypeError('The encoded data was not valid.');
      }
      chunks.push('\ufffd');
      i += 1;
    } else if (b0 >= 0xf0 && b0 <= 0xf4) {
      const lower = b0 === 0xf0 ? 0x90 : 0x80;
      const upper = b0 === 0xf4 ? 0x8f : 0xbf;
      if (i + 1 < end) {
        const b1 = buf[i + 1];
        if (b1 >= lower && b1 <= upper) {
          if (i + 2 < end) {
            const b2 = buf[i + 2];
            if (b2 >= 0x80 && b2 <= 0xbf) {
              if (i + 3 < end) {
                const b3 = buf[i + 3];
                if (b3 >= 0x80 && b3 <= 0xbf) {
                  const cp =
                    ((b0 & 0x07) << 18) | ((b1 & 0x3f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f);
                  const adj = cp - 0x10000;
                  chunks.push(String.fromCharCode(0xd800 + (adj >> 10), 0xdc00 + (adj & 0x3ff)));
                  i += 4;
                  continue;
                }
              }
              // Maximal subpart is [b0, b1, b2]
              if (fatal) {
                throw new TypeError('The encoded data was not valid.');
              }
              chunks.push('\ufffd');
              i += 3;
              continue;
            }
          }
          // Maximal subpart is [b0, b1]
          if (fatal) {
            throw new TypeError('The encoded data was not valid.');
          }
          chunks.push('\ufffd');
          i += 2;
          continue;
        }
      }
      if (fatal) {
        throw new TypeError('The encoded data was not valid.');
      }
      chunks.push('\ufffd');
      i += 1;
    } else {
      if (fatal) {
        throw new TypeError('The encoded data was not valid.');
      }
      chunks.push('\ufffd');
      i += 1;
    }
  }
  return chunks.join('');
}

function hexVal(ch) {
  if (ch >= 48 && ch <= 57) return ch - 48;
  if (ch >= 65 && ch <= 70) return ch - 55;
  if (ch >= 97 && ch <= 102) return ch - 87;
  return -1;
}

function hexToBytes(str) {
  const out = [];
  const limit = str.length & ~1;
  for (let i = 0; i < limit; i += 2) {
    const hi = hexVal(str.charCodeAt(i));
    const lo = hexVal(str.charCodeAt(i + 1));
    if (hi === -1 || lo === -1) {
      break;
    }
    out.push((hi << 4) | lo);
  }
  return out;
}

function base64ToBytes(str) {
  const out = [];
  let accum = 0;
  let bits = 0;
  for (let i = 0; i < str.length; i++) {
    const code = str.charCodeAt(i);
    if (code === 61) { // '='
      break;
    }
    if (code > 127) {
      continue;
    }
    const val = BASE64_REV[code];
    if (val === -1) {
      continue;
    }
    accum = (accum << 6) | val;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push((accum >> bits) & 0xff);
    }
  }
  return out;
}

function bytesToBase64(buf, start = 0, end = buf.length, urlSafe = false) {
  const alphabet = urlSafe ? BASE64URL_CHARS : BASE64_CHARS;
  const parts = [];
  for (let i = start; i < end; i += 3) {
    const b0 = buf[i];
    const has1 = i + 1 < end;
    const has2 = i + 2 < end;
    const b1 = has1 ? buf[i + 1] : 0;
    const b2 = has2 ? buf[i + 2] : 0;
    const triple = (b0 << 16) | (b1 << 8) | b2;
    parts.push(
      alphabet[(triple >> 18) & 0x3f],
      alphabet[(triple >> 12) & 0x3f],
      has1 ? alphabet[(triple >> 6) & 0x3f] : (urlSafe ? '' : '='),
      has2 ? alphabet[triple & 0x3f] : (urlSafe ? '' : '=')
    );
  }
  return parts.join('');
}

function decodeBytes(buf, encoding = 'utf8', start = 0, end = buf.length) {
  const enc = normalizeEncoding(encoding);
  const s = start === undefined ? 0 : Math.max(0, Math.trunc(Number(start) || 0));
  const e = end === undefined ? buf.length : Math.min(buf.length, Math.trunc(Number(end) || 0));
  if (e <= s) {
    return '';
  }
  switch (enc) {
    case 'utf8':
      return bytesToUtf8(buf, s, e);
    case 'ascii': {
      let out = '';
      for (let i = s; i < e; i++) {
        out += String.fromCharCode(buf[i] & 0x7f);
      }
      return out;
    }
    case 'latin1': {
      let out = '';
      for (let i = s; i < e; i++) {
        out += String.fromCharCode(buf[i]);
      }
      return out;
    }
    case 'utf16le': {
      let out = '';
      for (let i = s; i + 1 < e; i += 2) {
        out += String.fromCharCode(buf[i] | (buf[i + 1] << 8));
      }
      return out;
    }
    case 'hex': {
      let out = '';
      for (let i = s; i < e; i++) {
        out += buf[i].toString(16).padStart(2, '0');
      }
      return out;
    }
    case 'base64':
      return bytesToBase64(buf, s, e, false);
    case 'base64url':
      return bytesToBase64(buf, s, e, true);
    default:
      return bytesToUtf8(buf, s, e);
  }
}

function encodeString(str, encoding) {
  const enc = normalizeEncoding(encoding);
  switch (enc) {
    case 'utf8':
      return utf8ToBytes(str);
    case 'ascii': {
      const out = new Array(str.length);
      for (let i = 0; i < str.length; i++) {
        out[i] = str.charCodeAt(i) & 0x7f;
      }
      return out;
    }
    case 'latin1': {
      const out = new Array(str.length);
      for (let i = 0; i < str.length; i++) {
        out[i] = str.charCodeAt(i) & 0xff;
      }
      return out;
    }
    case 'utf16le': {
      const out = new Array(str.length * 2);
      for (let i = 0; i < str.length; i++) {
        const c = str.charCodeAt(i);
        out[i * 2] = c & 0xff;
        out[i * 2 + 1] = (c >> 8) & 0xff;
      }
      return out;
    }
    case 'hex':
      return hexToBytes(str);
    case 'base64':
    case 'base64url':
      return base64ToBytes(str);
    default:
      return utf8ToBytes(str);
  }
}

function assertSize(size) {
  if (typeof size !== 'number') {
    throw new TypeError(`The "size" argument must be of type number. Received type ${typeof size}`);
  }
  if (!Number.isFinite(size) || size < 0 || size > kMaxLength) {
    throw new RangeError(`The value "${size}" is invalid for option "size"`);
  }
}

function checkOffset(offset, byteWidth, length) {
  if (typeof offset !== 'number') {
    throw new TypeError(`The "offset" argument must be of type number. Received type ${typeof offset}`);
  }
  if (!Number.isInteger(offset)) {
    throw new RangeError(`The value of "offset" is out of range. Received ${offset}`);
  }
  if (offset < 0 || offset + byteWidth > length) {
    throw new RangeError(
      `Attempt to access memory outside buffer bounds (offset=${offset}, byteWidth=${byteWidth}, length=${length})`
    );
  }
}

function checkIntRange(value, min, max, name = 'value') {
  if (typeof value !== 'number') {
    throw new TypeError(`The "${name}" argument must be of type number. Received type ${typeof value}`);
  }
  if (!Number.isInteger(value) || value < min || value > max) {
    throw new RangeError(
      `The value of "${name}" is out of range. It must be >= ${min} and <= ${max}. Received ${value}`
    );
  }
}

class Buffer extends Uint8Array {
  constructor(arg, byteOffset, length) {
    if (typeof arg === 'number') {
      assertSize(arg);
      super(arg);
    } else if (typeof arg === 'string') {
      const bytes = encodeString(arg, byteOffset);
      super(bytes);
    } else if (arg instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && arg instanceof SharedArrayBuffer)) {
      super(arg, byteOffset, length);
    } else if (ArrayBuffer.isView(arg)) {
      super(new Uint8Array(arg.buffer, arg.byteOffset, arg.byteLength));
    } else if (arg && typeof arg === 'object' && arg.type === 'Buffer' && Array.isArray(arg.data)) {
      super(arg.data);
    } else if (arg && typeof arg.length === 'number') {
      super(arg);
    } else if (arg === undefined) {
      super(0);
    } else {
      throw new TypeError('First argument must be a string, Buffer, ArrayBuffer, Array, or array-like object.');
    }
  }

  static isBuffer(obj) {
    return obj instanceof Buffer;
  }

  static isEncoding(encoding) {
    return isEncoding(encoding);
  }

  static alloc(size, fill, encoding) {
    assertSize(size);
    const buf = new Buffer(size);
    if (fill !== undefined && size > 0) {
      buf.fill(fill, 0, size, encoding);
    }
    return buf;
  }

  static allocUnsafe(size) {
    assertSize(size);
    return new Buffer(size);
  }

  static allocUnsafeSlow(size) {
    assertSize(size);
    return new Buffer(size);
  }

  static from(value, encodingOrOffset, length) {
    if (typeof value === 'string') {
      const bytes = encodeString(value, encodingOrOffset);
      return new Buffer(bytes);
    }
    if (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && value instanceof SharedArrayBuffer)) {
      const offset = encodingOrOffset === undefined ? 0 : Number(encodingOrOffset);
      if (!Number.isInteger(offset) || offset < 0 || offset > value.byteLength) {
        throw new RangeError('Invalid byteOffset for Buffer.from(ArrayBuffer)');
      }
      const len = length === undefined ? value.byteLength - offset : Number(length);
      if (!Number.isInteger(len) || len < 0 || offset + len > value.byteLength) {
        throw new RangeError('Invalid length for Buffer.from(ArrayBuffer)');
      }
      return new Buffer(value, offset, len);
    }
    if (ArrayBuffer.isView(value)) {
      const view = new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      const out = new Buffer(view.length);
      out.set(view);
      return out;
    }
    if (value && typeof value === 'object' && value.type === 'Buffer' && Array.isArray(value.data)) {
      const out = new Buffer(value.data.length);
      for (let i = 0; i < value.data.length; i++) {
        out[i] = value.data[i] & 0xff;
      }
      return out;
    }
    if (Array.isArray(value) || (value && typeof value === 'object' && typeof value.length === 'number')) {
      const len = value.length >>> 0;
      const out = new Buffer(len);
      for (let i = 0; i < len; i++) {
        out[i] = value[i] & 0xff;
      }
      return out;
    }
    if (value && typeof value === 'object' && typeof value[Symbol.iterator] === 'function') {
      return Buffer.from(Array.from(value));
    }
    throw new TypeError(
      'The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object.'
    );
  }

  static concat(list, totalLength) {
    if (!Array.isArray(list)) {
      throw new TypeError('The "list" argument must be an instance of Array.');
    }
    if (list.length === 0) {
      return Buffer.alloc(0);
    }
    let computedLen = 0;
    for (let i = 0; i < list.length; i++) {
      const item = list[i];
      if (!(item instanceof Uint8Array)) {
        throw new TypeError('The "list" elements must be instances of Buffer or Uint8Array.');
      }
      computedLen += item.length;
    }
    const finalLen = totalLength === undefined ? computedLen : totalLength;
    assertSize(finalLen);
    const out = new Buffer(finalLen);
    let offset = 0;
    for (let i = 0; i < list.length && offset < finalLen; i++) {
      const item = list[i];
      const remaining = finalLen - offset;
      if (item.length <= remaining) {
        out.set(item, offset);
        offset += item.length;
      } else {
        out.set(item.subarray(0, remaining), offset);
        offset += remaining;
      }
    }
    return out;
  }

  static byteLength(string, encoding = 'utf8') {
    if (typeof string === 'string') {
      const enc = normalizeEncoding(encoding);
      switch (enc) {
        case 'utf8':
          return utf8ByteLength(string);
        case 'ascii':
        case 'latin1':
          return string.length;
        case 'utf16le':
          return string.length * 2;
        case 'hex':
          return hexToBytes(string).length;
        case 'base64':
        case 'base64url':
          return base64ToBytes(string).length;
        default:
          return utf8ByteLength(string);
      }
    }
    if (ArrayBuffer.isView(string) || string instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && string instanceof SharedArrayBuffer)) {
      return string.byteLength;
    }
    throw new TypeError('The "string" argument must be of type string or an instance of Buffer or ArrayBuffer.');
  }

  static compare(buf1, buf2) {
    if (!(buf1 instanceof Uint8Array) || !(buf2 instanceof Uint8Array)) {
      throw new TypeError('The "buf1" and "buf2" arguments must be instances of Buffer or Uint8Array.');
    }
    if (buf1 === buf2) {
      return 0;
    }
    const len = Math.min(buf1.length, buf2.length);
    for (let i = 0; i < len; i++) {
      if (buf1[i] !== buf2[i]) {
        return buf1[i] < buf2[i] ? -1 : 1;
      }
    }
    if (buf1.length === buf2.length) {
      return 0;
    }
    return buf1.length < buf2.length ? -1 : 1;
  }

  toString(encoding = 'utf8', start = 0, end = this.length) {
    return decodeBytes(this, encoding, start, end);
  }

  write(string, offset, length, encoding) {
    if (typeof string !== 'string') {
      throw new TypeError('The "string" argument must be of type string.');
    }
    if (offset === undefined) {
      offset = 0;
      length = this.length;
      encoding = 'utf8';
    } else if (length === undefined && typeof offset === 'string') {
      encoding = offset;
      offset = 0;
      length = this.length;
    } else {
      if (typeof offset !== 'number') {
        const err = new TypeError(`The "offset" argument must be of type number. Received type ${typeof offset}`);
        err.code = 'ERR_INVALID_ARG_TYPE';
        throw err;
      }
      if (!Number.isInteger(offset) || offset < 0 || offset > this.length) {
        const err = new RangeError(`The value of "offset" is out of range. Received ${offset}`);
        err.code = 'ERR_OUT_OF_RANGE';
        throw err;
      }
      if (encoding === undefined && typeof length === 'string') {
        encoding = length;
        length = this.length - offset;
      } else {
        length = length === undefined ? this.length - offset : length;
        encoding = encoding === undefined ? 'utf8' : encoding;
      }
    }

    if (typeof length !== 'number') {
      const err = new TypeError(`The "length" argument must be of type number. Received type ${typeof length}`);
      err.code = 'ERR_INVALID_ARG_TYPE';
      throw err;
    }
    if (!Number.isInteger(length) || length < 0 || length > this.length) {
      const err = new RangeError(`The value of "length" is out of range. Received ${length}`);
      err.code = 'ERR_OUT_OF_RANGE';
      throw err;
    }
    const maxBytes = Math.min(length, this.length - offset);
    if (maxBytes <= 0) {
      return 0;
    }

    const enc = normalizeEncoding(encoding);
    if (enc === 'utf8') {
      return utf8EncodeInto(string, this, offset, maxBytes).written;
    }

    const bytes = encodeString(string, enc);
    const toWrite = enc === 'utf16le'
      ? Math.min(bytes.length, maxBytes & ~1)
      : Math.min(bytes.length, maxBytes);
    for (let i = 0; i < toWrite; i++) {
      this[offset + i] = bytes[i];
    }
    return toWrite;
  }

  fill(value, offset = 0, end = this.length, encoding = 'utf8') {
    if (typeof offset === 'string') {
      encoding = offset;
      offset = 0;
      end = this.length;
    } else if (typeof end === 'string') {
      encoding = end;
      end = this.length;
    }
    const start = Math.max(0, Math.min(this.length, Math.trunc(Number(offset) || 0)));
    const stop = Math.max(0, Math.min(this.length, Math.trunc(Number(end) || 0)));
    if (stop <= start) {
      return this;
    }

    if (typeof value === 'number') {
      const b = value & 0xff;
      for (let i = start; i < stop; i++) {
        this[i] = b;
      }
      return this;
    }

    let pattern;
    if (typeof value === 'string') {
      if (value.length === 0) {
        pattern = [0];
      } else {
        pattern = encodeString(value, encoding);
        if (pattern.length === 0) {
          pattern = [0];
        }
      }
    } else if (value instanceof Uint8Array) {
      pattern = value.length > 0 ? value : [0];
    } else {
      pattern = [Number(value) & 0xff];
    }

    const pLen = pattern.length;
    for (let i = start; i < stop; i++) {
      this[i] = pattern[(i - start) % pLen];
    }
    return this;
  }

  slice(start, end) {
    return this.subarray(start, end);
  }

  copy(target, targetStart = 0, sourceStart = 0, sourceEnd = this.length) {
    if (!(target instanceof Uint8Array)) {
      throw new TypeError('The "target" argument must be an instance of Buffer or Uint8Array.');
    }
    const tStart = Math.trunc(Number(targetStart) || 0);
    const sStart = Math.trunc(Number(sourceStart) || 0);
    const sEnd = Math.trunc(sourceEnd === undefined ? this.length : Number(sourceEnd) || 0);
    if (tStart < 0 || sStart < 0 || sEnd < 0) {
      throw new RangeError('Index out of range');
    }
    if (tStart >= target.length || sStart >= this.length || sEnd <= sStart) {
      return 0;
    }
    const clampedEnd = Math.min(sEnd, this.length);
    const len = Math.min(clampedEnd - sStart, target.length - tStart);
    target.set(this.subarray(sStart, sStart + len), tStart);
    return len;
  }

  equals(otherBuffer) {
    if (!(otherBuffer instanceof Uint8Array)) {
      throw new TypeError('The "otherBuffer" argument must be an instance of Buffer or Uint8Array.');
    }
    return Buffer.compare(this, otherBuffer) === 0;
  }

  compare(target, targetStart = 0, targetEnd, sourceStart = 0, sourceEnd) {
    if (!(target instanceof Uint8Array)) {
      throw new TypeError('The "target" argument must be an instance of Buffer or Uint8Array.');
    }
    const tStart = Math.max(0, Math.trunc(Number(targetStart) || 0));
    const tEnd = targetEnd === undefined ? target.length : Math.min(target.length, Math.trunc(Number(targetEnd) || 0));
    const sStart = Math.max(0, Math.trunc(Number(sourceStart) || 0));
    const sEnd = sourceEnd === undefined ? this.length : Math.min(this.length, Math.trunc(Number(sourceEnd) || 0));
    return Buffer.compare(this.subarray(sStart, sEnd), target.subarray(tStart, tEnd));
  }

  _needleBytes(value, encoding) {
    if (typeof value === 'number') {
      return [value & 0xff];
    }
    if (typeof value === 'string') {
      return encodeString(value, encoding);
    }
    if (value instanceof Uint8Array) {
      return value;
    }
    throw new TypeError('The "value" argument must be of type number, string, Buffer, or Uint8Array.');
  }

  indexOf(value, byteOffset = 0, encoding = 'utf8') {
    if (typeof byteOffset === 'string') {
      encoding = byteOffset;
      byteOffset = 0;
    }
    const needle = this._needleBytes(value, encoding);
    let offset = Math.trunc(Number(byteOffset) || 0);
    if (offset < 0) {
      offset = Math.max(0, this.length + offset);
    }
    if (needle.length === 0) {
      return offset <= this.length ? offset : this.length;
    }
    if (offset + needle.length > this.length) {
      return -1;
    }
    const limit = this.length - needle.length;
    for (let i = offset; i <= limit; i++) {
      let matched = true;
      for (let j = 0; j < needle.length; j++) {
        if (this[i + j] !== needle[j]) {
          matched = false;
          break;
        }
      }
      if (matched) {
        return i;
      }
    }
    return -1;
  }

  lastIndexOf(value, byteOffset = this.length - 1, encoding = 'utf8') {
    if (typeof byteOffset === 'string') {
      encoding = byteOffset;
      byteOffset = this.length - 1;
    }
    const needle = this._needleBytes(value, encoding);
    if (this.length === 0 && needle.length > 0) {
      return -1;
    }
    let offset = Math.trunc(Number(byteOffset));
    if (Number.isNaN(offset)) {
      offset = this.length - 1;
    }
    if (offset < 0) {
      offset = this.length + offset;
    }
    if (offset < 0) {
      return -1;
    }
    if (needle.length === 0) {
      return Math.min(offset, this.length);
    }
    const start = Math.min(offset, this.length - needle.length);
    for (let i = start; i >= 0; i--) {
      let matched = true;
      for (let j = 0; j < needle.length; j++) {
        if (this[i + j] !== needle[j]) {
          matched = false;
          break;
        }
      }
      if (matched) {
        return i;
      }
    }
    return -1;
  }

  includes(value, byteOffset = 0, encoding = 'utf8') {
    return this.indexOf(value, byteOffset, encoding) !== -1;
  }

  toJSON() {
    return {
      type: 'Buffer',
      data: Array.from(this),
    };
  }

  swap16() {
    if (this.length % 2 !== 0) {
      throw new RangeError('Buffer size must be a multiple of 16-bits');
    }
    for (let i = 0; i < this.length; i += 2) {
      const t = this[i];
      this[i] = this[i + 1];
      this[i + 1] = t;
    }
    return this;
  }

  swap32() {
    if (this.length % 4 !== 0) {
      throw new RangeError('Buffer size must be a multiple of 32-bits');
    }
    for (let i = 0; i < this.length; i += 4) {
      const b0 = this[i];
      const b1 = this[i + 1];
      this[i] = this[i + 3];
      this[i + 1] = this[i + 2];
      this[i + 2] = b1;
      this[i + 3] = b0;
    }
    return this;
  }

  swap64() {
    if (this.length % 8 !== 0) {
      throw new RangeError('Buffer size must be a multiple of 64-bits');
    }
    for (let i = 0; i < this.length; i += 8) {
      for (let j = 0; j < 4; j++) {
        const t = this[i + j];
        this[i + j] = this[i + 7 - j];
        this[i + 7 - j] = t;
      }
    }
    return this;
  }

  readUInt8(offset = 0) {
    checkOffset(offset, 1, this.length);
    return this[offset];
  }

  readInt8(offset = 0) {
    checkOffset(offset, 1, this.length);
    const v = this[offset];
    return v & 0x80 ? v - 0x100 : v;
  }

  writeUInt8(value, offset = 0) {
    checkIntRange(value, 0, 0xff);
    checkOffset(offset, 1, this.length);
    this[offset] = value;
    return offset + 1;
  }

  writeInt8(value, offset = 0) {
    checkIntRange(value, -0x80, 0x7f);
    checkOffset(offset, 1, this.length);
    this[offset] = value < 0 ? value + 0x100 : value;
    return offset + 1;
  }

  readUInt16BE(offset = 0) {
    checkOffset(offset, 2, this.length);
    return (this[offset] << 8) | this[offset + 1];
  }

  readUInt16LE(offset = 0) {
    checkOffset(offset, 2, this.length);
    return this[offset] | (this[offset + 1] << 8);
  }

  readInt16BE(offset = 0) {
    checkOffset(offset, 2, this.length);
    const v = (this[offset] << 8) | this[offset + 1];
    return v & 0x8000 ? v - 0x10000 : v;
  }

  readInt16LE(offset = 0) {
    checkOffset(offset, 2, this.length);
    const v = this[offset] | (this[offset + 1] << 8);
    return v & 0x8000 ? v - 0x10000 : v;
  }

  writeUInt16BE(value, offset = 0) {
    checkIntRange(value, 0, 0xffff);
    checkOffset(offset, 2, this.length);
    this[offset] = (value >>> 8) & 0xff;
    this[offset + 1] = value & 0xff;
    return offset + 2;
  }

  writeUInt16LE(value, offset = 0) {
    checkIntRange(value, 0, 0xffff);
    checkOffset(offset, 2, this.length);
    this[offset] = value & 0xff;
    this[offset + 1] = (value >>> 8) & 0xff;
    return offset + 2;
  }

  writeInt16BE(value, offset = 0) {
    checkIntRange(value, -0x8000, 0x7fff);
    checkOffset(offset, 2, this.length);
    const u = value < 0 ? value + 0x10000 : value;
    this[offset] = (u >>> 8) & 0xff;
    this[offset + 1] = u & 0xff;
    return offset + 2;
  }

  writeInt16LE(value, offset = 0) {
    checkIntRange(value, -0x8000, 0x7fff);
    checkOffset(offset, 2, this.length);
    const u = value < 0 ? value + 0x10000 : value;
    this[offset] = u & 0xff;
    this[offset + 1] = (u >>> 8) & 0xff;
    return offset + 2;
  }

  readUInt32BE(offset = 0) {
    checkOffset(offset, 4, this.length);
    return (
      ((this[offset] * 0x1000000) +
        ((this[offset + 1] << 16) | (this[offset + 2] << 8) | this[offset + 3])) >>>
      0
    );
  }

  readUInt32LE(offset = 0) {
    checkOffset(offset, 4, this.length);
    return (
      ((this[offset] | (this[offset + 1] << 8) | (this[offset + 2] << 16)) +
        this[offset + 3] * 0x1000000) >>>
      0
    );
  }

  readInt32BE(offset = 0) {
    checkOffset(offset, 4, this.length);
    return (
      (this[offset] << 24) |
      (this[offset + 1] << 16) |
      (this[offset + 2] << 8) |
      this[offset + 3]
    );
  }

  readInt32LE(offset = 0) {
    checkOffset(offset, 4, this.length);
    return (
      this[offset] |
      (this[offset + 1] << 8) |
      (this[offset + 2] << 16) |
      (this[offset + 3] << 24)
    );
  }

  writeUInt32BE(value, offset = 0) {
    checkIntRange(value, 0, 0xffffffff);
    checkOffset(offset, 4, this.length);
    this[offset] = (value >>> 24) & 0xff;
    this[offset + 1] = (value >>> 16) & 0xff;
    this[offset + 2] = (value >>> 8) & 0xff;
    this[offset + 3] = value & 0xff;
    return offset + 4;
  }

  writeUInt32LE(value, offset = 0) {
    checkIntRange(value, 0, 0xffffffff);
    checkOffset(offset, 4, this.length);
    this[offset] = value & 0xff;
    this[offset + 1] = (value >>> 8) & 0xff;
    this[offset + 2] = (value >>> 16) & 0xff;
    this[offset + 3] = (value >>> 24) & 0xff;
    return offset + 4;
  }

  writeInt32BE(value, offset = 0) {
    checkIntRange(value, -0x80000000, 0x7fffffff);
    checkOffset(offset, 4, this.length);
    const u = value >>> 0;
    this[offset] = (u >>> 24) & 0xff;
    this[offset + 1] = (u >>> 16) & 0xff;
    this[offset + 2] = (u >>> 8) & 0xff;
    this[offset + 3] = u & 0xff;
    return offset + 4;
  }

  writeInt32LE(value, offset = 0) {
    checkIntRange(value, -0x80000000, 0x7fffffff);
    checkOffset(offset, 4, this.length);
    const u = value >>> 0;
    this[offset] = u & 0xff;
    this[offset + 1] = (u >>> 8) & 0xff;
    this[offset + 2] = (u >>> 16) & 0xff;
    this[offset + 3] = (u >>> 24) & 0xff;
    return offset + 4;
  }

  readFloatBE(offset = 0) {
    checkOffset(offset, 4, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 4).getFloat32(0, false);
  }

  readFloatLE(offset = 0) {
    checkOffset(offset, 4, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 4).getFloat32(0, true);
  }

  writeFloatBE(value, offset = 0) {
    checkOffset(offset, 4, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 4).setFloat32(0, value, false);
    return offset + 4;
  }

  writeFloatLE(value, offset = 0) {
    checkOffset(offset, 4, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 4).setFloat32(0, value, true);
    return offset + 4;
  }

  readDoubleBE(offset = 0) {
    checkOffset(offset, 8, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 8).getFloat64(0, false);
  }

  readDoubleLE(offset = 0) {
    checkOffset(offset, 8, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 8).getFloat64(0, true);
  }

  writeDoubleBE(value, offset = 0) {
    checkOffset(offset, 8, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 8).setFloat64(0, value, false);
    return offset + 8;
  }

  writeDoubleLE(value, offset = 0) {
    checkOffset(offset, 8, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 8).setFloat64(0, value, true);
    return offset + 8;
  }

  readBigUInt64BE(offset = 0) {
    checkOffset(offset, 8, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 8).getBigUint64(0, false);
  }

  readBigUInt64LE(offset = 0) {
    checkOffset(offset, 8, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 8).getBigUint64(0, true);
  }

  readBigInt64BE(offset = 0) {
    checkOffset(offset, 8, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 8).getBigInt64(0, false);
  }

  readBigInt64LE(offset = 0) {
    checkOffset(offset, 8, this.length);
    return new DataView(this.buffer, this.byteOffset + offset, 8).getBigInt64(0, true);
  }

  writeBigUInt64BE(value, offset = 0) {
    checkOffset(offset, 8, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 8).setBigUint64(0, BigInt(value), false);
    return offset + 8;
  }

  writeBigUInt64LE(value, offset = 0) {
    checkOffset(offset, 8, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 8).setBigUint64(0, BigInt(value), true);
    return offset + 8;
  }

  writeBigInt64BE(value, offset = 0) {
    checkOffset(offset, 8, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 8).setBigInt64(0, BigInt(value), false);
    return offset + 8;
  }

  writeBigInt64LE(value, offset = 0) {
    checkOffset(offset, 8, this.length);
    new DataView(this.buffer, this.byteOffset + offset, 8).setBigInt64(0, BigInt(value), true);
    return offset + 8;
  }
}

// Lowercase Uint aliases matching Node.js Buffer API
Buffer.prototype.readUint8 = Buffer.prototype.readUInt8;
Buffer.prototype.writeUint8 = Buffer.prototype.writeUInt8;
Buffer.prototype.readUint16BE = Buffer.prototype.readUInt16BE;
Buffer.prototype.readUint16LE = Buffer.prototype.readUInt16LE;
Buffer.prototype.writeUint16BE = Buffer.prototype.writeUInt16BE;
Buffer.prototype.writeUint16LE = Buffer.prototype.writeUInt16LE;
Buffer.prototype.readUint32BE = Buffer.prototype.readUInt32BE;
Buffer.prototype.readUint32LE = Buffer.prototype.readUInt32LE;
Buffer.prototype.writeUint32BE = Buffer.prototype.writeUInt32BE;
Buffer.prototype.writeUint32LE = Buffer.prototype.writeUInt32LE;
Buffer.prototype.readBigUint64BE = Buffer.prototype.readBigUInt64BE;
Buffer.prototype.readBigUint64LE = Buffer.prototype.readBigUInt64LE;
Buffer.prototype.writeBigUint64BE = Buffer.prototype.writeBigUInt64BE;
Buffer.prototype.writeBigUint64LE = Buffer.prototype.writeBigUInt64LE;

function SlowBuffer(size) {
  assertSize(size);
  return Buffer.alloc(size);
}
SlowBuffer.prototype = Buffer.prototype;

module.exports = {
  Buffer,
  SlowBuffer,
  kMaxLength,
  INSPECT_MAX_BYTES,
  constants,
  utf8ToBytes,
  utf8EncodeInto,
  bytesToUtf8,
  base64ToBytes,
  bytesToBase64,
  encodeString,
  decodeBytes,
};
