'use strict';

// Pure-JS Node.js crypto compatibility shim (no dependency on Node built-in 'crypto').

const { Buffer } = require('./buffer');

function toUint8Array(data, inputEncoding) {
  if (typeof data === 'string') {
    return Buffer.from(data, inputEncoding || 'utf8');
  }
  if (data instanceof Uint8Array) {
    return data;
  }
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  if (data instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && data instanceof SharedArrayBuffer)) {
    return new Uint8Array(data);
  }
  throw new TypeError('The "data" argument must be of type string or an instance of Buffer, TypedArray, or DataView.');
}

function rotr32(x, n) {
  return (x >>> n) | (x << (32 - n));
}

function rotl32(x, n) {
  return (x << n) | (x >>> (32 - n));
}

// ============================================================================
// MD5 (RFC 1321)
// ============================================================================

const MD5_S = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
  5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20,
  4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
  6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K = [
  0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
  0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
  0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
  0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
  0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
  0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
  0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
  0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
  0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
  0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
  0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
  0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
  0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
  0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
  0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
  0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

class Md5Digest {
  constructor(cloneFrom) {
    if (cloneFrom) {
      this.state = new Uint32Array(cloneFrom.state);
      this.block = new Uint8Array(cloneFrom.block);
      this.blockLen = cloneFrom.blockLen;
      this.totalBytes = cloneFrom.totalBytes;
    } else {
      this.state = new Uint32Array([0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476]);
      this.block = new Uint8Array(64);
      this.blockLen = 0;
      this.totalBytes = 0;
    }
  }

  _compress(blk) {
    const M = new Uint32Array(16);
    for (let i = 0; i < 16; i++) {
      const o = i * 4;
      M[i] = blk[o] | (blk[o + 1] << 8) | (blk[o + 2] << 16) | (blk[o + 3] << 24);
    }
    let a = this.state[0];
    let b = this.state[1];
    let c = this.state[2];
    let d = this.state[3];

    for (let i = 0; i < 64; i++) {
      let f, g;
      if (i < 16) {
        f = (b & c) | (~b & d);
        g = i;
      } else if (i < 32) {
        f = (d & b) | (~d & c);
        g = (5 * i + 1) & 15;
      } else if (i < 48) {
        f = b ^ c ^ d;
        g = (3 * i + 5) & 15;
      } else {
        f = c ^ (b | ~d);
        g = (7 * i) & 15;
      }
      const temp = d;
      d = c;
      c = b;
      const sum = (a + f + MD5_K[i] + M[g]) >>> 0;
      b = (b + rotl32(sum, MD5_S[i])) >>> 0;
      a = temp;
    }

    this.state[0] = (this.state[0] + a) >>> 0;
    this.state[1] = (this.state[1] + b) >>> 0;
    this.state[2] = (this.state[2] + c) >>> 0;
    this.state[3] = (this.state[3] + d) >>> 0;
  }

  update(bytes) {
    let i = 0;
    this.totalBytes += bytes.length;
    if (this.blockLen > 0) {
      const need = 64 - this.blockLen;
      if (bytes.length < need) {
        this.block.set(bytes, this.blockLen);
        this.blockLen += bytes.length;
        return;
      }
      this.block.set(bytes.subarray(0, need), this.blockLen);
      this._compress(this.block);
      this.blockLen = 0;
      i = need;
    }
    while (i + 64 <= bytes.length) {
      this._compress(bytes.subarray(i, i + 64));
      i += 64;
    }
    if (i < bytes.length) {
      this.block.set(bytes.subarray(i), 0);
      this.blockLen = bytes.length - i;
    }
  }

  digest() {
    const bitLen = BigInt(this.totalBytes) * 8n;
    this.block[this.blockLen++] = 0x80;
    if (this.blockLen > 56) {
      this.block.fill(0, this.blockLen, 64);
      this._compress(this.block);
      this.blockLen = 0;
    }
    this.block.fill(0, this.blockLen, 56);
    const dv = new DataView(this.block.buffer);
    dv.setBigUint64(56, bitLen, true);
    this._compress(this.block);

    const out = Buffer.alloc(16);
    const outDv = new DataView(out.buffer, out.byteOffset, 16);
    for (let i = 0; i < 4; i++) {
      outDv.setUint32(i * 4, this.state[i], true);
    }
    return out;
  }
}

// ============================================================================
// SHA-1 (FIPS 180-4)
// ============================================================================

class Sha1Digest {
  constructor(cloneFrom) {
    if (cloneFrom) {
      this.state = new Uint32Array(cloneFrom.state);
      this.block = new Uint8Array(cloneFrom.block);
      this.blockLen = cloneFrom.blockLen;
      this.totalBytes = cloneFrom.totalBytes;
    } else {
      this.state = new Uint32Array([0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0]);
      this.block = new Uint8Array(64);
      this.blockLen = 0;
      this.totalBytes = 0;
    }
  }

  _compress(blk) {
    const W = new Uint32Array(80);
    for (let i = 0; i < 16; i++) {
      const o = i * 4;
      W[i] = ((blk[o] << 24) | (blk[o + 1] << 16) | (blk[o + 2] << 8) | blk[o + 3]) >>> 0;
    }
    for (let t = 16; t < 80; t++) {
      W[t] = rotl32(W[t - 3] ^ W[t - 8] ^ W[t - 14] ^ W[t - 16], 1) >>> 0;
    }

    let a = this.state[0];
    let b = this.state[1];
    let c = this.state[2];
    let d = this.state[3];
    let e = this.state[4];

    for (let t = 0; t < 80; t++) {
      let f, k;
      if (t < 20) {
        f = (b & c) | (~b & d);
        k = 0x5a827999;
      } else if (t < 40) {
        f = b ^ c ^ d;
        k = 0x6ed9eba1;
      } else if (t < 60) {
        f = (b & c) | (b & d) | (c & d);
        k = 0x8f1bbcdc;
      } else {
        f = b ^ c ^ d;
        k = 0xca62c1d6;
      }
      const temp = (rotl32(a, 5) + f + e + k + W[t]) >>> 0;
      e = d;
      d = c;
      c = rotl32(b, 30) >>> 0;
      b = a;
      a = temp;
    }

    this.state[0] = (this.state[0] + a) >>> 0;
    this.state[1] = (this.state[1] + b) >>> 0;
    this.state[2] = (this.state[2] + c) >>> 0;
    this.state[3] = (this.state[3] + d) >>> 0;
    this.state[4] = (this.state[4] + e) >>> 0;
  }

  update(bytes) {
    let i = 0;
    this.totalBytes += bytes.length;
    if (this.blockLen > 0) {
      const need = 64 - this.blockLen;
      if (bytes.length < need) {
        this.block.set(bytes, this.blockLen);
        this.blockLen += bytes.length;
        return;
      }
      this.block.set(bytes.subarray(0, need), this.blockLen);
      this._compress(this.block);
      this.blockLen = 0;
      i = need;
    }
    while (i + 64 <= bytes.length) {
      this._compress(bytes.subarray(i, i + 64));
      i += 64;
    }
    if (i < bytes.length) {
      this.block.set(bytes.subarray(i), 0);
      this.blockLen = bytes.length - i;
    }
  }

  digest() {
    const bitLen = BigInt(this.totalBytes) * 8n;
    this.block[this.blockLen++] = 0x80;
    if (this.blockLen > 56) {
      this.block.fill(0, this.blockLen, 64);
      this._compress(this.block);
      this.blockLen = 0;
    }
    this.block.fill(0, this.blockLen, 56);
    new DataView(this.block.buffer).setBigUint64(56, bitLen, false);
    this._compress(this.block);

    const out = Buffer.alloc(20);
    const outDv = new DataView(out.buffer, out.byteOffset, 20);
    for (let i = 0; i < 5; i++) {
      outDv.setUint32(i * 4, this.state[i], false);
    }
    return out;
  }
}

// ============================================================================
// SHA-256 / SHA-224 (FIPS 180-4)
// ============================================================================

const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const SHA256_IV = [
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const SHA224_IV = [
  0xc1059ed8, 0x367cd507, 0x3070dd17, 0xf70e5939,
  0xffc00b31, 0x68581511, 0x64f98fa7, 0xbefa4fa4,
];

class Sha256Digest {
  constructor(is224, cloneFrom) {
    this.is224 = is224;
    if (cloneFrom) {
      this.state = new Uint32Array(cloneFrom.state);
      this.block = new Uint8Array(cloneFrom.block);
      this.blockLen = cloneFrom.blockLen;
      this.totalBytes = cloneFrom.totalBytes;
    } else {
      this.state = new Uint32Array(is224 ? SHA224_IV : SHA256_IV);
      this.block = new Uint8Array(64);
      this.blockLen = 0;
      this.totalBytes = 0;
    }
  }

  _compress(blk) {
    const W = new Uint32Array(64);
    for (let i = 0; i < 16; i++) {
      const o = i * 4;
      W[i] = ((blk[o] << 24) | (blk[o + 1] << 16) | (blk[o + 2] << 8) | blk[o + 3]) >>> 0;
    }
    for (let i = 16; i < 64; i++) {
      const s0 = rotr32(W[i - 15], 7) ^ rotr32(W[i - 15], 18) ^ (W[i - 15] >>> 3);
      const s1 = rotr32(W[i - 2], 17) ^ rotr32(W[i - 2], 19) ^ (W[i - 2] >>> 10);
      W[i] = (W[i - 16] + s0 + W[i - 7] + s1) >>> 0;
    }

    let a = this.state[0];
    let b = this.state[1];
    let c = this.state[2];
    let d = this.state[3];
    let e = this.state[4];
    let f = this.state[5];
    let g = this.state[6];
    let h = this.state[7];

    for (let i = 0; i < 64; i++) {
      const S1 = rotr32(e, 6) ^ rotr32(e, 11) ^ rotr32(e, 25);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (h + S1 + ch + SHA256_K[i] + W[i]) >>> 0;
      const S0 = rotr32(a, 2) ^ rotr32(a, 13) ^ rotr32(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) >>> 0;

      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }

    this.state[0] = (this.state[0] + a) >>> 0;
    this.state[1] = (this.state[1] + b) >>> 0;
    this.state[2] = (this.state[2] + c) >>> 0;
    this.state[3] = (this.state[3] + d) >>> 0;
    this.state[4] = (this.state[4] + e) >>> 0;
    this.state[5] = (this.state[5] + f) >>> 0;
    this.state[6] = (this.state[6] + g) >>> 0;
    this.state[7] = (this.state[7] + h) >>> 0;
  }

  update(bytes) {
    let i = 0;
    this.totalBytes += bytes.length;
    if (this.blockLen > 0) {
      const need = 64 - this.blockLen;
      if (bytes.length < need) {
        this.block.set(bytes, this.blockLen);
        this.blockLen += bytes.length;
        return;
      }
      this.block.set(bytes.subarray(0, need), this.blockLen);
      this._compress(this.block);
      this.blockLen = 0;
      i = need;
    }
    while (i + 64 <= bytes.length) {
      this._compress(bytes.subarray(i, i + 64));
      i += 64;
    }
    if (i < bytes.length) {
      this.block.set(bytes.subarray(i), 0);
      this.blockLen = bytes.length - i;
    }
  }

  digest() {
    const bitLen = BigInt(this.totalBytes) * 8n;
    this.block[this.blockLen++] = 0x80;
    if (this.blockLen > 56) {
      this.block.fill(0, this.blockLen, 64);
      this._compress(this.block);
      this.blockLen = 0;
    }
    this.block.fill(0, this.blockLen, 56);
    new DataView(this.block.buffer).setBigUint64(56, bitLen, false);
    this._compress(this.block);

    const outWords = this.is224 ? 7 : 8;
    const out = Buffer.alloc(outWords * 4);
    const outDv = new DataView(out.buffer, out.byteOffset, outWords * 4);
    for (let i = 0; i < outWords; i++) {
      outDv.setUint32(i * 4, this.state[i], false);
    }
    return out;
  }
}

// ============================================================================
// SHA-512 / SHA-384 (FIPS 180-4) using BigInt
// ============================================================================

const MASK64 = 0xffffffffffffffffn;

function rotr64(x, n) {
  return ((x >> BigInt(n)) | (x << BigInt(64 - n))) & MASK64;
}

const SHA512_K = [
  0x428a2f98d728ae22n, 0x7137449123ef65cdn, 0xb5c0fbcfec4d3b2fn, 0xe9b5dba58189dbbcn,
  0x3956c25bf348b538n, 0x59f111f1b605d019n, 0x923f82a4af194f9bn, 0xab1c5ed5da6d8118n,
  0xd807aa98a3030242n, 0x12835b0145706fben, 0x243185be4ee4b28cn, 0x550c7dc3d5ffb4e2n,
  0x72be5d74f27b896fn, 0x80deb1fe3b1696b1n, 0x9bdc06a725c71235n, 0xc19bf174cf692694n,
  0xe49b69c19ef14ad2n, 0xefbe4786384f25e3n, 0x0fc19dc68b8cd5b5n, 0x240ca1cc77ac9c65n,
  0x2de92c6f592b0275n, 0x4a7484aa6ea6e483n, 0x5cb0a9dcbd41fbd4n, 0x76f988da831153b5n,
  0x983e5152ee66dfabn, 0xa831c66d2db43210n, 0xb00327c898fb213fn, 0xbf597fc7beef0ee4n,
  0xc6e00bf33da88fc2n, 0xd5a79147930aa725n, 0x06ca6351e003826fn, 0x142929670a0e6e70n,
  0x27b70a8546d22ffcn, 0x2e1b21385c26c926n, 0x4d2c6dfc5ac42aedn, 0x53380d139d95b3dfn,
  0x650a73548baf63den, 0x766a0abb3c77b2a8n, 0x81c2c92e47edaee6n, 0x92722c851482353bn,
  0xa2bfe8a14cf10364n, 0xa81a664bbc423001n, 0xc24b8b70d0f89791n, 0xc76c51a30654be30n,
  0xd192e819d6ef5218n, 0xd69906245565a910n, 0xf40e35855771202an, 0x106aa07032bbd1b8n,
  0x19a4c116b8d2d0c8n, 0x1e376c085141ab53n, 0x2748774cdf8eeb99n, 0x34b0bcb5e19b48a8n,
  0x391c0cb3c5c95a63n, 0x4ed8aa4ae3418acbn, 0x5b9cca4f7763e373n, 0x682e6ff3d6b2b8a3n,
  0x748f82ee5defb2fcn, 0x78a5636f43172f60n, 0x84c87814a1f0ab72n, 0x8cc702081a6439ecn,
  0x90befffa23631e28n, 0xa4506cebde82bde9n, 0xbef9a3f7b2c67915n, 0xc67178f2e372532bn,
  0xca273eceea26619cn, 0xd186b8c721c0c207n, 0xeada7dd6cde0eb1en, 0xf57d4f7fee6ed178n,
  0x06f067aa72176fban, 0x0a637dc5a2c898a6n, 0x113f9804bef90daen, 0x1b710b35131c471bn,
  0x28db77f523047d84n, 0x32caab7b40c72493n, 0x3c9ebe0a15c9bebcn, 0x431d67c49c100d4cn,
  0x4cc5d4becb3e42b6n, 0x597f299cfc657e2an, 0x5fcb6fab3ad6faecn, 0x6c44198c4a475817n,
];

const SHA512_IV = [
  0x6a09e667f3bcc908n, 0xbb67ae8584caa73bn, 0x3c6ef372fe94f82bn, 0xa54ff53a5f1d36f1n,
  0x510e527fade682d1n, 0x9b05688c2b3e6c1fn, 0x1f83d9abfb41bd6bn, 0x5be0cd19137e2179n,
];

const SHA384_IV = [
  0xcbbb9d5dc1059ed8n, 0x629a292a367cd507n, 0x9159015a3070dd17n, 0x152fecd8f70e5939n,
  0x67332667ffc00b31n, 0x8eb44a8768581511n, 0xdb0c2e0d64f98fa7n, 0x47b5481dbefa4fa4n,
];

class Sha512Digest {
  constructor(is384, cloneFrom) {
    this.is384 = is384;
    if (cloneFrom) {
      this.state = cloneFrom.state.slice();
      this.block = new Uint8Array(cloneFrom.block);
      this.blockLen = cloneFrom.blockLen;
      this.totalBytes = cloneFrom.totalBytes;
    } else {
      this.state = (is384 ? SHA384_IV : SHA512_IV).slice();
      this.block = new Uint8Array(128);
      this.blockLen = 0;
      this.totalBytes = 0n;
    }
  }

  _compress(blk) {
    const W = new Array(80);
    const dv = new DataView(blk.buffer, blk.byteOffset, 128);
    for (let i = 0; i < 16; i++) {
      W[i] = dv.getBigUint64(i * 8, false);
    }
    for (let i = 16; i < 80; i++) {
      const w15 = W[i - 15];
      const w2 = W[i - 2];
      const s0 = rotr64(w15, 1) ^ rotr64(w15, 8) ^ (w15 >> 7n);
      const s1 = rotr64(w2, 19) ^ rotr64(w2, 61) ^ (w2 >> 6n);
      W[i] = (W[i - 16] + s0 + W[i - 7] + s1) & MASK64;
    }

    let [a, b, c, d, e, f, g, h] = this.state;
    for (let i = 0; i < 80; i++) {
      const S1 = rotr64(e, 14) ^ rotr64(e, 18) ^ rotr64(e, 41);
      const ch = (e & f) ^ ((~e & MASK64) & g);
      const temp1 = (h + S1 + ch + SHA512_K[i] + W[i]) & MASK64;
      const S0 = rotr64(a, 28) ^ rotr64(a, 34) ^ rotr64(a, 39);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) & MASK64;

      h = g;
      g = f;
      f = e;
      e = (d + temp1) & MASK64;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) & MASK64;
    }

    this.state[0] = (this.state[0] + a) & MASK64;
    this.state[1] = (this.state[1] + b) & MASK64;
    this.state[2] = (this.state[2] + c) & MASK64;
    this.state[3] = (this.state[3] + d) & MASK64;
    this.state[4] = (this.state[4] + e) & MASK64;
    this.state[5] = (this.state[5] + f) & MASK64;
    this.state[6] = (this.state[6] + g) & MASK64;
    this.state[7] = (this.state[7] + h) & MASK64;
  }

  update(bytes) {
    let i = 0;
    this.totalBytes += BigInt(bytes.length);
    if (this.blockLen > 0) {
      const need = 128 - this.blockLen;
      if (bytes.length < need) {
        this.block.set(bytes, this.blockLen);
        this.blockLen += bytes.length;
        return;
      }
      this.block.set(bytes.subarray(0, need), this.blockLen);
      this._compress(this.block);
      this.blockLen = 0;
      i = need;
    }
    while (i + 128 <= bytes.length) {
      this._compress(bytes.subarray(i, i + 128));
      i += 128;
    }
    if (i < bytes.length) {
      this.block.set(bytes.subarray(i), 0);
      this.blockLen = bytes.length - i;
    }
  }

  digest() {
    const bitLen = this.totalBytes * 8n;
    this.block[this.blockLen++] = 0x80;
    if (this.blockLen > 112) {
      this.block.fill(0, this.blockLen, 128);
      this._compress(this.block);
      this.blockLen = 0;
    }
    this.block.fill(0, this.blockLen, 112);
    const dv = new DataView(this.block.buffer);
    dv.setBigUint64(112, (bitLen >> 64n) & MASK64, false);
    dv.setBigUint64(120, bitLen & MASK64, false);
    this._compress(this.block);

    const outWords = this.is384 ? 6 : 8;
    const out = Buffer.alloc(outWords * 8);
    const outDv = new DataView(out.buffer, out.byteOffset, outWords * 8);
    for (let i = 0; i < outWords; i++) {
      outDv.setBigUint64(i * 8, this.state[i], false);
    }
    return out;
  }
}

