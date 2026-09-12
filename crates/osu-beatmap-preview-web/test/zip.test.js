// ZIP 读取回归测试：使用仓库内的 sample.osz fixture，覆盖中央目录解析、
// 条目查找（大小写不敏感）、解压结果与损坏输入。

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  extractEntry,
  findEntry,
  hasZipEndRecord,
  isZipBuffer,
  normalizeArchivePath,
  readZipIndex,
} from '../backend/zip.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const fixture = fs.readFileSync(path.join(here, 'fixtures/sample.osz'));

test('解析中央目录并列出条目', () => {
  const entries = readZipIndex(fixture);
  assert.deepEqual(
    entries.map((entry) => entry.name).sort(),
    ['audio.mp3', 'bg.jpg', 'map.osu'],
  );
  assert.ok(isZipBuffer(fixture));
});

test('按不区分大小写的名字取出条目内容', () => {
  const entries = readZipIndex(fixture);
  const background = findEntry(entries, 'BG.JPG');
  assert.ok(background);
  assert.equal(extractEntry(fixture, background).toString(), 'fake-jpg-payload');
});

test('尾部片段即可完成缓存校验', () => {
  const tail = fixture.subarray(Math.max(0, fixture.length - 4096));
  assert.ok(hasZipEndRecord(tail));
  assert.ok(!hasZipEndRecord(Buffer.from('not a zip file at all, definitely not')));
});

test('损坏的压缩包会被拒绝', () => {
  const broken = Buffer.from(fixture);
  broken.writeUInt32LE(0, broken.length - 22);
  assert.equal(isZipBuffer(broken), false);
  assert.throws(() => readZipIndex(broken));
});

test('压缩包路径归一化拒绝越界与绝对路径', () => {
  assert.equal(normalizeArchivePath('folder\\bg.jpg'), 'folder/bg.jpg');
  assert.equal(normalizeArchivePath('./a//b.png'), 'a/b.png');
  assert.equal(normalizeArchivePath('../secret'), null);
  assert.equal(normalizeArchivePath('/etc/passwd'), null);
  assert.equal(normalizeArchivePath('C:/windows/system32'), null);
  assert.equal(normalizeArchivePath(''), null);
});
