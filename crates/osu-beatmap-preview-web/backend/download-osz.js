// OSZ 多镜像竞速下载。
//
// 与 CLI 保持同样的策略：
//   1. 先按固定顺序拿到候选镜像（sayobot / osu.direct / nekoha / catboy），
//      缓存里存在优选 IP 时在 osu.direct 之前插入一个优选候选；
//   2. 最多同时进行 MAX_ACTIVE_ATTEMPTS 个尝试，失败立即补位；
//   3. 某个尝试 3 秒没有首字节、或 5 秒窗口内速度低于 128 KiB/s 时触发回退，
//      必要时取消最慢的尝试再开新镜像；
//   4. 单个尝试优先用 Range 分块并行下载，失败后回退单流下载；
//   5. 全部尝试都有 DOWNLOAD_HARD_TIMEOUT 的硬超时，胜出文件先落到 .part 再改名。

import fsp from 'node:fs/promises';
import {
  BUFFER_SIZE,
  DOWNLOAD_HARD_TIMEOUT,
  LOW_SPEED_BYTES_PER_SECOND,
  LOW_SPEED_WINDOW,
  MAX_ACTIVE_ATTEMPTS,
  MAX_OSZ_BYTES,
  MIRRORS,
  NO_FIRST_BYTE_TIMEOUT,
  PARALLEL_PARTS,
  POLL_INTERVAL,
  USER_AGENT,
} from './config.js';
import { ensurePreferredIp } from './cf-ip.js';
import {
  abortError,
  assertNotHtml,
  createLookup,
  drain,
  httpRequest,
  isAbortError,
  parseContentRange,
  readAll,
} from './http.js';
import { hasZipEndRecord, isZipBuffer } from './zip.js';

const REQUEST_HEADERS = {
  'User-Agent': USER_AGENT,
  'Accept-Encoding': 'identity',
};

/** 下载指定谱面集的 OSZ 到缓存目录，返回缓存文件路径。 */
export async function downloadBeatmapsetArchive({
  cache,
  setId,
  bid,
  log,
  signal,
  deadlineAt,
  fresh = false,
  onProgress,
}) {
  const target = cache.oszPath(setId);
  if (!fresh && (await isValidOsz(target))) {
    log(`OSZ 命中缓存：${setId}.osz`);
    return target;
  }

  const cachedIp = await cache.readPreferredIp();
  if (!cachedIp) {
    // 没有优选 IP 时先在后台探测，本次下载不等待探测结果。
    ensurePreferredIp(cache, { log }).catch(() => {});
  }
  const candidates = buildCandidates(setId, cachedIp);
  log(`OSZ 下载开始：bid=${bid} set=${setId} 候选=${candidates.length}`);

  const startedAt = Date.now();
  const partPath = await raceDownload({
    cache,
    candidates,
    setId,
    log,
    signal,
    deadlineAt,
    onProgress,
  });
  const elapsed = Date.now() - startedAt;

  await fsp.rm(target, { force: true });
  await fsp.rename(partPath, target);
  const size = (await fsp.stat(target)).size;
  log(`OSZ 下载完成：${(size / 1024 / 1024).toFixed(1)} MiB，用时 ${elapsed} ms`);
  return target;
}

function buildCandidates(setId, preferredIp) {
  const candidates = [{ name: 'sayobot', url: MIRRORS.sayobot(setId) }];
  if (preferredIp) {
    candidates.push({
      name: 'osu.direct-preferred-ip',
      url: MIRRORS.osuDirect(setId),
      preferredIp,
    });
  }
  candidates.push(
    { name: 'osu.direct-dns', url: MIRRORS.osuDirect(setId) },
    { name: 'nekoha', url: MIRRORS.nekoha(setId) },
    { name: 'catboy', url: MIRRORS.catboy(setId) },
  );
  return candidates;
}

/**
 * 在候选之间竞速，返回胜出的临时文件路径。
 *
 * 事件循环用一个单槽邮箱把「尝试结束」和「轮询心跳」串起来：心跳负责低速检测
 * 与补位，结束事件负责判定胜负或记录失败。
 */
