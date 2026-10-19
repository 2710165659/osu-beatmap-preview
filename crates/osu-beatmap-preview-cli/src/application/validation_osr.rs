//! 回放参数互斥校验；任何文件读取之前执行，CLI 与库调用共用。
use super::request::{RenderRequest, RequestKind};
use osu_beatmap_preview_core::{PreviewError, Result};

pub(super) fn validate(request: &RenderRequest) -> Result<()> {
    let RequestKind::Osr { source } = &request.kind else {
        return Ok(());
    };
    match source {
        crate::ReplaySource::File(path) => super::source_osu::validate_local_path(path, "osr")?,
        crate::ReplaySource::Score(id) => positive_id(id, "--score-id")?,
        crate::ReplaySource::Player {
            user,
            selection,
            include_fails,
            ..
        } => {
            if user.trim().is_empty() || user.len() > 256 {
                return Err(PreviewError::new("--user 需要非空的玩家名"));
            }
            if !request.source.bid.is_empty() {
                positive_id(&request.source.bid, "--bid")?;
            } else if *selection == crate::ScoreSelection::Best {
                return Err(PreviewError::new("--score-select=best 需要 --bid"));
            }
            if *include_fails && *selection != crate::ScoreSelection::Recent {
                return Err(PreviewError::new(
                    "--include-fails 仅用于最近成绩查询（--score-select=recent）",
                ));
            }
        }
    }
    if (!matches!(source, crate::ReplaySource::Player { .. }) && !request.source.bid.is_empty())
        || request.ruleset.convert.is_some()
        || !request.ruleset.mods.is_empty()
    {
        return Err(PreviewError::new(
            "OSR rejects --convert and user --mod (including NM); --bid 仅用于玩家查询",
        ));
    }
    for (file, extension) in [
        (&request.source.osu_file, "osu"),
        (&request.source.osz_file, "osz"),
    ] {
        if let Some(file) = file {
            super::source_osu::validate_local_path(file, extension)?;
        }
    }
    Ok(())
}

fn positive_id(id: &str, flag: &str) -> Result<()> {
    if id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || id.parse::<u64>().ok().is_none_or(|v| v == 0)
    {
        return Err(PreviewError::new(format!("{flag} 需要正整数 ID")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_conflicting_library_requests_including_nm() {
        let mut request = RenderRequest::new("");
        request.kind = RequestKind::Osr {
            source: crate::ReplaySource::File("a.osr".into()),
        };
        request.clone().validate().unwrap();
        request.source.osu_file = Some("a.osu".into());
        request.output.format = Some("mp4".into());
        request.clone().validate().unwrap();
        request.ruleset.mods.push("NM".into());
        assert!(request.clone().validate().is_err());
        request.ruleset.mods.clear();
        request.ruleset.convert = Some("standard".into());
        assert!(request.clone().validate().is_err());
        request.ruleset.convert = None;
        request.output.format = Some("png".into());
        request.clone().validate().unwrap();
        request.output.format = Some("invalid".into());
        assert!(request.validate().is_err());
    }

    #[test]
    fn player_library_requests_share_recent_selection_constraints() {
        let make_request = |bid: &str, selection, include_fails| {
            let mut request = RenderRequest::new(bid);
            request.kind = RequestKind::Osr {
                source: crate::ReplaySource::Player {
                    user: "玩家名".into(),
                    selection,
                    mode: None,
                    include_fails,
                },
            };
            request
        };
        use crate::ScoreSelection::{Best, Recent};
        assert!(make_request("", Best, false).validate().is_err());
        assert!(make_request("2", Best, true).validate().is_err());
        assert!(make_request("", Recent, false).validate().is_ok());
        assert!(make_request("", Recent, true).validate().is_ok());
        assert!(make_request("2", Recent, true).validate().is_ok());
    }
}
