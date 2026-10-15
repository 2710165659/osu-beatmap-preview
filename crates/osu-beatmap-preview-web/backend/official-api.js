// 官方 OAuth 客户端；凭据只存在服务端，共用令牌缓存与并发认证请求。
export function createOfficialApi({ fetchImpl = fetch, env = process.env, now = Date.now } = {}) {
  let cachedToken = null;
  let pendingToken = null;
  async function request(url, options = {}) {
    let response;
    try { response = await fetchImpl(url, { ...options, signal: AbortSignal.timeout(20_000), redirect: 'error' }); }
    catch { throw new Error('官方 osu! API 网络请求失败'); }
    if (!response.ok) throw new Error(`官方 osu! API 请求失败：HTTP ${response.status}`);
    return response;
  }
  async function json(response) {
    try { return await response.json(); } catch { throw new Error('官方 osu! API 响应无效'); }
  }
  async function token() {
    if (env.OSU_API_TOKEN?.trim()) return env.OSU_API_TOKEN.trim();
    if (cachedToken && cachedToken.expires > now()) return cachedToken.value;
    if (!env.OSU_CLIENT_ID || !env.OSU_CLIENT_SECRET) throw new Error('自动查谱和下载回放需要后端配置 OSU_API_TOKEN 或 OSU_CLIENT_ID + OSU_CLIENT_SECRET；也可上传本地 OSR 和匹配的 .osz');
    if (!pendingToken) pendingToken = (async () => {
      const result = await json(await request('https://osu.ppy.sh/oauth/token', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ client_id: env.OSU_CLIENT_ID, client_secret: env.OSU_CLIENT_SECRET, grant_type: 'client_credentials', scope: 'public' }) }));
      if (typeof result.access_token !== 'string' || !result.access_token) throw new Error('官方 OAuth 响应缺少 token');
      cachedToken = { value: result.access_token, expires: now() + Math.max(0, Number(result.expires_in) - 60) * 1000 };
      return cachedToken.value;
    })().finally(() => { pendingToken = null; });
    return pendingToken;
  }
  async function response(path) {
    return request(`https://osu.ppy.sh/api/v2/${path}`, { headers: { Authorization: `Bearer ${await token()}`, 'x-api-version': '20220705' } });
  }
  return { response, get: async (path) => json(await response(path)) };
}
