<script setup>
// 常驻进度条：只负责显示进度与 seek。
//
// 隐藏时只改透明度，盒子仍占着原来的位置，所以视频区域不会因为淡出而位移；
// 鼠标移动会重新显示，指针停在进度条上或焦点在里面时保持可见。
import { formatTime, pinTimeline, seekTo, showTimeline, state } from '../preview.js';
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
      <span class="w-11 shrink-0 text-right font-mono text-[11px] text-neutral-400">
        {{ formatTime(state.position) }}
      </span>
      <input
        class="h-8 min-w-0 flex-1 cursor-pointer"
        type="range"
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
    </div>
  </div>
</template>