async function raceDownload({ cache, candidates, setId, log, signal, deadlineAt, onProgress }) {
  const queue = [];
  let waitResolve = null;
  const emit = (event) => {
    if (waitResolve) {
      const resolve = waitResolve;
      waitResolve = null;
      resolve(event);
      return;
    }
    queue.push(event);
  };
  const nextEvent = (ms) =>
    queue.length
      ? Promise.resolve(queue.shift())
      : new Promise((resolve) => {
          const timer = setTimeout(() => {
            waitResolve = null;
            resolve({ type: 'tick' });
          }, ms);
          waitResolve = (event) => {
            clearTimeout(timer);
            resolve(event);
          };
        });

  const active = new Map();
  const failures = [];
  let nextCandidate = 0;
  let nextId = 0;
  let preferredRefreshTriggered = false;
  const startedAt = Date.now();
  let reportedBytes = 0;
  let reportedTotal = 0;

  // 前端只关心「下到多少了」：多个镜像在竞速、单个镜像又分块并行，
  // 所以取所有活跃尝试里的最大值，并且只前进不后退（尝试失败会让瞬时字节数回退）。
  const publishProgress = () => {
    if (!onProgress) return;
    let received = 0;
    let total = 0;
    for (const attempt of active.values()) {
      received = Math.max(received, attempt.progress.bytes);
      total = Math.max(total, attempt.progress.total || 0);
    }
    reportedBytes = Math.max(reportedBytes, received);
    reportedTotal = Math.max(reportedTotal, total);
    onProgress({ received: reportedBytes, total: reportedTotal });
  };

  const startNext = async () => {
    const preferred = await cache.readPreferredIp();
    if (
      preferred &&
      !candidates.some((candidate) => candidate.preferredIp) &&
      candidates[nextCandidate]?.name === 'osu.direct-dns'
    ) {
      candidates.splice(nextCandidate, 0, {
        name: 'osu.direct-preferred-ip',
        url: MIRRORS.osuDirect(setId),
        preferredIp: preferred,
      });
    }
    if (nextCandidate >= candidates.length || active.size >= MAX_ACTIVE_ATTEMPTS) return;

    const candidate = candidates[nextCandidate];
    nextCandidate += 1;
    const id = nextId;
    nextId += 1;
    const controller = new AbortController();
    const attempt = {
      id,
      candidate,
      controller,
      startedAt: Date.now(),
      partPath: `${cache.oszPath(setId)}.${process.pid}.attempt-${id}.part`,
      progress: { bytes: 0, firstByteAt: 0, total: 0 },
      monitor: { samples: [], fallbackTriggered: false },
    };
    await fsp.rm(attempt.partPath, { force: true });
    active.set(id, attempt);
    log(`尝试 ${id} 开始：${candidate.name}`);
    runAttempt({ attempt, log, signal })
      .then((result) => emit({ type: 'settled', id, ...result }))
      .catch((error) => emit({ type: 'settled', id, ok: false, error: error.message }));
  };

  const cancelAll = () => {
    for (const attempt of active.values()) {
      attempt.controller.abort();
      fsp.rm(attempt.partPath, { force: true }).catch(() => {});
    }
    active.clear();
  };

  await startNext();
  for (;;) {
    if (signal?.aborted) {
      cancelAll();
      throw abortError();
    }
    if (Date.now() - startedAt >= DOWNLOAD_HARD_TIMEOUT) {
      cancelAll();
      throw new Error('全局下载超时');
    }
    if (Date.now() >= deadlineAt) {
      cancelAll();
      throw new Error('渲染请求超时');
    }

    const event = await nextEvent(POLL_INTERVAL);
    publishProgress();
    if (event.type === 'settled') {
      const attempt = active.get(event.id);
      active.delete(event.id);
      if (attempt && event.ok) {
        cancelAll();
        publishProgress();
        return attempt.partPath;
      }
      if (attempt) {
        await fsp.rm(attempt.partPath, { force: true });
        if (!event.aborted) {
          failures.push(`${attempt.candidate.name}: ${event.error}`);
          log(`尝试 ${attempt.id} 失败：${attempt.candidate.name} ${event.error}`);
          if (attempt.candidate.preferredIp && !preferredRefreshTriggered) {
            // 优选 IP 失效时清掉缓存并重新探测，下一次候选插入会拿到新地址。
            preferredRefreshTriggered = true;
            await cache.invalidatePreferredIp();
            ensurePreferredIp(cache, { log, force: true }).catch(() => {});
          }
        }
        await startNext();
      }
    }

    const now = Date.now();
    const fallback = detectFallback(active, now);
    if (fallback && nextCandidate < candidates.length) {
      log(`尝试 ${fallback.attempt.id} 触发回退：${fallback.reason}`);
      if (active.size >= MAX_ACTIVE_ATTEMPTS) {
        const slowest = slowestAttempt(active, now);
        if (slowest) {
          log(`取消最慢的尝试 ${slowest.id}（${slowest.candidate.name}）`);
          slowest.controller.abort();
          fsp.rm(slowest.partPath, { force: true }).catch(() => {});
          active.delete(slowest.id);
        }
      }
      await startNext();
    }

    if (active.size === 0 && nextCandidate >= candidates.length) {
      throw new Error(`所有 OSZ 镜像都下载失败：${failures.join('; ')}`);
    }
  }
}

