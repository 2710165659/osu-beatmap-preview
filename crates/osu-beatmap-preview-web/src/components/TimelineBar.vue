<script setup>
// 常驻进度条：只负责显示进度与 seek。
//
// 隐藏时只改透明度，盒子仍占着原来的位置，所以视频区域不会因为淡出而位移；
// 鼠标移动会重新显示，指针停在进度条上或焦点在里面时保持可见。
import { formatTime, pinTimeline, seekTo, showTimeline, state, togglePlay } from '../preview.js';

defineProps({ fullscreen: Boolean });
defineEmits(['toggle-fullscreen']);
</script>

<template>
  <div
    class="shrink-0 border-t border-neutral-800 bg-[#111419] px-3 py-2 transition-opacity duration-200 ease-out"
    :class="state.timelineVisible ? 'opacity-100' : 'opacity-0'"
    @pointerenter="pinTimeline('hover', true)"
    @pointerleave="pinTimeline('hover', false)"
    @focusin="pinTimeline('focus', true)"
    @focusout="pinTimeline('focus', false)"
  >
    <div class="flex items-center gap-3">
      <button
        type="button"
        class="grid h-8 w-8 shrink-0 place-items-center rounded border border-neutral-700 bg-neutral-900 text-sm text-neutral-200 transition hover:border-neutral-500 focus-visible:outline-2 focus-visible:outline-[#ff5f45]"
        :aria-label="state.playing ? '暂停' : '继续播放'"
        :title="state.playing ? '暂停' : '继续播放'"
        @click="togglePlay()"
      >
        {{ state.playing ? 'Ⅱ' : '▶' }}
      </button>
      <span class="w-11 shrink-0 text-right font-mono text-[11px] text-neutral-400">
        {{ formatTime(state.position) }}
      </span>
      <input
        class="h-8 min-w-0 flex-1 cursor-pointer outline-none select-none"
        type="range"
        tabindex="-1"
        min="0"
        :max="state.duration"
        step="100"
        :value="state.position"
        aria-label="播放进度"
        @input="seekTo(Number($event.target.value)); showTimeline()"
      >
      <span class="w-11 shrink-0 font-mono text-[11px] text-neutral-400">
        {{ formatTime(state.duration) }}
      </span>
      <button
        type="button"
        class="grid h-8 w-8 shrink-0 place-items-center rounded border border-neutral-700 bg-neutral-900 text-base text-neutral-200 transition hover:border-neutral-500 focus-visible:outline-2 focus-visible:outline-[#ff5f45]"
        :aria-label="fullscreen ? '退出全屏' : '全屏播放'"
        :title="fullscreen ? '退出全屏' : '全屏播放'"
        @click="$emit('toggle-fullscreen')"
      >
        {{ fullscreen ? '⛶' : '⛶' }}
      </button>
    </div>
  </div>
</template>
