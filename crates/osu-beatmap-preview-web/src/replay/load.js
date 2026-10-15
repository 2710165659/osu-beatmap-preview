// OSR 输入与谱面资源绑定；回放文件保留浏览器本地，服务端只收到谱面 checksum。
export async function readReplayFile(file, wasm) {
  if (!/\.osr$/i.test(file?.name ?? '')) throw new Error('请选择 .osr 回放文件');
  if (file.size > 64 * 1024 * 1024) throw new Error('回放文件超过 64 MiB');
  const bytes = new Uint8Array(await file.arrayBuffer());
  const info = wasm.replayInfo(bytes);
  return { bytes, info, name: file.name };
}

export async function resolveReplaySource({ wasm, replay, localBytes, localKind, download, lookup, signal }) {
  const selector = { checksum: replay.info.checksum };
  if (localBytes) {
    // 用户给出的资源必须先通过 MD5 绑定；不匹配时拒绝，不能悄悄换谱。
    const info = wasm.beatmapInfo(localBytes, selector);
    if (localKind === 'osz') return { bytes: localBytes, selector, info };
    const bid = String(info.beatmapId ?? '');
    const resolved = /^\d+$/.test(bid) && Number(bid) > 0 ? { bid } : await lookup(replay.info.checksum, signal);
    const bytes = await download(resolved.bid, { signal });
    return { bytes, selector, info: wasm.beatmapInfo(bytes, selector) };
  }
  const resolved = replay.location ?? await lookup(replay.info.checksum, signal);
  const bytes = await download(resolved.bid, { signal });
  return { bytes, selector, info: wasm.beatmapInfo(bytes, selector) };
}

export async function lookupReplayBeatmap(checksum, signal) {
  const response = await fetch(`/resource/replay-lookup?checksum=${encodeURIComponent(checksum)}`, { signal });
  if (!response.ok) throw new Error(await response.text());
  return response.json();
}

/** 同一文件快速替换时只提交最后一次解析结果，离开加载页无需保存旧字节。 */
export function createReplayInput(state, loadWasm) {
  let input = null;
  let selection = 0;
  return {
    current: () => input,
    async select(file) {
      const token = ++selection;
      state.replaySelecting = true;
      try {
        const result = await readReplayFile(file, await loadWasm());
        if (token !== selection) return;
        input = result;
        state.replayFileName = result.name;
        state.replayChecksum = result.info.checksum;
        state.convert = '';
      } catch (error) {
        if (token === selection) throw error;
      } finally {
        if (token === selection) state.replaySelecting = false;
      }
    },
    clear() {
      selection += 1;
      input = null;
      state.replayFileName = '';
      state.replayChecksum = '';
      state.replaySelecting = false;
    },
  };
}
