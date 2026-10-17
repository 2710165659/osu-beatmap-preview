// 谱面文本与媒体包分别保存；替换或清除其中一个输入不影响另一个。
export function createLocalInputs(state, loadWasm) {
  const bytes = { osu: null, osz: null };
  const tokens = { osu: 0, osz: 0 };
  const fields = { osu: 'localOsu', osz: 'localOsz' };
  return {
    current: () => ({ osuBytes: bytes.osu, oszBytes: bytes.osz }),
    async select(kind, file) {
      const field = fields[kind];
      if (!field) throw new Error('未知的谱面文件类型');
      const token = ++tokens[kind];
      state[`${field}Selecting`] = true;
      try {
        if (!file?.name?.toLowerCase().endsWith(`.${kind}`)) throw new Error(`请选择 .${kind} 文件`);
        const raw = new Uint8Array(await file.arrayBuffer());
        const wasm = await loadWasm();
        const info = wasm.beatmapInfo(raw, {});
        if (token !== tokens[kind]) return;
        bytes[kind] = raw;
        state[`${field}Name`] = file.name;
        if (kind === 'osz') {
          state.localDifficulties = info.difficulties ?? [];
          const matched = state.localDifficulties.find((item) => String(item.beatmapId) === state.bid.trim());
          state.localDifficulty = (matched ?? state.localDifficulties[0])?.entry ?? '';
        }
      } catch (error) {
        if (token === tokens[kind]) throw error;
      } finally {
        if (token === tokens[kind]) state[`${field}Selecting`] = false;
      }
    },
    clear(kind) {
      const field = fields[kind];
      tokens[kind] += 1;
      bytes[kind] = null;
      state[`${field}Name`] = '';
      state[`${field}Selecting`] = false;
      if (kind === 'osz') { state.localDifficulties = []; state.localDifficulty = ''; }
    },
  };
}

export async function resolveBeatmapSource({ wasm, osuBytes, oszBytes, bid, difficulty, download, signal }) {
  if (bid && !/^[1-9]\d*$/.test(bid)) throw new Error('BID 必须为正整数');
  // 外部 .osu 决定实际画面；BID 优先校验，只有单独的包才使用难度下拉选择。
  const selector = bid ? { bid } : !osuBytes && difficulty ? { difficulty } : {};
  if (osuBytes && oszBytes) selector.osuBytes = osuBytes;
  const bytes = oszBytes ?? osuBytes ?? await download(bid, { signal });
  return { bytes, selector, info: wasm.beatmapInfo(bytes, selector), hasMusic: Boolean(oszBytes || !osuBytes) };
}
