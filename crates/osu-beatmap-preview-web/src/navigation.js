// 分享链接只从路径读取 ID，避免旧查询参数与当前预览模式互相覆盖。
export function readPreviewLink(pathname, search = '') {
  const match = /^\/(bids|scores)\/([1-9]\d*)\/?$/.exec(pathname);
  if (!match) return null;
  const convert = new URLSearchParams(search).get('convert') ?? '';
  const options = { convert: ['standard', 'taiko', 'catch', 'mania'].includes(convert) ? convert : '' };
  return match[1] === 'scores'
    ? { type: 'osr', scoreId: match[2], ...options }
    : { type: 'osu', bid: match[2], ...options };
}

export function previewPath({ type, bid, scoreId, convert = '' }) {
  const id = String(type === 'osr' ? scoreId ?? '' : bid ?? '').trim();
  if (!/^[1-9]\d*$/.test(id)) return '/';
  const path = `/${type === 'osr' ? 'scores' : 'bids'}/${id}`;
  return convert ? `${path}?${new URLSearchParams({ convert })}` : path;
}
