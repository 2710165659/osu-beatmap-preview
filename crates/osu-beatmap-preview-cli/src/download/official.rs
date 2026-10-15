//! 官方 OAuth 与 checksum 查询；认证数据只在宿主内使用，不进入日志和 core。
use osu_beatmap_preview_core::support::timeout::RequestDeadline;
use osu_beatmap_preview_core::{PreviewError, Result};
use serde::Deserialize;
use std::io::Read;
use std::time::Duration;
pub(crate) mod scores;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Lookup {
    pub id: u64,
    pub beatmapset_id: u64,
    pub checksum: String,
}
pub(crate) fn lookup(checksum: &str, deadline: &RequestDeadline) -> Result<Lookup> {
    let client = Client::new(deadline)?;
    let result: Lookup = client.get("beatmaps/lookup", &[("checksum", checksum)])?;
    deadline.check()?;
    if result.id == 0
        || result.beatmapset_id == 0
        || !result.checksum.eq_ignore_ascii_case(checksum)
    {
        return Err(PreviewError::download(
            "official checksum lookup returned a mismatched beatmap",
        ));
    }
    Ok(result)
}

/// 每次请求拥有认证与总截止时间，凭据不会进入 URL、错误链或缓存键。
pub(super) struct Client<'a> {
    agent: ureq::Agent,
    token: String,
    deadline: &'a RequestDeadline,
    base: String,
}
impl<'a> Client<'a> {
    pub fn new(deadline: &'a RequestDeadline) -> Result<Self> {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(deadline.cap(Duration::from_secs(20))?)
            .build();
        let token = match std::env::var("OSU_API_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
        {
            Some(token) => token,
            None => {
                let id = std::env::var("OSU_CLIENT_ID").map_err(|_| credentials_error())?;
                let secret = std::env::var("OSU_CLIENT_SECRET").map_err(|_| credentials_error())?;
                let body = serde_json::json!({"client_id": id, "client_secret": secret,
                "grant_type": "client_credentials", "scope": "public"});
                let response = agent
                    .post("https://osu.ppy.sh/oauth/token")
                    .set("Content-Type", "application/json")
                    .send_string(&body.to_string())
                    .map_err(|error| http_error("OAuth", error))?;
                let value: serde_json::Value = read_json(response)?;
                value["access_token"]
                    .as_str()
                    .ok_or_else(credentials_error)?
                    .to_string()
            }
        };
        Ok(Self {
            agent,
            token,
            deadline,
            base: "https://osu.ppy.sh/api/v2/".into(),
        })
    }
    pub fn response(&self, path: &str, query: &[(&str, &str)]) -> Result<ureq::Response> {
        self.deadline.check()?;
        let mut request = self
            .agent
            .get(&format!("{}{path}", self.base))
            .timeout(self.deadline.cap(Duration::from_secs(20))?)
            .set("Authorization", &format!("Bearer {}", self.token))
            // 请求统一成绩格式，避免 API 默认旧版把 lazer 成绩映射为 legacy ID。
            .set("x-api-version", "20220705");
        for (key, value) in query {
            request = request.query(key, value);
        }
        request.call().map_err(|error| http_error("request", error))
    }
    pub fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let result = read_json(self.response(path, query)?)?;
        self.deadline.check()?;
        Ok(result)
    }
}
fn read_json<T: serde::de::DeserializeOwned>(response: ureq::Response) -> Result<T> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreviewError::download("official API response read failed"))?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(PreviewError::download(
            "official API response exceeds limit",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| PreviewError::download("invalid official API response"))
}
fn credentials_error() -> PreviewError {
    PreviewError::download("official API requires OSU_API_TOKEN or OSU_CLIENT_ID + OSU_CLIENT_SECRET; offline input requires --osr-file and matching --osz-file")
}
fn http_error(operation: &str, error: ureq::Error) -> PreviewError {
    // 错误链可能含认证请求信息，因此只报告操作名与状态码。
    let detail = match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        _ => "network failure".into(),
    };
    PreviewError::download(format!("official {operation} failed: {detail}"))
}
