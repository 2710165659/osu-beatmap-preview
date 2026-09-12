// 解析类回归测试：`.osu` 字段、Range 请求头、分块切分与静态路径安全。

import assert from 'node:assert/strict';
import test from 'node:test';
import { parseBeatmapText, parseBackgroundFilename } from '../backend/beatmap-meta.js';
import { beatmapSetIdFromUrl } from '../backend/download-osu.js';
import { splitRanges } from '../backend/download-osz.js';
import { parseByteRange, safeRelativePath } from '../backend/static.js';

const SAMPLE = `osu file format v14

[General]
AudioFilename: audio.mp3
AudioLeadIn: 0

[Metadata]
Title:Sample
BeatmapSetID:1236927

[Events]
//Background and Video events
0,0,"bg with space.jpg",0,0
`;

test('解析 .osu 的音频、谱面集 ID 与背景图', () => {
  const beatmap = parseBeatmapText(SAMPLE);
  assert.equal(beatmap.audioFilename, 'audio.mp3');
  assert.equal(beatmap.beatmapSetId, 1236927);
  assert.equal(beatmap.backgroundFilename, 'bg with space.jpg');
});

test('背景图支持反斜杠路径与无引号写法', () => {
  assert.equal(parseBackgroundFilename(['0,0,"dir\\bg.png",0,0']), 'dir/bg.png');
  assert.equal(parseBackgroundFilename(['0,0,bg.jpg,0,0']), 'bg.jpg');
  assert.equal(parseBackgroundFilename(['1,0,"video.avi",0,0']), null);
  assert.equal(parseBackgroundFilename(['0,0,"unterminated']), null);
  assert.equal(parseBackgroundFilename([]), null);
});

test('缺少 BeatmapSetID 时返回 null 交给重定向解析', () => {
  const beatmap = parseBeatmapText('[General]\nAudioFilename: a.mp3\n');
  assert.equal(beatmap.beatmapSetId, null);
});

test('从 osu.ppy.sh 重定向地址解析谱面集 ID', () => {
  assert.equal(beatmapSetIdFromUrl('https://osu.ppy.sh/beatmapsets/1236927#osu/2628991'), 1236927);
  assert.equal(beatmapSetIdFromUrl('https://osu.ppy.sh/beatmapsets/0'), null);
  assert.equal(beatmapSetIdFromUrl('https://osu.ppy.sh/beatmaps/2628991'), null);
});

test('Range 请求头解析覆盖常规、后缀与越界情况', () => {
  assert.deepEqual(parseByteRange('bytes=0-99', 1000), { start: 0, end: 99 });
  assert.deepEqual(parseByteRange('bytes=100-', 1000), { start: 100, end: 999 });
  assert.deepEqual(parseByteRange('bytes=-100', 1000), { start: 900, end: 999 });
  assert.deepEqual(parseByteRange('bytes=0-4999', 1000), { start: 0, end: 999 });
  assert.equal(parseByteRange('bytes=1000-', 1000), null);
  assert.equal(parseByteRange('items=0-1', 1000), null);
  assert.equal(parseByteRange('bytes=0-99', 0), null);
  assert.equal(parseByteRange(undefined, 1000), null);
});

test('静态路径归一化拒绝越界与盘符', () => {
  assert.equal(safeRelativePath('/'), 'index.html');
  assert.equal(safeRelativePath('/pkg/app.js'), 'pkg/app.js');
  assert.equal(safeRelativePath('/pkg/../secret'), null);
  assert.equal(safeRelativePath('/C:/windows/win.ini'), null);
  assert.equal(safeRelativePath('/%2e%2e/secret'), null);
});

test('分块切分覆盖整包且块数受上限约束', () => {
  const ranges = splitRanges(1000, 4);
  assert.deepEqual(ranges, [
    { start: 0, end: 249, total: 1000 },
    { start: 250, end: 499, total: 1000 },
    { start: 500, end: 749, total: 1000 },
    { start: 750, end: 999, total: 1000 },
  ]);
  assert.equal(splitRanges(3, 4).length, 3);
  const single = splitRanges(10, 1);
  assert.deepEqual(single, [{ start: 0, end: 9, total: 10 }]);
});
