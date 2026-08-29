use std::time::Duration;

/// Cloudflare IP 缓存有效期。
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(86_400);
/// TCP 连接尝试超时时间。
pub(crate) const TCP_TIMEOUT: Duration = Duration::from_millis(1_200);
/// HTTP 请求超时时间。
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_millis(2_500);
/// 并行探测的 HTTP 候选数量。
pub(crate) const HTTP_CANDIDATES: usize = 8;
/// Cloudflare 官方 IPv4 网段列表。
pub(crate) const CLOUDFLARE_IPV4_RANGES: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/12",
    "172.64.0.0/17",
    "172.64.128.0/18",
    "172.64.192.0/19",
    "172.64.224.0/22",
    "172.64.229.0/24",
    "172.64.230.0/23",
    "172.64.232.0/21",
    "172.64.240.0/21",
    "172.64.248.0/21",
    "172.65.0.0/16",
    "172.66.0.0/16",
    "172.67.0.0/16",
    "131.0.72.0/22",
];

/// MiB 对应的字节数。
pub(crate) const MIB_BYTES: u64 = 1_048_576;
/// 下载读写缓冲区大小（字节）。
pub(crate) const BUFFER_SIZE: usize = 65_536;
/// 下载状态轮询间隔。
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// 首字节等待超时时间。
pub(crate) const NO_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(3);
/// 低速检测窗口时长。
pub(crate) const LOW_SPEED_WINDOW: Duration = Duration::from_secs(5);
/// 低速阈值（字节/秒）。
pub(crate) const LOW_SPEED_BYTES_PER_SECOND: u64 = 131_072;
/// TCP 连接超时时间。
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// HTTP 读取超时时间。
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// HTTP 写入超时时间。
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
