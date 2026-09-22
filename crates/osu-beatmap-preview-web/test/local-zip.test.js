// 浏览器侧 ZIP 读取（src/zip.js）回归测试。
//
// 本地上传的 .osz 全靠它在浏览器里解开，因此与后端 backend/zip.js 用同一个 fixture
// 对照：两边解析出的条目清单与解压结果必须完全一致，任一实现改了规则都会被钉住。

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { extractEntry as backendExtract, readZipIndex as backendIndex } from '../backend/zip.js';
import { extractEntry, findEntry, normalizeArchivePath, readZipIndex } from '../src/zip.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const fixture = fs.readFileSync(path.join(here, 'fixtures/sample.osz'));

test('前端与后端解析出完全相同的条目清单', () => {
  assert.deepEqual(readZipIndex(fixture), backendIndex(fixture));
});

test('解压结果与后端一致（fixture 全部是 deflate 条目）', async () => {
  const entries = readZipIndex(fixture);
  assert.ok(entries.every((entry) => entry.method === 8), 'fixture 应覆盖 deflate 路径');
  for (const entry of entries) {
    const mine = await extractEntry(fixture, entry);
    const theirs = backendExtract(fixture, entry);
    assert.deepEqual([...mine], [...theirs], entry.name);
  }
});

test('按不区分大小写的名字取出条目内容', async () => {
  const entries = readZipIndex(fixture);
  const background = findEntry(entries, 'BG.JPG');
  assert.ok(background);
  const bytes = await extractEntry(fixture, background);
  assert.equal(new TextDecoder().decode(bytes), 'fake-jpg-payload');
});

test('损坏的压缩包会被拒绝', () => {
  const broken = Uint8Array.from(fixture);
  new DataView(broken.buffer).setUint32(broken.byteLength - 22, 0, true);
  assert.throws(() => readZipIndex(broken));
});

test('压缩包路径归一化拒绝越界与绝对路径', () => {
  // 与 test/media-contract.test.js、core 的 domain/media.rs 同一张用例表。
  assert.equal(normalizeArchivePath('folder\\bg.jpg'), 'folder/bg.jpg');
  assert.equal(normalizeArchivePath('./a//b.png'), 'a/b.png');
  assert.equal(normalizeArchivePath('../secret'), null);
  assert.equal(normalizeArchivePath('/etc/passwd'), null);
  assert.equal(normalizeArchivePath('C:/windows/system32'), null);
  assert.equal(normalizeArchivePath(''), null);
});
