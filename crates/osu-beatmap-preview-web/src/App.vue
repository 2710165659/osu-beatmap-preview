<script setup>
// 根组件：加载页与播放页同时挂载，用 v-show 切换。
//
// 播放页必须提前挂载：WASM 会话在「加载预览」时就需要拿到 canvas 元素，
// 而 WebGPU 上下文在 display:none 的 canvas 上同样可以创建（旧版页面就是这么做的）。
import { onBeforeUnmount, onMounted } from 'vue';
import LoadPage from './components/LoadPage.vue';
import PlayPage from './components/PlayPage.vue';
import {
  cancelArrowHold,
  loadPreview,
  pressArrowKey,
  readDeepLink,
  releaseArrowKey,
  setSheetOpen,
  state,
  togglePlay,
} from './preview.js';

const isEditable = (target) =>
  target instanceof HTMLElement
  && (target.isContentEditable || ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName));

/** 左右方向键：短按在松开时跳转 ±5 秒，长按进入快进 / 倒带（实现见 preview.js）。 */
const arrowSide = (code) => (code === 'ArrowLeft' ? 'left' : code === 'ArrowRight' ? 'right' : null);

function onKeydown(event) {
  if (state.page !== 'play') return;
  if (event.code === 'Escape') {
    setSheetOpen(false);
    return;
  }
  // 输入框和下拉框自己要用空格与方向键，不要抢。
  if (isEditable(event.target)) return;
  if (event.code === 'Space') {
    event.preventDefault();
    // 按住空格不要反复播放/暂停。
    if (!event.repeat) togglePlay();
    return;
  }
  const side = arrowSide(event.code);
  if (!side) return;
  // 阻止页面滚动；长按的重复事件不重复起定时器。
  event.preventDefault();
  if (!event.repeat) pressArrowKey(side);
}

function onKeyup(event) {
  if (state.page !== 'play') return;
  const side = arrowSide(event.code);
  if (side) releaseArrowKey(side);
}

onMounted(() => {
  document.addEventListener('keydown', onKeydown);
  document.addEventListener('keyup', onKeyup);
  // 切走窗口时可能收不到 keyup，清掉按键状态避免卡在快进/倒带。
  window.addEventListener('blur', cancelArrowHold);
  // ?bid=xxx 直接进预览（原模式）。子组件的 onMounted 先于父组件执行，
  // 所以这里 canvas 已经登记好，可以直接加载。
  const deepLink = readDeepLink();
  if (!deepLink) return;
  state.bid = deepLink.bid;
  state.convert = deepLink.convert;
  loadPreview();
});

onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKeydown);
  document.removeEventListener('keyup', onKeyup);
  window.removeEventListener('blur', cancelArrowHold);
});
</script>

<template>
  <div class="app-viewport bg-[#0b0d10] text-neutral-200">
    <LoadPage v-show="state.page === 'load'" />
    <PlayPage v-show="state.page === 'play'" />
  </div>
</template>
