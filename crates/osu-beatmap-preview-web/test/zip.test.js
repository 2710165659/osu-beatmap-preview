// ZIP 读取回归测试：使用仓库内的 sample.osz fixture，覆盖中央目录解析、
// 条目查找（大小写不敏感）、解压结果、路径归一化与损坏输入。
//
// 后端下载只用 `hasZipEndRecord` / `isZipBuffer` 做廉价校验，但整套读取实现仍被
// 契约测试（media-contract.test.js）用来与 core 的媒体策略对表，因此都要钉住。

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

/** fixture 的条目清单（名字以压缩包内为准）。 */
const FIXTURE_ENTRIES = ['audio.mp3', 'bg.jpg', 'map.osu'];

test('中央目录解析出全部条目', () => {
  const entries = readZipIndex(fixture);
  const names = entries.map((entry) => entry.name).sort();
  assert.deepEqual(names, FIXTURE_ENTRIES);
});

test('条目查找大小写不敏感、忽略路径分隔符差异', () => {
  const entries = readZipIndex(fixture);
  assert.equal(findEntry(entries, 'map.osu')?.name, 'map.osu');
  assert.equal(findEntry(entries, 'MAP.OSU')?.name, 'map.osu');
  assert.equal(findEntry(entries, 'bg.jpg')?.name, 'bg.jpg');
  assert.equal(findEntry(entries, '../map.osu'), null);
  assert.equal(findEntry(entries, ''), null);
});

test('解压结果与条目内容一致', () => {
  const entries = readZipIndex(fixture);
  const osu = extractEntry(fixture, findEntry(entries, 'map.osu'));
  assert.match(osu.toString('utf8'), /osu file format/);
});

test('ZIP 校验：结束记录与完整解析', () => {
  assert.equal(hasZipEndRecord(fixture), true);
  assert.equal(isZipBuffer(fixture), true);
  assert.equal(hasZipEndRecord(Buffer.from('not a zip')), false);
  assert.equal(isZipBuffer(Buffer.from('not a zip')), false);
});

test('损坏输入报错而不是崩溃', () => {
  // 截断到中央目录之前：解析必须抛错，且错误可读。
  assert.throws(() => readZipIndex(fixture.subarray(0, 10)), /中央目录|条目/);
  assert.throws(() => extractEntry(fixture.subarray(0, 10), {
    name: 'map.osu', localHeaderOffset: 0, compressedSize: 10, method: 0,
  }), /本地头|超出/);
});

test('路径归一化拒绝越界、保留合法路径', () => {
  assert.equal(normalizeArchivePath('audio\\song.mp3'), 'audio/song.mp3');
  assert.equal(normalizeArchivePath('a/./b//c'), 'a/b/c');
  assert.equal(normalizeArchivePath(' bg.jpg '), 'bg.jpg');
  assert.equal(normalizeArchivePath('../song.mp3'), null);
  assert.equal(normalizeArchivePath('/song.mp3'), null);
  assert.equal(normalizeArchivePath('C:/song.mp3'), null);
  assert.equal(normalizeArchivePath('   '), null);
  assert.equal(normalizeArchivePath('.'), null);
});
