<script setup>
// 加载页：输入 BID 或选择本地 .osu / .osz 文件，可选转谱模式，然后交给 preview.js 加载会话。
// 本地 .osz 通常包含多个难度：文件选好后立刻解析出难度清单让用户选择（填了 BID 时
// 默认选中对应 BeatmapID 的难度，加载时按 BID 匹配、找不到会报错）。
// 加载期间显示后端与浏览器两段传输的进度：OSZ 动辄几十 MiB，还要经服务端转发一次，
// 没有阶段与速率会让人以为卡死了。
import { ref } from 'vue';

import {
  CONVERT_MODES,
  clearLocalFile,
  loadPreview,
  progressDetail,
  progressLabel,
  progressPercent,
  selectLocalFile,
  state,
} from '../preview.js';

/** 本地文件选择阶段的错误（后缀不对、压缩包里没有 .osu 等），就地显示。 */
const localError = ref('');
/** 文件输入框：清除选择时要同步清空它的值，否则同一个文件再次选择不会触发 change。 */
const fileInput = ref(null);

async function onFileChange(event) {
  const file = event.target.files?.[0] ?? null;
  localError.value = '';
  if (!file) {
    clearLocalFile();
    return;
  }
  try {
    await selectLocalFile(file);
  } catch (error) {
    clearLocalFile();
    localError.value = String(error?.message ?? error);
    // 清空输入框，否则同一个文件再次选择不会触发 change。
    event.target.value = '';
  }
}

/** 移除已选文件，回到按 BID 加载。 */
function removeFile() {
  clearLocalFile();
  localError.value = '';
  if (fileInput.value) fileInput.value.value = '';
}
</script>

<template>
  <section class="min-h-0 flex-1 overflow-auto">
    <div class="mx-auto w-full max-w-[720px] px-5 pt-[9vh] pb-10 sm:px-7">
      <div class="mb-9 flex items-center gap-3.5">
        <span class="grid h-12 w-12 shrink-0 place-items-center rounded-xl bg-[#ff5f45] text-[22px] font-extrabold text-white">
          o!
        </span>
        <div>
          <small class="text-[10px] tracking-[.16em] text-[#ff5f45]">LOCAL RENDER ENGINE</small>
          <h1 class="m-0 text-[26px] leading-tight font-bold text-white">谱面预览</h1>
        </div>
      </div>

      <form class="grid gap-4" @submit.prevent="loadPreview()">
        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          谱面 BID
          <input
            v-model="state.bid"
            inputmode="numeric"
            autocomplete="off"
            placeholder="例如 738063；选择 .osz 时可指定难度"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
        </label>

        <div class="flex items-center gap-3 text-[11px] tracking-wider text-neutral-600 select-none">
          <span class="h-px flex-1 bg-neutral-800"></span>
          或选择本地文件（.osu / .osz）
          <span class="h-px flex-1 bg-neutral-800"></span>
        </div>

        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          本地文件
          <input
            ref="fileInput"
            type="file"
            accept=".osu,.osz"
            class="block w-full rounded-md border border-neutral-700 bg-neutral-900 pr-3 pl-3 pt-2.5 pb-2.5 text-[13px] font-normal text-neutral-400 file:mr-3 file:h-7 file:rounded file:border-0 file:bg-neutral-800 file:px-2.5 file:text-[12px] file:font-bold file:text-white"
            @change="onFileChange"
          >
        </label>

        <p v-if="state.localFileName" class="-mt-2 flex items-start gap-2 text-[12px] text-neutral-500">
          <span class="min-w-0 flex-1">
            已选择：{{ state.localFileName }}（本地文件优先于 BID；填 BID 仅用于在 .osz 内指定难度）
            <span v-if="state.localKind === 'osu'">本地 .osu 没有音频与背景，纯画面预览。</span>
          </span>
          <button
            type="button"
            class="shrink-0 font-bold text-[#ff5f45] hover:underline"
            @click="removeFile"
          >
            移除
          </button>
        </p>

        <label v-if="state.localKind === 'osz' && state.localDifficulties.length" class="grid gap-2 text-[13px] font-bold text-neutral-300">
          难度（本地 .osz）
          <select
            v-model="state.localDifficulty"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
            <option v-for="item in state.localDifficulties" :key="item.entry" :value="item.entry">
              {{ item.label }}
            </option>
          </select>
        </label>

        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          转谱
          <select
            v-model="state.convert"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
            <option value="">原模式</option>
            <option v-for="mode in CONVERT_MODES" :key="mode" :value="mode">{{ mode }}</option>
          </select>
        </label>

        <button
          type="submit"
          :disabled="state.loading"
          class="h-11 rounded-md bg-[#ff5f45] font-bold text-white transition hover:brightness-105 disabled:opacity-60"
        >
          {{ state.loading ? '加载中...' : '加载预览' }}
        </button>
      </form>

      <p
        v-if="localError"
        class="mt-5 rounded-md border border-red-700/60 bg-red-950/40 p-3 text-xs leading-relaxed text-red-300"
      >
        {{ localError }}
      </p>

      <div v-if="state.loading" class="mt-5" aria-live="polite">
        <div class="flex items-baseline justify-between gap-3 text-xs text-neutral-400">
          <span class="truncate">{{ progressLabel }}</span>
          <span v-if="progressDetail" class="shrink-0 font-mono text-[11px] text-neutral-500">
            {{ progressDetail }}
          </span>
        </div>
        <div class="mt-2 flex items-center gap-2">
          <div class="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-neutral-800">
            <div
              class="h-full rounded-full bg-[#ff5f45] transition-[width] duration-200 ease-out"
              :class="progressPercent === null ? 'w-1/3 animate-pulse' : ''"
              :style="progressPercent === null ? null : { width: `${progressPercent}%` }"
            />
          </div>
          <span v-if="progressPercent !== null" class="w-9 shrink-0 text-right font-mono text-[11px] text-neutral-400">
            {{ progressPercent }}%
          </span>
        </div>
      </div>

      <p
        v-if="!state.gpuAvailable"
        class="mt-5 rounded-md border border-amber-700/60 bg-amber-950/40 p-3 text-xs leading-relaxed text-amber-300"
      >
        <template v-if="!state.secureContext">
          当前页面不是安全上下文（<code>http://</code> 加 IP 或域名），浏览器不会暴露
          <code>navigator.gpu</code>，所以无法渲染。WebGPU 只允许
          <code>https://</code>、<code>localhost</code> 和 <code>127.0.0.1</code>，
          请改用 <code>https://</code> 访问本站（自签证书点“继续访问”也可以）。
        </template>
        <template v-else>
          当前浏览器没有可用的 WebGPU（<code>navigator.gpu</code> 为空），页面无法渲染。
        </template>
        可以打开
        <a class="underline" href="/gpu-check.html" target="_blank" rel="noreferrer">/gpu-check.html</a>
        查看具体原因。
      </p>

      <pre
        v-if="state.loadLogs.length"
        class="mt-5 max-h-32 overflow-auto font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-[#ff9d8a]"
      >{{ state.loadLogs.join('\n') }}</pre>
    </div>
  </section>
</template>
