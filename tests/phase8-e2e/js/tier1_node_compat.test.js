'use strict';

const assert = require('assert/strict');
const fs = require('fs');
const path = require('path');

const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const NODE_COMPAT_DIR = path.join(REPO_ROOT, 'ext-host-js', 'node-compat');

function createSuite(suiteName) {
  const tests = [];
  return {
    test(name, fn) {
      tests.push({ name, fn });
    },
    async run() {
      let filter = null;
      const argv = process.argv.slice(2);
      for (let i = 0; i < argv.length; i++) {
        if (argv[i] === '--filter' && i + 1 < argv.length) filter = argv[i + 1];
        else if (argv[i].startsWith('--filter=')) filter = argv[i].slice('--filter='.length);
      }
      const selected = filter ? tests.filter((t) => t.name.includes(filter)) : tests;
      process.stdout.write(`TAP version 13\n# Suite: ${suiteName}\n1..${selected.length}\n`);
      let passed = 0;
      let failed = 0;
      for (let i = 0; i < selected.length; i++) {
        const t = selected[i];
        const startedAt = Date.now();
        try {
          await t.fn();
          passed++;
          process.stdout.write(`ok ${i + 1} - ${t.name} (${Date.now() - startedAt}ms)\n`);
        } catch (err) {
          failed++;
          process.stdout.write(`not ok ${i + 1} - ${t.name} (${Date.now() - startedAt}ms)\n`);
          process.stdout.write(`  # ${err && err.stack ? err.stack.replace(/\n/g, '\n  # ') : String(err)}\n`);
        }
      }
      process.stdout.write(`# Summary (${suiteName}): ${passed}/${selected.length} passed, ${failed} failed\n`);
      if (failed > 0) process.exitCode = 1;
    },
  };
}

const suite = createSuite('tier1_node_compat');

