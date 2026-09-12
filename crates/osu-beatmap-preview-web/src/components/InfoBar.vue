<script setup>
// 谱面信息条：夹在顶栏与视频之间的一行文字。
//
// 只展示名称与难度，长名字截断显示，既不遮挡画面也不占掉视频的高度。
// 数据来自 WASM 的 `beatmapInfo`（全量输出），这里只取其中两个字段。
import { computed } from 'vue';
import { state } from '../preview.js';

// osu! 客户端优先显示 Unicode 名称，缺失时才回退到 ASCII 字段。
const pick = (unicodeKey, asciiKey) => {
  const info = state.info;
  if (!info) return '';
  return (info[unicodeKey] || info[asciiKey] || '').trim();
};

const songName = computed(() => {
  const artist = pick('artistUnicode', 'artist');
  const title = pick('titleUnicode', 'title');
  if (artist && title) return `${artist} - ${title}`;
  return title || artist;
});

const version = computed(() => (state.info?.version ?? '').trim());
</script>

<template>
  <div
    v-if="songName || version"
    class="flex h-6 shrink-0 items-center gap-2 overflow-hidden border-b border-neutral-800 bg-[#0b0d10] px-3 text-[11px]"
  >
    <span class="min-w-0 truncate text-neutral-300">{{ songName }}</span>
    <span v-if="version" class="shrink-0 text-neutral-500">[{{ version }}]</span>
  </div>
</template>
