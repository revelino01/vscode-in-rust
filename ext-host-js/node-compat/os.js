'use strict';

// Pure-JS + RPC-backed Node.js os compatibility shim (no dependency on Node built-in 'os').

const rpc = require('../rpc-bridge');

let osCache = null;

function mergeOsInfo(info) {
  if (info && typeof info === 'object' && !Array.isArray(info) && !info.error) {
    osCache = { ...(osCache || {}), ...info };
  }
}

function hydrateSync() {
  try {
    if (typeof rpc.hasSyncTransport === 'function' && rpc.hasSyncTransport()) {
      const info = rpc.callSync('env.getOsInfo', {});
      mergeOsInfo(info);
    }
  } catch (_) {}
}

hydrateSync();

if (typeof rpc.registerInitHook === 'function') {
  rpc.registerInitHook(async () => {
    try {
      const info = await rpc.call('env.getOsInfo', {});
      mergeOsInfo(info);
    } catch (_) {}
  });
}

function getEnvVar(name) {
  try {
    if (typeof process !== 'undefined' && process.env && process.env[name]) {
      return process.env[name];
    }
  } catch (_) {}
  try {
    if (typeof Deno !== 'undefined' && Deno.env && typeof Deno.env.get === 'function') {
      const v = Deno.env.get(name);
      if (v) return v;
    }
  } catch (_) {}
  return '';
}

function detectHostPlatform() {
  if (osCache && typeof osCache.platform === 'string' && osCache.platform.length > 0) {
    return osCache.platform;
  }
  if (typeof process !== 'undefined' && typeof process.platform === 'string' && process.platform) {
    return process.platform;
  }
  if (typeof Deno !== 'undefined' && Deno.build && Deno.build.os) {
    const osName = Deno.build.os;
    if (osName === 'windows') return 'win32';
    if (osName === 'macos') return 'darwin';
    return osName;
  }
  return 'linux';
}

function detectHostArch() {
  if (osCache && typeof osCache.arch === 'string' && osCache.arch.length > 0) {
    return osCache.arch;
  }
  if (typeof process !== 'undefined' && typeof process.arch === 'string' && process.arch) {
    return process.arch;
  }
  if (typeof Deno !== 'undefined' && Deno.build && Deno.build.arch) {
    const a = Deno.build.arch;
    if (a === 'x86_64') return 'x64';
    if (a === 'aarch64') return 'arm64';
    if (a === 'x86') return 'ia32';
    return a;
  }
  return 'x64';
}

const HOST_ENDIANNESS = (() => {
  const buf = new ArrayBuffer(2);
  new Uint8Array(buf)[0] = 0x12;
  new Uint8Array(buf)[1] = 0x34;
  return new Uint16Array(buf)[0] === 0x3412 ? 'LE' : 'BE';
})();

function platform() {
  return detectHostPlatform();
}

function arch() {
  return detectHostArch();
}

function type() {
  if (osCache && typeof osCache.type === 'string' && osCache.type.length > 0) {
    return osCache.type;
  }
  const p = detectHostPlatform();
  if (p === 'win32') return 'Windows_NT';
  if (p === 'darwin') return 'Darwin';
  return 'Linux';
}

function release() {
  if (osCache && typeof osCache.release === 'string' && osCache.release.length > 0) {
    return osCache.release;
  }
  return '0.0.0';
}

function version() {
  if (osCache && typeof osCache.version === 'string' && osCache.version.length > 0) {
    return osCache.version;
  }
  return `${type()} ${release()}`;
}

function machine() {
  if (osCache && typeof osCache.machine === 'string' && osCache.machine.length > 0) {
    return osCache.machine;
  }
  const a = detectHostArch();
  if (a === 'x64') return 'x86_64';
  if (a === 'arm64') return 'aarch64';
  return a;
}

function hostname() {
  if (osCache && typeof osCache.hostname === 'string' && osCache.hostname.length > 0) {
    return osCache.hostname;
  }
  return getEnvVar('COMPUTERNAME') || getEnvVar('HOSTNAME') || 'localhost';
}

