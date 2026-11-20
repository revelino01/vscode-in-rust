#!/usr/bin/env node
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const E2E_DIR = __dirname;
const REPO_ROOT = path.resolve(E2E_DIR, '..', '..');
const MANIFEST_PATH = path.join(E2E_DIR, 'Cargo.toml');
const JS_DIR = path.join(E2E_DIR, 'js');
const RUST_TESTS_DIR = path.join(E2E_DIR, 'tests');

function parseArgs(argv) {
  const opts = {
    tier: 'all',
    syntaxCheck: false,
    json: false,
    filter: null,
    help: false,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') {
      opts.help = true;
    } else if (arg === '--syntax-check') {
      opts.syntaxCheck = true;
    } else if (arg === '--json') {
      opts.json = true;
    } else if (arg === '--tier') {
      if (i + 1 >= argv.length) {
        throw new Error('--tier requires a value (1, 2, 3, 4, or all)');
      }
      opts.tier = argv[++i];
    } else if (arg.startsWith('--tier=')) {
      opts.tier = arg.slice('--tier='.length);
    } else if (arg === '--filter') {
      if (i + 1 >= argv.length) {
        throw new Error('--filter requires a substring argument');
      }
      opts.filter = argv[++i];
    } else if (arg.startsWith('--filter=')) {
      opts.filter = arg.slice('--filter='.length);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (!['1', '2', '3', '4', 'all'].includes(opts.tier)) {
    throw new Error(`Invalid --tier value "${opts.tier}". Expected 1, 2, 3, 4, or all.`);
  }

  return opts;
}

function buildEnv() {
  const env = { ...process.env };
  if (process.platform === 'win32') {
    const userProfile = env.USERPROFILE || os.homedir();
    const candidates = [
      path.join(userProfile, '.cargo', 'bin'),
      path.join(userProfile, 'scoop', 'apps', 'mingw', 'current', 'bin'),
    ];
    const pathKey = Object.keys(env).find((k) => k.toLowerCase() === 'path') || 'PATH';
    const existing = (env[pathKey] || '').split(path.delimiter).filter(Boolean);
    const existingLower = new Set(existing.map((p) => p.toLowerCase()));
    const additions = [];
    for (const dir of candidates) {
      if (fs.existsSync(dir) && !existingLower.has(dir.toLowerCase())) {
        additions.push(dir);
      }
    }
    if (additions.length > 0) {
      env[pathKey] = [...additions, ...existing].join(path.delimiter);
    }
  } else {
    const homeDir = env.HOME || os.homedir();
    const cargoBin = path.join(homeDir, '.cargo', 'bin');
    const existing = (env.PATH || '').split(path.delimiter).filter(Boolean);
    if (fs.existsSync(cargoBin) && !existing.includes(cargoBin)) {
      env.PATH = [cargoBin, ...existing].join(path.delimiter);
    }
  }
  return env;
}

function runCommand(cmd, args, { env, cwd, jsonMode, label }) {
  const startedAt = Date.now();
  if (!jsonMode) {
    process.stdout.write(`==> ${label}: ${cmd} ${args.join(' ')}\n`);
  }
  const res = spawnSync(cmd, args, {
    cwd,
    env,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
    stdio: jsonMode ? 'pipe' : 'inherit',
  });
  const durationMs = Date.now() - startedAt;
  const ok = !res.error && res.status === 0;
  return {
    label,
    command: `${cmd} ${args.join(' ')}`,
    ok,
    exitCode: res.status === null ? 1 : res.status,
    durationMs,
    error: res.error ? res.error.message : undefined,
    stdout: jsonMode ? (res.stdout || '') : undefined,
    stderr: jsonMode ? (res.stderr || '') : undefined,
  };
}

function listJsFilesForSyntaxCheck() {
  const files = [path.join(E2E_DIR, 'runner.js')];
  if (fs.existsSync(JS_DIR)) {
    const entries = fs.readdirSync(JS_DIR, { withFileTypes: true });
    const jsEntries = entries
      .filter((e) => e.isFile() && e.name.endsWith('.js'))
      .map((e) => path.join(JS_DIR, e.name))
      .sort();
    files.push(...jsEntries);
  }
  return files;
}

function discoverTierTargets(tierNum) {
  const jsFiles = [];
  if (fs.existsSync(JS_DIR)) {
    const prefix = `tier${tierNum}_`;
    const entries = fs.readdirSync(JS_DIR, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.isFile() && entry.name.startsWith(prefix) && entry.name.endsWith('.test.js')) {
        jsFiles.push(path.join(JS_DIR, entry.name));
      }
    }
    jsFiles.sort();
  }

  const rustTargets = [];
  if (fs.existsSync(RUST_TESTS_DIR)) {
    const prefix = `tier${tierNum}_`;
    const entries = fs.readdirSync(RUST_TESTS_DIR, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.isFile() && entry.name.startsWith(prefix) && entry.name.endsWith('.rs')) {
        rustTargets.push(entry.name.slice(0, -'.rs'.length));
      }
    }
    rustTargets.sort();
  }

  return { jsFiles, rustTargets };
}