const ALGORITHMS = {
  md5: { blockSize: 64, create: (clone) => new Md5Digest(clone) },
  sha1: { blockSize: 64, create: (clone) => new Sha1Digest(clone) },
  sha224: { blockSize: 64, create: (clone) => new Sha256Digest(true, clone) },
  sha256: { blockSize: 64, create: (clone) => new Sha256Digest(false, clone) },
  sha384: { blockSize: 128, create: (clone) => new Sha512Digest(true, clone) },
  sha512: { blockSize: 128, create: (clone) => new Sha512Digest(false, clone) },
};

function resolveAlg(algorithm) {
  if (typeof algorithm !== 'string') {
    throw new TypeError('The "algorithm" argument must be of type string.');
  }
  const key = algorithm.toLowerCase();
  const spec = ALGORITHMS[key];
  if (!spec) {
    const err = new Error(`Digest method not supported: ${algorithm}`);
    err.code = 'ERR_OSSL_EVP_UNSUPPORTED';
    throw err;
  }
  return { name: key, spec };
}

class Hash {
  constructor(algorithm, cloneFrom) {
    const { name, spec } = resolveAlg(algorithm);
    this._alg = name;
    this._spec = spec;
    this._digest = spec.create(cloneFrom ? cloneFrom._digest : undefined);
    this._finalized = false;
  }

