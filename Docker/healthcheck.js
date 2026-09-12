// 容器健康检查。
//
// HEALTHCHECK 在镜像里是静态的，但 CMD 可以被覆盖（换端口、开 HTTPS、换监听地址），
// 写死 `http://127.0.0.1:8787/` 会让所有非默认启动方式永远 unhealthy。所以这里从
// PID 1 的命令行里读 --port / --https / --tls-*，再用对应的协议请求一次首页。
import fs from 'node:fs';
import http from 'node:http';
import https from 'node:https';

/** 与 backend/config.js 的 DEFAULT_PORT 保持一致。 */
const DEFAULT_PORT = 8787;

/** /proc/1/cmdline 用 NUL 分隔，末尾也有一个 NUL。 */
function readArguments() {
  try {
    return fs.readFileSync('/proc/1/cmdline', 'utf8').split('\0').filter(Boolean);
  } catch {
    // 读不到（比如被 --entrypoint 换成别的进程）时退回默认端口，让检查如实失败。
    return [];
  }
}

/** 取监听端口；`--port=8443` 与 `--port 8443` 两种写法都认。 */
function readPort(argv) {
  for (let index = 0; index < argv.length; index += 1) {
    const [name, inline] = argv[index].split('=');
    if (name !== '--port') continue;
    const value = inline ?? argv[index + 1];
    const port = Number(value);
    if (Number.isInteger(port) && port > 0 && port <= 65535) return port;
  }
  return DEFAULT_PORT;
}

const argv = readArguments();
const port = readPort(argv);
const secure = argv.some((value) => value.startsWith('--https')
  || value.startsWith('--tls-cert')
  || value.startsWith('--tls-key')
  || value.startsWith('--tls-host'));
const scheme = secure ? 'https' : 'http';
const endpoint = `${scheme}://127.0.0.1:${port}/`;

const request = (secure ? https : http).request({
  host: '127.0.0.1',
  port,
  path: '/',
  method: 'GET',
  // 自签证书是预期内的，健康检查不校验证书。
  rejectUnauthorized: false,
  timeout: 5_000,
}, (response) => {
  response.resume();
  if (response.statusCode === 200) {
    // 输出会留在 docker inspect 的健康检查日志里，也可以直接手跑这个脚本自检。
    process.stdout.write(`ok ${endpoint}\n`);
    process.exit(0);
  }
  process.stderr.write(`unexpected status ${response.statusCode} from ${endpoint}\n`);
  process.exit(1);
});

request.on('timeout', () => request.destroy());
request.on('error', (error) => {
  process.stderr.write(`cannot reach ${endpoint}: ${error.message}\n`);
  process.exit(1);
});
request.end();