async function runAttempt({ attempt, log, signal }) {
  const { candidate, partPath, progress, controller } = attempt;
  const lookup = createLookup(candidate.preferredIp);
  const onAbort = () => controller.abort();
  signal?.addEventListener('abort', onAbort, { once: true });
  try {
    await downloadOnce({
      candidate,
      partPath,
      lookup,
      progress,
      signal: controller.signal,
      log,
      attemptId: attempt.id,
    });
    await validateArchive(partPath);
    return { ok: true };
  } catch (error) {
    if (isAbortError(error) || controller.signal.aborted) throw abortError();
    return { ok: false, error: error.message };
  } finally {
    signal?.removeEventListener('abort', onAbort);
  }
}

/** 单个镜像的一次完整下载：探测 Range → 分块并行 → 失败回退单流。 */
async function downloadOnce({ candidate, partPath, lookup, progress, signal, log, attemptId }) {
  let probe;
  try {
    probe = await probeRangeSupport({ candidate, lookup, progress, signal });
  } catch (error) {
    if (isAbortError(error)) throw error;
    await fsp.rm(partPath, { force: true });
    await downloadSingle({ candidate, partPath, lookup, progress, signal });
    return;
  }

  if (probe.response) {
    await writeResponse(probe.response, partPath, progress, signal);
    return;
  }

  try {
    await downloadParallel({ candidate, lookup, partPath, total: probe.total, progress, signal });
    log(`尝试 ${attemptId} 分块下载完成：${candidate.name}`);
    return;
  } catch (error) {
    if (isAbortError(error)) throw error;
    await fsp.rm(partPath, { force: true });
    try {
      await downloadSingle({ candidate, partPath, lookup, progress, signal });
    } catch (fallbackError) {
      throw new Error(
        `分块下载失败（${error.message}）；单流回退也失败（${fallbackError.message}）`,
      );
    }
  }
}

/** 用 `Range: bytes=0-0` 探测服务端是否支持分块；不支持时直接返回整包响应。 */
async function probeRangeSupport({ candidate, lookup, progress, signal }) {
  const response = await httpRequest(candidate.url, {
    headers: { ...REQUEST_HEADERS, Range: 'bytes=0-0' },
    lookup,
    signal,
  });
  assertNotHtml(response.headers);
  if (response.status !== 206) {
    // 200 说明镜像不支持 Range，直接把整包响应交给调用方；其他状态码（403 等）
    // 一定是错误响应，直接报出状态码比落到「不是有效压缩包」更容易排查。
    if (response.status === 200) {
      return { response };
    }
    await drain(response.body);
    throw new Error(`HTTP ${response.status}`);
  }
  const contentRange = parseContentRange(response.headers['content-range']);
  if (contentRange.start !== 0 || contentRange.end !== 0) {
    throw new Error(`Range 探测返回了意外的 Content-Range：${response.headers['content-range']}`);
  }
  validateDeclaredSize(contentRange.total);
  // 探测拿到的总长度是进度条的分母。
  progress.total = contentRange.total;
  const byte = await readAll(response.body, { maxBytes: 1, signal });
  if (byte.length !== 1) {
    throw new Error(`Range 探测返回了 ${byte.length} 字节，期望 1 字节`);
  }
  progress.bytes += 1;
  progress.firstByteAt ||= Date.now();
  return { total: contentRange.total };
}