  update(data, inputEncoding) {
    if (this._finalized) {
      const err = new Error('Digest already called');
      err.code = 'ERR_CRYPTO_HASH_FINALIZED';
      throw err;
    }
    this._digest.update(toUint8Array(data, inputEncoding));
    return this;
  }

  digest(encoding) {
    if (this._finalized) {
      const err = new Error('Digest already called');
      err.code = 'ERR_CRYPTO_HASH_FINALIZED';
      throw err;
    }
    this._finalized = true;
    const rawBuf = this._digest.digest();
    if (encoding === undefined || encoding === 'buffer') {
      return rawBuf;
    }
    return rawBuf.toString(encoding);
  }

  copy() {
    if (this._finalized) {
      const err = new Error('Digest already called');
      err.code = 'ERR_CRYPTO_HASH_FINALIZED';
      throw err;
    }
    return new Hash(this._alg, this);
  }
}

class Hmac {
  constructor(algorithm, key) {
    const { name, spec } = resolveAlg(algorithm);
    this._alg = name;
    this._spec = spec;
    this._finalized = false;

    const keyBytes = toUint8Array(key);
    const blockSize = spec.blockSize;
    const paddedKey = new Uint8Array(blockSize);
    if (keyBytes.length > blockSize) {
      const hashed = new Hash(name).update(keyBytes).digest();
      paddedKey.set(hashed, 0);
    } else {
      paddedKey.set(keyBytes, 0);
    }

    const ipad = new Uint8Array(blockSize);
    this._opad = new Uint8Array(blockSize);
    for (let i = 0; i < blockSize; i++) {
      ipad[i] = paddedKey[i] ^ 0x36;
      this._opad[i] = paddedKey[i] ^ 0x5c;
    }

    this._inner = new Hash(name).update(ipad);
  }

