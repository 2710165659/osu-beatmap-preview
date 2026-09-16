// 谱面自带打击音的候选名匹配测试。
//
// 浏览器只知道「wasm 需要哪些候选名」与「后端解出了哪些条目名」，两者要对上才能用上
// 谱面自定义的音效。规则由 core 的 `sample_entry_matches`（`domain/media.rs`）定义，
// 这里用同一张用例表钉住 JS 实现，避免任一侧改规则而另一边悄悄失效。

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createBeatmapSampleIndex, sampleLookupKeys } from '../src/hitsound.js';

/** 候选名能否命中某个条目名。 */
function matches(entryName, candidate) {
  const index = createBeatmapSampleIndex([{ name: entryName, url: `/s/${entryName}` }]);
  return index.has(candidate.toLowerCase());
}

test('条目名与候选名的匹配规则与 core 一致', () => {
  // 不带扩展名的样本名对应压缩包里的音频文件。
  assert.ok(matches('soft-hitnormal.ogg', 'soft-hitnormal'));
  assert.ok(matches('Soft-Hitnormal.WAV', 'soft-hitnormal'));
  assert.ok(matches('hitnormal.mp3', 'hitnormal'));
  // `hitSample` 的自定义文件名可以带子目录。
  assert.ok(matches('sub/custom-hit.ogg', 'custom-hit.ogg'));
  assert.ok(matches('Custom-Hit.OGG', 'custom-hit.ogg'));
  // bank 前缀是候选名的一部分，不能只按文件名后缀命中。
  assert.ok(!matches('custom-hit.ogg', 'normal-custom-hit.ogg'));
  assert.ok(!matches('hitnormal.ogg', 'soft-hitnormal'));
  assert.ok(!matches('spinnerbonus.ogg', 'spinnerbonus-max'));
});

test('候选名键只保留可用写法', () => {
  assert.deepEqual(sampleLookupKeys('soft-hitnormal.ogg'), [
    'soft-hitnormal.ogg',
    'soft-hitnormal',
  ]);
  assert.deepEqual(sampleLookupKeys('sub/Custom-Hit.OGG'), [
    'sub/custom-hit.ogg',
    'custom-hit.ogg',
    'custom-hit',
  ]);
  // 反斜杠与首尾空白按压缩包条目名的规则归一化。
  assert.deepEqual(sampleLookupKeys(' sub\\hit.ogg '), ['sub/hit.ogg', 'hit.ogg', 'hit']);
  assert.deepEqual(sampleLookupKeys(''), []);
  assert.deepEqual(sampleLookupKeys(null), []);
  assert.deepEqual(sampleLookupKeys('.ogg'), ['.ogg']);
});

test('清单里没有的候选名不会命中', () => {
  const index = createBeatmapSampleIndex([
    { name: 'soft-hitnormal.ogg', url: '/resource/sample?bid=1&name=soft-hitnormal.ogg' },
    { name: 'custom-hit.wav', url: '/resource/sample?bid=1&name=custom-hit.wav' },
  ]);
  assert.equal(
    index.get('soft-hitnormal'),
    '/resource/sample?bid=1&name=soft-hitnormal.ogg',
  );
  assert.equal(index.get('custom-hit.wav'), '/resource/sample?bid=1&name=custom-hit.wav');
  assert.equal(index.get('drum-hitclap'), undefined);
  // 不完整的条目不会被登记。
  assert.equal(createBeatmapSampleIndex([{ name: 'x.ogg' }]).size, 0);
  assert.equal(createBeatmapSampleIndex(null).size, 0);
});