function runSyntaxChecks(env, jsonMode) {
  const checks = [];

  for (const filePath of listJsFilesForSyntaxCheck()) {
    const rel = path.relative(REPO_ROOT, filePath);
    checks.push(
      runCommand(process.execPath, ['--check', filePath], {
        env,
        cwd: REPO_ROOT,
        jsonMode,
        label: `node --check ${rel}`,
      })
    );
  }

  checks.push(
    runCommand(
      'cargo',
      ['test', '--manifest-path', MANIFEST_PATH, '--no-run', '-j', '2'],
      {
        env,
        cwd: REPO_ROOT,
        jsonMode,
        label: 'cargo test --no-run',
      }
    )
  );

  checks.push(
    runCommand(
      'cargo',
      [
        'clippy',
        '--manifest-path',
        MANIFEST_PATH,
        '--all-targets',
        '-j',
        '2',
        '--',
        '-D',
        'warnings',
      ],
      {
        env,
        cwd: REPO_ROOT,
        jsonMode,
        label: 'cargo clippy',
      }
    )
  );

  const passed = checks.filter((c) => c.ok).length;
  const failed = checks.length - passed;
  return {
    ok: failed === 0,
    mode: 'syntax-check',
    checks,
    summary: {
      total: checks.length,
      passed,
      failed,
    },
  };
}

function runTiers(opts, env) {
  const selectedTiers = opts.tier === 'all' ? [1, 2, 3, 4] : [Number(opts.tier)];
  const tierResults = [];
  let totalChecks = 0;
  let totalPassed = 0;
  let totalFailed = 0;

  for (const tierNum of selectedTiers) {
    const { jsFiles, rustTargets } = discoverTierTargets(tierNum);
    const checks = [];

    for (const jsFile of jsFiles) {
      const rel = path.relative(REPO_ROOT, jsFile);
      const args = [jsFile];
      if (opts.filter) {
        args.push('--filter', opts.filter);
      }
      checks.push(
        runCommand(process.execPath, args, {
          env,
          cwd: REPO_ROOT,
          jsonMode: opts.json,
          label: `tier${tierNum} js: ${rel}`,
        })
      );
    }

    for (const target of rustTargets) {
      const args = ['test', '--manifest-path', MANIFEST_PATH];
      if (process.platform === 'win32') {
        args.push('-j', '2');
      }
      args.push('--test', target);
      if (opts.filter) {
        args.push('--', opts.filter);
      }
      checks.push(
        runCommand('cargo', args, {
          env,
          cwd: REPO_ROOT,
          jsonMode: opts.json,
          label: `tier${tierNum} rust: ${target}`,
        })
      );
    }

    const passed = checks.filter((c) => c.ok).length;
    const failed = checks.length - passed;
    totalChecks += checks.length;
    totalPassed += passed;
    totalFailed += failed;

    tierResults.push({
      tier: tierNum,
      ok: failed === 0,
      jsFiles: jsFiles.map((f) => path.relative(REPO_ROOT, f)),
      rustTargets,
      checks,
      summary: {
        total: checks.length,
        passed,
        failed,
      },
    });
  }

  return {
    ok: totalFailed === 0,
    mode: 'run',
    tier: opts.tier,
    filter: opts.filter,
    tiers: tierResults,
    summary: {
      total: totalChecks,
      passed: totalPassed,
      failed: totalFailed,
    },
  };
}

function printHumanSummary(report) {
  process.stdout.write('\n=== Phase 7 E2E Summary ===\n');
  if (report.mode === 'syntax-check') {
    for (const c of report.checks) {
      const status = c.ok ? 'PASS' : 'FAIL';
      process.stdout.write(`  [${status}] ${c.label} (${c.durationMs}ms)\n`);
    }
    process.stdout.write(
      `Total: ${report.summary.passed}/${report.summary.total} passed, ${report.summary.failed} failed\n`
    );
    return;
  }

  for (const t of report.tiers) {
    const status = t.ok ? 'PASS' : 'FAIL';
    process.stdout.write(
      `  Tier ${t.tier} [${status}]: ${t.summary.passed}/${t.summary.total} checks passed (JS: ${t.jsFiles.length}, Rust: ${t.rustTargets.length})\n`
    );
    for (const c of t.checks) {
      if (!c.ok) {
        process.stdout.write(`    [FAIL] ${c.label} (exit ${c.exitCode})\n`);
      }
    }
  }
  process.stdout.write(
    `Overall: ${report.summary.passed}/${report.summary.total} passed, ${report.summary.failed} failed\n`
  );
}

function printHelp() {
  process.stdout.write(
    [
      'Usage: node tests/phase7-e2e/runner.js [options]',
      '',
      'Options:',
      '  --tier <1|2|3|4|all>   Select E2E tier to run (default: all)',
      '  --syntax-check         Run node --check, cargo test --no-run, and cargo clippy',
      '  --filter <substr>      Forward test name filter to Rust and JS tests',
      '  --json                 Output machine-readable JSON summary',
      '  --help, -h             Show this help message',
      '',
    ].join('\n')
  );
}

function main() {
  let opts;
  try {
    opts = parseArgs(process.argv.slice(2));
  } catch (err) {
    process.stderr.write(`Error: ${err.message}\n`);
    process.exit(2);
  }

  if (opts.help) {
    printHelp();
    process.exit(0);
  }

  const env = buildEnv();
  const report = opts.syntaxCheck ? runSyntaxChecks(env, opts.json) : runTiers(opts, env);

  if (opts.json) {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  } else {
    printHumanSummary(report);
  }

  process.exit(report.ok ? 0 : 1);
}

main();