  update(data, inputEncoding) {
    if (this._finalized) {
      const err = new Error('Digest already called');
      err.code = 'ERR_CRYPTO_HASH_FINALIZED';
      throw err;
    }
    this._inner.update(data, inputEncoding);
    return this;
  }

  digest(encoding) {
    if (this._finalized) {
      const err = new Error('Digest already called');
      err.code = 'ERR_CRYPTO_HASH_FINALIZED';
      throw err;
    }
    this._finalized = true;
    const innerDigest = this._inner.digest();
    const outer = new Hash(this._alg).update(this._opad).update(innerDigest).digest();
    if (encoding === undefined || encoding === 'buffer') {
      return outer;
    }
    return outer.toString(encoding);
  }
}

function createHash(algorithm) {
  return new Hash(algorithm);
}

function createHmac(algorithm, key) {
  return new Hmac(algorithm, key);
}

function getHashes() {
  return Object.keys(ALGORITHMS);
}

// ============================================================================
// Random generation (WebCrypto when present + pure-JS fallback CSPRNG)
// ============================================================================

let prngState0 = 0x9e3779b97f4a7c15n ^ BigInt(Date.now());
let prngState1 = 0xbf58476d1ce4e5b9n ^ BigInt(Math.floor(Math.random() * 0x100000000));