function homedir() {
  if (osCache && typeof osCache.homedir === 'string' && osCache.homedir.length > 0) {
    return osCache.homedir;
  }
  const isWin = detectHostPlatform() === 'win32';
  if (isWin) {
    const up = getEnvVar('USERPROFILE');
    if (up) return up;
    const hd = getEnvVar('HOMEDRIVE');
    const hp = getEnvVar('HOMEPATH');
    if (hd && hp) return hd + hp;
    const home = getEnvVar('HOME');
    return home || 'C:\\Users\\user';
  }
  return getEnvVar('HOME') || '/home/user';
}

function tmpdir() {
  if (osCache && typeof osCache.tmpdir === 'string' && osCache.tmpdir.length > 0) {
    return osCache.tmpdir;
  }
  const isWin = detectHostPlatform() === 'win32';
  if (isWin) {
    return getEnvVar('TEMP') || getEnvVar('TMP') || 'C:\\Windows\\Temp';
  }
  return getEnvVar('TMPDIR') || getEnvVar('TMP') || getEnvVar('TEMP') || '/tmp';
}

function endianness() {
  if (osCache && (osCache.endianness === 'LE' || osCache.endianness === 'BE')) {
    return osCache.endianness;
  }
  return HOST_ENDIANNESS;
}

function defaultCpuEntry() {
  return {
    model: 'RustCode Virtual CPU',
    speed: 2400,
    times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 },
  };
}

function cpus() {
  if (osCache && Array.isArray(osCache.cpus) && osCache.cpus.length > 0) {
    return osCache.cpus.map((c) => ({
      model: (c && c.model) || 'CPU',
      speed: (c && typeof c.speed === 'number') ? c.speed : 2400,
      times: (c && c.times && typeof c.times === 'object')
        ? { user: 0, nice: 0, sys: 0, idle: 0, irq: 0, ...c.times }
        : { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 },
    }));
  }
  if (osCache && typeof osCache.cpus === 'number' && osCache.cpus > 0) {
    const arr = [];
    for (let i = 0; i < osCache.cpus; i++) {
      arr.push(defaultCpuEntry());
    }
    return arr;
  }
  return [defaultCpuEntry()];
}

function availableParallelism() {
  return cpus().length;
}

function totalmem() {
  if (osCache && typeof osCache.totalmem === 'number' && osCache.totalmem > 0) {
    return osCache.totalmem;
  }
  return 8 * 1024 * 1024 * 1024;
}

function freemem() {
  if (osCache && typeof osCache.freemem === 'number' && osCache.freemem >= 0) {
    return osCache.freemem;
  }
  return 4 * 1024 * 1024 * 1024;
}

function uptime() {
  if (osCache && typeof osCache.uptime === 'number' && osCache.uptime >= 0) {
    return osCache.uptime;
  }
  if (typeof process !== 'undefined' && typeof process.uptime === 'function') {
    return Math.floor(process.uptime());
  }
  return 0;
}

function loadavg() {
  if (osCache && Array.isArray(osCache.loadavg) && osCache.loadavg.length === 3) {
    return osCache.loadavg.slice();
  }
  return [0, 0, 0];
}

function userInfo(options = {}) {
  const isWin = detectHostPlatform() === 'win32';
  const username =
    (osCache && osCache.userInfo && osCache.userInfo.username) ||
    (osCache && osCache.username) ||
    getEnvVar('USERNAME') ||
    getEnvVar('USER') ||
    'user';
  const home =
    (osCache && osCache.userInfo && osCache.userInfo.homedir) ||
    homedir();
  const shell =
    (osCache && osCache.userInfo && osCache.userInfo.shell) ||
    (isWin ? null : (getEnvVar('SHELL') || '/bin/sh'));
  const uid =
    (osCache && osCache.userInfo && typeof osCache.userInfo.uid === 'number')
      ? osCache.userInfo.uid
      : (isWin ? -1 : 1000);
  const gid =
    (osCache && osCache.userInfo && typeof osCache.userInfo.gid === 'number')
      ? osCache.userInfo.gid
      : (isWin ? -1 : 1000);

  void options;
  return {
    uid,
    gid,
    username,
    homedir: home,
    shell,
  };
}

