#!/usr/bin/env node
// 把 wasm 产物构建到 public/pkg。
//
// 这里只是把仓库里既有的构建步骤串起来，发布包里已经带好 public/pkg，运行站点
// 不需要 Rust 工具链。开发时如果改了 core/renderer/wasm，重新执行本脚本即可。

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '../../..');
const outDir = path.resolve(here, '../public/pkg');
const wasmFile = path.join(
  repoRoot,
  'target/wasm32-unknown-unknown/release/osu_beatmap_preview_wasm.wasm',
);

const lockedVersion = readLockedWasmBindgenVersion();
if (lockedVersion) {
  const installed = spawnSync('wasm-bindgen', ['--version'], { encoding: 'utf8' });
  if (installed.error || installed.status !== 0) {
    fail(
      '找不到 wasm-bindgen。安装与 Cargo.lock 一致的版本：\n' +
        `  cargo install wasm-bindgen-cli --version ${lockedVersion} --locked`,
    );
  }
  const actual = installed.stdout.trim();
  if (!actual.endsWith(lockedVersion)) {
    // 版本不一致时 wasm-bindgen 会拒绝处理 .wasm，先给出明确提示再继续尝试。
    console.warn(
      `警告：wasm-bindgen 版本（${actual}）与 Cargo.lock 中的 ${lockedVersion} 不一致，可能失败。`,
    );
  }
}

run('cargo', [
  'build',
  '--release',
  '--package',
  'osu-beatmap-preview-wasm',
  '--target',
  'wasm32-unknown-unknown',
]);
if (!fs.existsSync(wasmFile)) {
  fail('没有找到 wasm 产物，请确认已安装 wasm32-unknown-unknown 目标：\n  rustup target add wasm32-unknown-unknown');
}

fs.mkdirSync(outDir, { recursive: true });
run('wasm-bindgen', [wasmFile, '--target', 'web', '--out-dir', outDir]);
console.log(`wasm 产物已生成：${path.relative(repoRoot, outDir)}`);

function run(command, args) {
  const result = spawnSync(command, args, { cwd: repoRoot, stdio: 'inherit', shell: false });
  if (result.error) fail(`执行 ${command} 失败：${result.error.message}`);
  if (result.status !== 0) fail(`${command} 退出码 ${result.status}`);
}

function readLockedWasmBindgenVersion() {
  try {
    const lock = fs.readFileSync(path.join(repoRoot, 'Cargo.lock'), 'utf8');
    const match = /name = "wasm-bindgen"\nversion = "([^"]+)"/.exec(lock);
    return match?.[1] ?? null;
  } catch {
    return null;
  }
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