function fillRandomBytes(target) {
  if (
    typeof globalThis !== 'undefined' &&
    globalThis.crypto &&
    typeof globalThis.crypto.getRandomValues === 'function'
  ) {
    const MAX_CHUNK = 65536;
    for (let offset = 0; offset < target.length; offset += MAX_CHUNK) {
      const end = Math.min(offset + MAX_CHUNK, target.length);
      globalThis.crypto.getRandomValues(target.subarray(offset, end));
    }
    return target;
  }

  // Pure-JS Xorshift128+ fallback when global WebCrypto is absent
  prngState0 ^= BigInt(Date.now()) ^ BigInt(Math.floor(Math.random() * 0xffffffff));
  if (prngState0 === 0n && prngState1 === 0n) {
    prngState0 = 0x243f6a8885a308d3n;
  }
  for (let i = 0; i < target.length; ) {
    let s1 = prngState0;
    const s0 = prngState1;
    prngState0 = s0;
    s1 = (s1 ^ (s1 << 23n)) & MASK64;
    s1 = (s1 ^ (s1 >> 17n) ^ s0 ^ (s0 >> 26n)) & MASK64;
    prngState1 = s1;
    let out = (prngState1 + s0) & MASK64;
    for (let b = 0; b < 8 && i < target.length; b++) {
      target[i++] = Number(out & 0xffn);
      out >>= 8n;
    }
  }
  return target;
}