suite.test('[C3-01] fs.js and util.js delegate Base64 and UTF-8 codecs to shared buffer.js helpers', () => {
  const fsSrc = fs.readFileSync(path.join(NODE_COMPAT_DIR, 'fs.js'), 'utf8');
  const utilSrc = fs.readFileSync(path.join(NODE_COMPAT_DIR, 'util.js'), 'utf8');
  assert.match(fsSrc, /require\(['"]\.\/buffer['"]\)/);
  assert.match(utilSrc, /require\(['"]\.\/buffer['"]\)/);
  assert.doesNotMatch(fsSrc, /function\s+decodeBase64Pure/);
  assert.doesNotMatch(fsSrc, /function\s+encodeBase64Pure/);
});

suite.test('[C3-02] buffer.js Base64 and Base64URL parity across 0x00..0xFF and unpadded inputs', () => {
  const compatBuffer = require(path.join(NODE_COMPAT_DIR, 'buffer.js'));
  const allBytes = new Uint8Array(256);
  for (let i = 0; i < 256; i++) allBytes[i] = i;

  const b64 = compatBuffer.bytesToBase64(allBytes);
  const roundtrip = compatBuffer.base64ToBytes(b64);
  assert.deepEqual(Array.from(roundtrip), Array.from(allBytes));

  // Unpadded + whitespace tolerant
  assert.equal(
    Buffer.from(compatBuffer.base64ToBytes(' YWJjZA \n')).toString('utf8'),
    'abcd'
  );

  // Base64URL roundtrip via compat Buffer
  const buf = compatBuffer.Buffer.from(allBytes);
  const b64url = buf.toString('base64url');
  assert.ok(!b64url.includes('+') && !b64url.includes('/') && !b64url.includes('='));
  assert.deepEqual(
    Array.from(compatBuffer.Buffer.from(b64url, 'base64url')),
    Array.from(allBytes)
  );
});

suite.test('[C3-03] util TextEncoder and TextDecoder parity (1..4 byte UTF-8, fatal, ignoreBOM)', () => {
  const compatUtil = require(path.join(NODE_COMPAT_DIR, 'util.js'));
  const enc = new compatUtil.TextEncoder();
  const encoded = enc.encode('aé€🚀');
  assert.deepEqual(
    Array.from(encoded),
    Array.from(Buffer.from('aé€🚀', 'utf8'))
  );

  const dec = new compatUtil.TextDecoder('utf-8');
  assert.equal(dec.decode(encoded), 'aé€🚀');
  assert.equal(dec.decode(new Uint8Array([0xf0, 0x9f, 0x98])), '\uFFFD');

  const fatalDec = new compatUtil.TextDecoder('utf-8', { fatal: true });
  assert.throws(() => fatalDec.decode(new Uint8Array([0xf0, 0x80])), TypeError);

  const bomBytes = new Uint8Array([0xef, 0xbb, 0xbf, 0x6f, 0x6b]);
  assert.equal(new compatUtil.TextDecoder('utf-8').decode(bomBytes), 'ok');
  assert.equal(new compatUtil.TextDecoder('utf-8', { ignoreBOM: true }).decode(bomBytes), '\uFEFFok');
});

suite.test('[M6-02] util.isDeepStrictEqual preserves Date(NaN) contract and closes edge cases', () => {
  const { isDeepStrictEqual } = require(path.join(NODE_COMPAT_DIR, 'util.js'));

  // Preserved invariant from ext-host-js/test.js
  assert.equal(isDeepStrictEqual(new Date(NaN), new Date(NaN)), false);

  // Boxed primitives with differing own properties
  assert.equal(
    isDeepStrictEqual(Object.assign(new Number(1), { a: 1 }), Object.assign(new Number(1), { a: 2 })),
    false
  );
  assert.equal(
    isDeepStrictEqual(Object.assign(new Number(1), { a: 1 }), Object.assign(new Number(1), { a: 1 })),
    true
  );

  // TypedArray own properties + Float64Array -0/+0 and NaN
  const u1 = Object.assign(new Uint8Array([1, 2]), { tag: 'a' });
  const u2 = Object.assign(new Uint8Array([1, 2]), { tag: 'b' });
  assert.equal(isDeepStrictEqual(u1, u2), false);
  assert.equal(isDeepStrictEqual(new Float64Array([NaN]), new Float64Array([NaN])), true);
  assert.equal(isDeepStrictEqual(new Float64Array([-0]), new Float64Array([0])), false);

  // Error.cause
  assert.equal(
    isDeepStrictEqual(new Error('x', { cause: 1 }), new Error('x', { cause: 2 })),
    false
  );
  assert.equal(
    isDeepStrictEqual(new Error('x', { cause: 1 }), new Error('x', { cause: 1 })),
    true
  );

  // Sparse arrays
  const sparse = new Array(1);
  assert.equal(isDeepStrictEqual(sparse, [undefined]), false);

  // Circular Set / Map graphs
  const s1 = new Set();
  const s2 = new Set();
  s1.add(s1);
  s2.add(s2);
  assert.equal(isDeepStrictEqual(s1, s2), true);

  const m1 = new Map();
  const m2 = new Map();
  const k1 = { id: 1 };
  const k2 = { id: 1 };
  m1.set(k1, m1);
  m2.set(k2, m2);
  assert.equal(isDeepStrictEqual(m1, m2), true);
});

suite.test('[ADV-01] rpc-bridge.js rejects >64 KiB headerless DoS, duplicate Content-Length, and +prefixed lengths', () => {
  const { PassThrough } = require('stream');
  const rpc = require(path.join(REPO_ROOT, 'ext-host-js', 'rpc-bridge.js'));

  const runTransportInput = (chunks) => {
    const input = new PassThrough();
    const output = new PassThrough();
    const transport = rpc.startStdioTransport({ input, output, redirectConsole: false });
    try {
      for (const chunk of chunks) {
        input.emit('data', chunk);
      }
      return {
        active: transport.active,
        bufferLen: transport.buffer.length,
      };
    } finally {
      rpc.stopStdioTransport();
      input.destroy();
      output.destroy();
    }
  };

  // 65+ KiB headerless junk must trip MAX_HEADER_BYTES and stop transport without retaining 70 KiB
  const dosRes = runTransportInput([Buffer.alloc(70 * 1024, 0x41)]);
  assert.equal(dosRes.active, false);
  assert.ok(dosRes.bufferLen <= 64 * 1024, `expected bounded buffer, got ${dosRes.bufferLen}`);

  // Duplicate Content-Length header smuggling must be rejected
  const dupRes = runTransportInput([
    Buffer.from('Content-Length: 2\r\nContent-Length: 999\r\n\r\n{}', 'ascii'),
  ]);
  assert.equal(dupRes.active, false);

  // +5 Content-Length must be rejected
  const plusRes = runTransportInput([
    Buffer.from('Content-Length: +5\r\n\r\nhello', 'ascii'),
  ]);
  assert.equal(plusRes.active, false);
});

suite.run();
