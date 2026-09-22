// 本地谱面文件规则（src/local-file.js）回归测试。
//
// 与 CLI 的 `application/local.rs` 是同一张用例表：文件类型判定、.osz 里「只认顶层
// .osu」的规则（stable / osu!lazer 的导入行为）、谱面自带音效的条目筛选限制，
// 以及本地 .osu 用的静音 WAV 头部。两边任何一边改了规则，这里都要跟着更新。

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  beatmapEntries,
  isBeatmapEntry,
  localFileKind,
  sampleEntries,
  silentWavBytes,
} from '../src/local-file.js';

const entry = (name, uncompressedSize = 1024) => ({ name, uncompressedSize });

test('文件类型判定接受大小写，其他后缀一律拒绝', () => {
  assert.equal(localFileKind('map.osu'), 'osu');
  assert.equal(localFileKind('C:\\maps\\MAP.OSZ'), 'osz');
  assert.equal(localFileKind('map.osk'), null);
  assert.equal(localFileKind('map'), null);
  assert.equal(localFileKind(''), null);
  assert.equal(localFileKind(null), null);
});

test('只认顶层 .osu：子目录里的谱面会被 stable 忽略', () => {
  assert.equal(isBeatmapEntry('map.osu'), true);
  assert.equal(isBeatmapEntry('MAP.OSU'), true);
  assert.equal(isBeatmapEntry('map.Osu'), true);
  assert.equal(isBeatmapEntry('sub/map.osu'), false);
  assert.equal(isBeatmapEntry('map.osz'), false);
  assert.equal(isBeatmapEntry('../map.osu'), false);
  assert.equal(isBeatmapEntry('map.osu.bak'), false);
});

test('谱面条目清单保持压缩包内顺序', () => {
  const entries = [
    entry('audio.mp3'),
    entry('Easy.osu'),
    entry('sub/Hard.osu'),
    entry('Hard.osu'),
  ];
  assert.deepEqual(
    beatmapEntries(entries).map((item) => item.name),
    ['Easy.osu', 'Hard.osu'],
  );
});

test('样本筛选跳过歌曲音频（不区分大小写）并按扩展名过滤', () => {
  const entries = [
    entry('audio/song.mp3'),
    entry('AUDIO\\SONG.MP3'),
    entry('soft-hitnormal.ogg'),
    entry('sub/custom-hit.ogg'),
    entry('bg.jpg'),
    entry('readme.txt'),
  ];
  // 前两个都是歌曲音频本身（大小写与分隔符不同），必须一起跳过。
  assert.deepEqual(
    sampleEntries(entries, 'audio/song.mp3').map((item) => item.name),
    ['soft-hitnormal.ogg', 'sub/custom-hit.ogg'],
  );
});

test('样本筛选有条目数与体积上限', () => {
  const many = Array.from({ length: 70 }, (_, index) => entry(`sample-${index}.wav`));
  assert.equal(sampleEntries(many, null).length, 64);

  // 单个 8 MiB 是允许的，合计达到 32 MiB 后停止取用。
  const huge = Array.from({ length: 6 }, (_, index) => entry(`big-${index}.ogg`, 8 * 1024 * 1024));
  assert.equal(sampleEntries(huge, null).length, 4);

  const oversized = [entry('too-big.ogg', 8 * 1024 * 1024 + 1)];
  assert.equal(sampleEntries(oversized, null).length, 0);
});

test('静音 WAV 的头部字段与长度正确', () => {
  const bytes = silentWavBytes(1.5);
  const view = new DataView(bytes.buffer);
  // 44 字节头 + 1.5 秒 × 8000 样本 × 2 字节。
  assert.equal(bytes.byteLength, 44 + 12000 * 2);
  const ascii = (offset, length) => String.fromCharCode(...bytes.subarray(offset, offset + length));
  assert.equal(ascii(0, 4), 'RIFF');
  assert.equal(ascii(8, 4), 'WAVE');
  assert.equal(ascii(12, 4), 'fmt ');
  assert.equal(view.getUint16(20, true), 1, 'PCM 编码');
  assert.equal(view.getUint16(22, true), 1, '单声道');
  assert.equal(view.getUint32(24, true), 8000, '采样率');
  assert.equal(view.getUint16(34, true), 16, '位深');
  assert.equal(ascii(36, 4), 'data');
  assert.equal(view.getUint32(40, true), 12000 * 2, '数据区长度');
  // 0 秒也要产出至少一个样本，避免非法 WAV。
  assert.equal(silentWavBytes(0).byteLength, 44 + 2);
});
