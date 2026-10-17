import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readPreviewLink, previewPath } from '../src/navigation.js';

test('复数 REST 路径决定模式，旧链接与非法 ID 不触发加载', () => {
  assert.deepEqual(readPreviewLink('/bids/3598855', '?convert=taiko&bid=1'), { type: 'osu', bid: '3598855', convert: 'taiko' });
  assert.deepEqual(readPreviewLink('/scores/7693452158/', '?bid=42&convert=mania'), { type: 'osr', scoreId: '7693452158', convert: 'mania' });
  assert.deepEqual(readPreviewLink('/scores/123'), { type: 'osr', scoreId: '123', convert: '' });
  assert.equal(readPreviewLink('/bids/42', '?convert=unknown').convert, '');
  for (const path of ['/', '/bid/42', '/score/42', '/bids/0', '/scores/-1', '/scores/abc', '/bids/42/extra']) {
    assert.equal(readPreviewLink(path, '?bid=42'), null);
  }
  assert.equal(previewPath({ type: 'osu', bid: '42', convert: 'mania' }), '/bids/42?convert=mania');
  assert.equal(previewPath({ type: 'osr', scoreId: '123', convert: 'mania' }), '/scores/123?convert=mania');
  assert.equal(previewPath({ type: 'osr' }), '/');
});
