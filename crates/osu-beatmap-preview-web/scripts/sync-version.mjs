#!/usr/bin/env node
// 把仓库根 Cargo.toml 的 [workspace.package].version 同步到 Web 包的 package.json。
// 本地开发可用 `npm run sync:version`；发布流程与 build:wasm 会自动调用。

import { syncPackageVersion } from './version.mjs';

const version = syncPackageVersion();
console.log(`package.json 版本已同步为 ${version}`);