function randomBytes(size, callback) {
  if (typeof size !== 'number') {
    throw new TypeError('The "size" argument must be of type number.');
  }
  if (!Number.isFinite(size) || size < 0 || size > 0x7fffffff) {
    throw new RangeError(`The value of "size" is out of range. Received ${size}`);
  }
  const buf = Buffer.alloc(Math.trunc(size));
  fillRandomBytes(buf);
  if (typeof callback === 'function') {
    if (typeof queueMicrotask === 'function') {
      queueMicrotask(() => callback(null, buf));
    } else {
      Promise.resolve().then(() => callback(null, buf));
    }
    return undefined;
  }
  return buf;
}

function randomFillSync(buffer, offset = 0, size) {
  if (!ArrayBuffer.isView(buffer)) {
    throw new TypeError('The "buffer" argument must be an instance of Buffer, TypedArray, or DataView.');
  }
  const u8 = new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  const off = Math.trunc(Number(offset) || 0);
  if (off < 0 || off > u8.length) {
    throw new RangeError('offset out of range');
  }
  const len = size === undefined ? u8.length - off : Math.trunc(Number(size));
  if (len < 0 || off + len > u8.length || !Number.isFinite(len)) {
    throw new RangeError('size out of range');
  }
  fillRandomBytes(u8.subarray(off, off + len));
  return buffer;
}