function networkInterfaces() {
  if (osCache && osCache.networkInterfaces && typeof osCache.networkInterfaces === 'object') {
    return osCache.networkInterfaces;
  }
  return {
    lo: [
      {
        address: '127.0.0.1',
        netmask: '255.0.0.0',
        family: 'IPv4',
        mac: '00:00:00:00:00:00',
        internal: true,
        cidr: '127.0.0.1/8',
      },
    ],
  };
}

function getPriority() {
  return 0;
}

function setPriority() {
  // No-op in sandboxed shim
}

const constants = {
  UV_UDP_REUSEADDR: 4,
  dlopen: {
    RTLD_LAZY: 1,
    RTLD_NOW: 2,
    RTLD_GLOBAL: 256,
    RTLD_LOCAL: 0,
  },
  errno: {
    E2BIG: 7,
    EACCES: 13,
    EADDRINUSE: 98,
    EADDRNOTAVAIL: 99,
    EAFNOSUPPORT: 97,
    EAGAIN: 11,
    EBADF: 9,
    EBUSY: 16,
    ECANCELED: 125,
    ECHILD: 10,
    ECONNABORTED: 103,
    ECONNREFUSED: 111,
    ECONNRESET: 104,
    EEXIST: 17,
    EFAULT: 14,
    EINVAL: 22,
    EIO: 5,
    EISDIR: 21,
    EMFILE: 24,
    ENAMETOOLONG: 36,
    ENOENT: 2,
    ENOMEM: 12,
    ENOSPC: 28,
    ENOSYS: 38,
    ENOTDIR: 20,
    ENOTEMPTY: 39,
    EPERM: 1,
    EPIPE: 32,
    EROFS: 30,
    ESPIPE: 29,
    ESRCH: 3,
    ETIMEDOUT: 110,
  },
  signals: {
    SIGHUP: 1,
    SIGINT: 2,
    SIGQUIT: 3,
    SIGILL: 4,
    SIGTRAP: 5,
    SIGABRT: 6,
    SIGBUS: 7,
    SIGFPE: 8,
    SIGKILL: 9,
    SIGUSR1: 10,
    SIGSEGV: 11,
    SIGUSR2: 12,
    SIGPIPE: 13,
    SIGALRM: 14,
    SIGTERM: 15,
  },
  priority: {
    PRIORITY_LOW: 19,
    PRIORITY_BELOW_NORMAL: 10,
    PRIORITY_NORMAL: 0,
    PRIORITY_ABOVE_NORMAL: -7,
    PRIORITY_HIGH: -14,
    PRIORITY_HIGHEST: -20,
  },
};

const osExport = {
  platform,
  arch,
  type,
  release,
  version,
  machine,
  hostname,
  homedir,
  tmpdir,
  endianness,
  cpus,
  availableParallelism,
  totalmem,
  freemem,
  uptime,
  loadavg,
  userInfo,
  networkInterfaces,
  getPriority,
  setPriority,
  constants,
};

Object.defineProperty(osExport, 'EOL', {
  enumerable: true,
  configurable: true,
  get() {
    if (osCache && typeof osCache.eol === 'string' && osCache.eol.length > 0) {
      return osCache.eol;
    }
    return detectHostPlatform() === 'win32' ? '\r\n' : '\n';
  },
});

Object.defineProperty(osExport, 'devNull', {
  enumerable: true,
  configurable: true,
  get() {
    return detectHostPlatform() === 'win32' ? '\\\\.\\nul' : '/dev/null';
  },
});

module.exports = osExport;