/** 把整包响应写入临时文件，边读边限制大小。 */
async function writeResponse(response, partPath, progress, signal) {
  assertNotHtml(response.headers);
  const declared = Number(response.headers['content-length']);
  if (Number.isFinite(declared)) {
    validateDeclaredSize(declared);
    progress.total = declared;
  }

  const handle = await fsp.open(partPath, 'w');
  try {
    let written = 0;
    for await (const chunk of response.body) {
      if (signal?.aborted) throw abortError();
      written += chunk.length;
      if (written > MAX_OSZ_BYTES) {
        throw new Error(`读取响应时超过 OSZ 大小上限 ${MAX_OSZ_BYTES} 字节`);
      }
      progress.bytes += chunk.length;
      progress.firstByteAt ||= Date.now();
      await handle.write(chunk);
    }
    if (written === 0) {
      throw new Error('服务端返回了空响应');
    }
  } catch (error) {
    await handle.close().catch(() => {});
    await fsp.rm(partPath, { force: true });
    throw error;
  }
  await handle.close();
}

/** 按 PARALLEL_PARTS 切分并并行下载，全部成功后校验文件长度。 */
async function downloadParallel({ candidate, lookup, partPath, total, progress, signal }) {
  const ranges = splitRanges(total, PARALLEL_PARTS);
  const writer = await fsp.open(partPath, 'w+');
  try {
    await writer.truncate(total);
    const workers = ranges.map((range) => {
      const worker = new AbortController();
      const onAbort = () => worker.abort();
      signal?.addEventListener('abort', onAbort, { once: true });
      return downloadRangePart({ candidate, lookup, writer, range, progress, signal: worker.signal })
        .catch((error) => {
          worker.abort();
          throw error;
        })
        .finally(() => signal?.removeEventListener('abort', onAbort));
    });
    await Promise.all(workers);
    const size = (await writer.stat()).size;
    if (size !== total) {
      throw new Error(`分块下载得到 ${size} 字节，期望 ${total} 字节`);
    }
  } catch (error) {
    await writer.close().catch(() => {});
    await fsp.rm(partPath, { force: true });
    throw error;
  }
  await writer.close();
}

async function downloadRangePart({ candidate, lookup, writer, range, progress, signal }) {
  const header = `bytes=${range.start}-${range.end}`;
  const expected = range.end - range.start + 1;
  const response = await httpRequest(candidate.url, {
    headers: { ...REQUEST_HEADERS, Range: header },
    lookup,
    signal,
  });
  assertNotHtml(response.headers);
  if (response.status !== 206) {
    throw new Error(`${header} 返回 HTTP ${response.status}，期望 206`);
  }
  const actual = parseContentRange(response.headers['content-range']);
  if (actual.start !== range.start || actual.end !== range.end || actual.total !== range.total) {
    throw new Error(`${header} 返回了意外的 Content-Range：${response.headers['content-range']}`);
  }
  const declared = Number(response.headers['content-length']);
  if (Number.isFinite(declared) && declared !== expected) {
    throw new Error(`${header} 声明 ${declared} 字节，期望 ${expected} 字节`);
  }

  const buffer = await readAll(response.body, {
    maxBytes: expected,
    signal,
    onBytes: (count) => {
      progress.bytes += count;
      progress.firstByteAt ||= Date.now();
    },
  });
  if (buffer.length !== expected) {
    throw new Error(`${header} 返回 ${buffer.length} 字节，期望 ${expected} 字节`);
  }
  if (signal?.aborted) throw abortError();
  await writer.write(buffer, 0, buffer.length, range.start);
}

/** 单流下载：不使用 Range，按 Content-Length 与上限边读边写。 */
async function downloadSingle({ candidate, partPath, lookup, progress, signal }) {
  const response = await httpRequest(candidate.url, {
    headers: REQUEST_HEADERS,
    lookup,
    signal,
  });
  assertNotHtml(response.headers);
  if (response.status !== 200) {
    await drain(response.body);
    throw new Error(`HTTP ${response.status}`);
  }
  const declared = Number(response.headers['content-length']);
  if (Number.isFinite(declared)) {
    validateDeclaredSize(declared);
    progress.total = declared;
  }

  const handle = await fsp.open(partPath, 'w');
  try {
    let written = 0;
    for await (const chunk of response.body) {
      if (signal?.aborted) throw abortError();
      written += chunk.length;
      if (written > MAX_OSZ_BYTES) {
        throw new Error(`OSZ 超过大小上限 ${MAX_OSZ_BYTES} 字节`);
      }
      progress.bytes += chunk.length;
      progress.firstByteAt ||= Date.now();
      await handle.write(chunk);
    }
    if (written === 0) throw new Error('服务端返回了空响应');
  } catch (error) {
    await handle.close().catch(() => {});
    await fsp.rm(partPath, { force: true });
    throw error;
  }
  await handle.close();
}