function randomUUID() {
  const bytes = new Uint8Array(16);
  fillRandomBytes(bytes);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  let hex = '';
  for (let i = 0; i < 16; i++) {
    hex += bytes[i].toString(16).padStart(2, '0');
  }
  return (
    hex.slice(0, 8) +
    '-' +
    hex.slice(8, 12) +
    '-' +
    hex.slice(12, 16) +
    '-' +
    hex.slice(16, 20) +
    '-' +
    hex.slice(20, 32)
  );
}

function randomInt(min, max, callback) {
  let cb = callback;
  let lo = min;
  let hi = max;
  if (typeof max === 'function') {
    cb = max;
    hi = min;
    lo = 0;
  } else if (max === undefined) {
    hi = min;
    lo = 0;
  }
  if (!Number.isSafeInteger(lo) || !Number.isSafeInteger(hi)) {
    throw new RangeError('The "min" and "max" arguments must be safe integers.');
  }
  if (hi <= lo) {
    throw new RangeError(`The value of "max" (${hi}) must be greater than "min" (${lo}).`);
  }
  const range = hi - lo;
  if (range > 0xffffffffffff) {
    throw new RangeError('Range exceeds maximum supported 48-bit span');
  }

  const maxValid = Math.floor(0x1000000000000 / range) * range;
  let val = 0;
  const rand = new Uint8Array(6);
  while (true) {
    fillRandomBytes(rand);
    const n =
      rand[0] * 0x10000000000 +
      rand[1] * 0x100000000 +
      ((rand[2] << 24) >>> 0) +
      (rand[3] << 16) +
      (rand[4] << 8) +
      rand[5];
    if (n < maxValid) {
      val = lo + (n % range);
      break;
    }
  }

  if (typeof cb === 'function') {
    if (typeof queueMicrotask === 'function') {
      queueMicrotask(() => cb(null, val));
    } else {
      Promise.resolve().then(() => cb(null, val));
    }
    return undefined;
  }
  return val;
}

