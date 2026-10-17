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
                let (id, secret) =
                    oauth_credentials(&crate::config::current().download.auth, |name| {
                        std::env::var(name).ok()
                    })?;
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

fn oauth_credentials(
    config: &crate::config::DownloadAuthConfig,
    env: impl Fn(&str) -> Option<String>,
) -> Result<(String, String)> {
    // 两个字段分别回退，空白环境变量视为未配置；不修改进程环境，避免并发请求相互影响。
    let read = |name: &str, fallback: &str| {
        env(name)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string())
    };
    let id = read("OSU_CLIENT_ID", &config.OSU_CLIENT_ID);
    let secret = read("OSU_CLIENT_SECRET", &config.OSU_CLIENT_SECRET);
    if id.trim().is_empty() || secret.trim().is_empty() {
        return Err(credentials_error());
    }
    Ok((id, secret))
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
    PreviewError::download("官方 API 需要 OSU_API_TOKEN 环境变量，或 OSU_CLIENT_ID + OSU_CLIENT_SECRET（优先环境变量，其次 download.auth 配置）；离线使用需提供 --osr-file 和匹配的 --osz-file")
}
fn http_error(operation: &str, error: ureq::Error) -> PreviewError {
    // 错误链可能含认证请求信息，因此只报告操作名与状态码。
    let detail = match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        _ => "network failure".into(),
    };
    PreviewError::download(format!("official {operation} failed: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_credentials_prefer_environment_and_fall_back_per_field() {
        let config = crate::config::DownloadAuthConfig {
            OSU_CLIENT_ID: "configured-id".into(),
            OSU_CLIENT_SECRET: "configured-secret".into(),
        };
        let configured = ("configured-id".into(), "configured-secret".into());
        assert_eq!(oauth_credentials(&config, |_| None).unwrap(), configured);
        assert_eq!(
            oauth_credentials(&config, |_| Some(" \t".into())).unwrap(),
            configured
        );
        assert_eq!(
            oauth_credentials(&config, |name| Some(format!("env-{name}"))).unwrap(),
            ("env-OSU_CLIENT_ID".into(), "env-OSU_CLIENT_SECRET".into())
        );
        assert_eq!(
            oauth_credentials(&config, |name| (name == "OSU_CLIENT_ID")
                .then(|| "env-id".into()))
            .unwrap(),
            ("env-id".into(), "configured-secret".into())
        );
        assert_eq!(
            oauth_credentials(&config, |name| (name == "OSU_CLIENT_SECRET")
                .then(|| "env-secret".into()))
            .unwrap(),
            ("configured-id".into(), "env-secret".into())
        );
    }

    #[test]
    fn missing_oauth_credentials_do_not_expose_the_other_field() {
        for (id, secret) in [("private-id", ""), (" ", "private-secret")] {
            let config = crate::config::DownloadAuthConfig {
                OSU_CLIENT_ID: id.into(),
                OSU_CLIENT_SECRET: secret.into(),
            };
            let error = oauth_credentials(&config, |_| None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("download.auth"));
            assert!(!error.contains("private-"));
        }
    }
}