/** 轮询低速检测：每 250 ms 采样一次，命中条件即触发回退。 */
function detectFallback(active, now) {
  for (const attempt of active.values()) {
    if (attempt.monitor.fallbackTriggered) continue;
    const reason = fallbackReason(attempt, now);
    if (reason) {
      attempt.monitor.fallbackTriggered = true;
      return { attempt, reason };
    }
  }
  return null;
}

function fallbackReason(attempt, now) {
  const { monitor, progress } = attempt;
  monitor.samples.push({ at: now, bytes: progress.bytes });
  while (
    monitor.samples.length > 1 &&
    now - monitor.samples[1].at >= LOW_SPEED_WINDOW
  ) {
    monitor.samples.shift();
  }

  if (!progress.firstByteAt && now - attempt.startedAt >= NO_FIRST_BYTE_TIMEOUT) {
    return 'no-first-byte';
  }
  const oldest = monitor.samples[0];
  if (!oldest) return null;
  const elapsed = now - oldest.at;
  const received = progress.bytes - oldest.bytes;
  if (
    elapsed >= LOW_SPEED_WINDOW &&
    received * 1000 < LOW_SPEED_BYTES_PER_SECOND * elapsed
  ) {
    return 'low-speed';
  }
  return null;
}

function recentBytesPerSecond(attempt, now) {
  const oldest = attempt.monitor.samples[0];
  if (!oldest) return 0;
  const elapsed = now - oldest.at;
  if (elapsed <= 0) return 0;
  return ((attempt.progress.bytes - oldest.bytes) * 1000) / elapsed;
}

function slowestAttempt(active, now) {
  const entries = [...active.values()].map((attempt) => ({
    attempt,
    speed: recentBytesPerSecond(attempt, now),
  }));
  entries.sort((left, right) =>
    left.speed === right.speed ? right.attempt.id - left.attempt.id : left.speed - right.speed,
  );
  return entries[0]?.attempt ?? null;
}

/** 按 `download.osz.PARALLEL_PARTS` 的语义切分区间。 */
export function splitRanges(total, requestedParts) {
  const partCount = Math.max(1, Math.min(requestedParts, total));
  const chunkSize = Math.ceil(total / partCount);
  const ranges = [];
  for (let start = 0; start < total; ) {
    const end = Math.min(start + chunkSize - 1, total - 1);
    ranges.push({ start, end, total });
    start = end + 1;
  }
  return ranges;
}

function validateDeclaredSize(length) {
  if (!Number.isFinite(length)) return;
  if (length === 0) throw new Error('服务端声明了空响应（Content-Length: 0）');
  if (length > MAX_OSZ_BYTES) {
    throw new Error(
      `OSZ 过大：服务端声明 ${length} 字节，超过上限 ${MAX_OSZ_BYTES} 字节`,
    );
  }
}

/** 校验缓存里的 OSZ：大小合法且尾部存在 ZIP 中央目录记录。 */
export async function isValidOsz(target) {
  let stat;
  try {
    stat = await fsp.stat(target);
  } catch {
    return false;
  }
  if (!stat.isFile() || stat.size === 0 || stat.size > MAX_OSZ_BYTES) return false;
  // 命中缓存时只读尾部：完整解析留到真正需要条目时再做，避免每次请求读整包。
  const tailSize = Math.min(stat.size, 64 * 1024 + 4096);
  const handle = await fsp.open(target, 'r').catch(() => null);
  if (!handle) return false;
  try {
    const tail = Buffer.alloc(tailSize);
    await handle.read(tail, 0, tailSize, stat.size - tailSize);
    return hasZipEndRecord(tail);
  } catch {
    return false;
  } finally {
    await handle.close().catch(() => {});
  }
}

async function validateArchive(partPath) {
  const stat = await fsp.stat(partPath);
  if (stat.size === 0) throw new Error('下载结果为空');
  if (stat.size > MAX_OSZ_BYTES) {
    throw new Error(`下载结果超过上限 ${MAX_OSZ_BYTES} 字节`);
  }
  // 下载完成后做一次完整解析，避免把损坏的压缩包写进缓存。
  const buffer = await fsp.readFile(partPath);
  if (!isZipBuffer(buffer)) {
    throw new Error('响应不是有效的 ZIP/OSZ 压缩包');
  }
}
