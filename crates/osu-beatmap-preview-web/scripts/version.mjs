// 版本号的唯一来源是仓库根目录 Cargo.toml 的 [workspace.package].version。
// Web 站点是 Node.js 项目，package.json 的版本从这里单向同步，
// 由 `npm run sync:version`、`npm run build:wasm` 和发布流程调用。

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
export const repoRoot = path.resolve(here, '../../..');
export const rootManifest = path.join(repoRoot, 'Cargo.toml');
export const packageJsonPath = path.resolve(here, '../package.json');

/** 读取根 Cargo.toml 中的 workspace 版本；缺失或格式异常时抛错。 */
export function readWorkspaceVersion() {
  const manifest = fs.readFileSync(rootManifest, 'utf8');
  const section = /^\[workspace\.package\][^\[]*/m.exec(manifest);
  const version = section && /^version\s*=\s*"([^"]+)"/m.exec(section[0]);
  if (!version) {
    throw new Error(`未能在 ${path.relative(repoRoot, rootManifest)} 的 [workspace.package] 中找到 version`);
  }
  return version[1];
}

/** 把根 Cargo.toml 的版本写入 package.json，返回版本号。 */
export function syncPackageVersion() {
  const version = readWorkspaceVersion();
  const raw = fs.readFileSync(packageJsonPath, 'utf8');
  const { name, version: _ignored, ...rest } = JSON.parse(raw);
  // 版本字段固定排在 name 之后，保证写回后的文件内容稳定、可复现。
  const text = `${JSON.stringify({ name, version, ...rest }, null, 2)}\n`;
  if (text !== raw) {
    fs.writeFileSync(packageJsonPath, text);
  }
  return version;
}