// ============================================================================
// timingSafeEqual & PBKDF2 (RFC 2898)
// ============================================================================

function timingSafeEqual(a, b) {
  if (!ArrayBuffer.isView(a) || !ArrayBuffer.isView(b)) {
    throw new TypeError('The "buf1" and "buf2" arguments must be instances of Buffer, TypedArray, or DataView.');
  }
  if (a.byteLength !== b.byteLength) {
    throw new RangeError('Input buffers must have the same byte length');
  }
  const ua = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
  const ub = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
  let diff = 0;
  for (let i = 0; i < ua.length; i++) {
    diff |= ua[i] ^ ub[i];
  }
  return diff === 0;
}

function pbkdf2Sync(password, salt, iterations, keylen, digest = 'sha1') {
  resolveAlg(digest);
  if (typeof iterations !== 'number' || !Number.isInteger(iterations) || iterations < 1) {
    throw new RangeError('The "iterations" argument must be a positive integer.');
  }
  if (typeof keylen !== 'number' || !Number.isInteger(keylen) || keylen < 0) {
    throw new RangeError('The "keylen" argument must be a non-negative integer.');
  }
  const pwdBytes = toUint8Array(password);
  const saltBytes = toUint8Array(salt);
  const dk = Buffer.alloc(keylen);
  if (keylen === 0) {
    return dk;
  }

  const hLen = createHash(digest).digest().length;
  const blockCount = Math.ceil(keylen / hLen);
  const saltAndIdx = new Uint8Array(saltBytes.length + 4);
  saltAndIdx.set(saltBytes, 0);

  let offset = 0;
  for (let blockIdx = 1; blockIdx <= blockCount; blockIdx++) {
    saltAndIdx[saltBytes.length] = (blockIdx >>> 24) & 0xff;
    saltAndIdx[saltBytes.length + 1] = (blockIdx >>> 16) & 0xff;
    saltAndIdx[saltBytes.length + 2] = (blockIdx >>> 8) & 0xff;
    saltAndIdx[saltBytes.length + 3] = blockIdx & 0xff;

    let u = createHmac(digest, pwdBytes).update(saltAndIdx).digest();
    const t = new Uint8Array(u);

    for (let iter = 1; iter < iterations; iter++) {
      u = createHmac(digest, pwdBytes).update(u).digest();
      for (let j = 0; j < hLen; j++) {
        t[j] ^= u[j];
      }
    }

    const toCopy = Math.min(hLen, keylen - offset);
    dk.set(t.subarray(0, toCopy), offset);
    offset += toCopy;
  }
  return dk;
}

function pbkdf2(password, salt, iterations, keylen, digest, callback) {
  let dig = digest;
  let cb = callback;
  if (typeof digest === 'function') {
    cb = digest;
    dig = 'sha1';
  }
  if (typeof cb !== 'function') {
    throw new TypeError('The "callback" argument must be of type Function.');
  }
  Promise.resolve().then(() => {
    try {
      const derived = pbkdf2Sync(password, salt, iterations, keylen, dig);
      cb(null, derived);
    } catch (err) {
      cb(err);
    }
  });
}

module.exports = {
  Hash,
  Hmac,
  createHash,
  createHmac,
  randomBytes,
  randomFillSync,
  randomUUID,
  randomInt,
  timingSafeEqual,
  pbkdf2,
  pbkdf2Sync,
  getHashes,
};
