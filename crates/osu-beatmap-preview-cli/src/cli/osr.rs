//! 回放来源参数组合；只解析选择，不读取文件或请求网络。
use super::CliError;
use crate::{ReplaySource, ReplayUser, RequestKind, ScoreSelection};

#[derive(Default)]
pub(super) struct Arguments {
    pub file: Option<String>,
    pub score: Option<String>,
    pub user: Option<String>,
    pub user_id: Option<String>,
    pub selection: Option<String>,
}
impl Arguments {
    pub fn kind(self, input_type: &str) -> Result<RequestKind, CliError> {
        let player = self.user.is_some() || self.user_id.is_some();
        let count = usize::from(self.file.is_some())
            + usize::from(self.score.is_some())
            + usize::from(player);
        if matches!(input_type, "" | "osu") {
            if count != 0 || self.selection.is_some() {
                return Err(CliError("回放来源参数需要 --type=osr".into()));
            }
            return Ok(RequestKind::Osu);
        }
        if input_type != "osr" {
            return Err(CliError("--type must be osu or osr".into()));
        }
        if count != 1 || (self.user.is_some() && self.user_id.is_some()) {
            return Err(CliError(
                "OSR 来源必须三选一：--osr-file、--score-id、--user/--user-id + --bid".into(),
            ));
        }
        if !player && self.selection.is_some() {
            return Err(CliError("--score-select 仅用于玩家查询".into()));
        }
        let selection = match self.selection.as_deref().unwrap_or("best") {
            "best" => ScoreSelection::Best,
            "recent" => ScoreSelection::Recent,
            _ => return Err(CliError("--score-select must be best or recent".into())),
        };
        let source = if let Some(file) = self.file {
            ReplaySource::File(file)
        } else if let Some(score) = self.score {
            ReplaySource::Score(score)
        } else {
            let user = match (self.user, self.user_id) {
                (Some(name), None) => ReplayUser::Name(name),
                (None, Some(id)) => ReplayUser::Id(id),
                _ => unreachable!(),
            };
            ReplaySource::Player { user, selection }
        };
        Ok(RequestKind::Osr { source })
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::{parse_args, CliAction};
    use crate::{ReplaySource, RequestKind, ScoreSelection};

    #[test]
    fn sources_are_exclusive_and_flags_cannot_be_silently_ignored() {
        for args in [
            vec!["--type=osr"],
            vec!["--type=osr", "--osr-file=a.osr", "--score-id=1"],
            vec!["--type=osr", "--score-id=1", "--user=a", "--bid=2"],
            vec!["--type=osr", "--user=a", "--user-id=1", "--bid=2"],
            vec!["--type=osr", "--user=a"],
            vec!["--type=osr", "--score-id=1", "--score-select=best"],
            vec!["--type=osr", "--osr-file=a.osr", "--bid=2"],
            vec!["--type=osr", "--score-id=0"],
            vec!["--type=osr", "--user=", "--bid=2"],
            vec!["--score-id=1", "--bid=2"],
            vec!["--type=osr", "--user-id=1", "--bid=2", "--score-select=pp"],
        ] {
            assert!(parse_args(args.clone()).is_err(), "{args:?}");
        }
    }

    #[test]
    fn player_defaults_to_best_and_accepts_recent() {
        for (flag, expected) in [
            ("best", ScoreSelection::Best),
            ("recent", ScoreSelection::Recent),
        ] {
            let CliAction::Render(request) = parse_args([
                "--type=osr",
                "--user=123",
                "--bid=2",
                &format!("--score-select={flag}"),
            ])
            .unwrap() else {
                panic!()
            };
            assert!(
                matches!(request.kind, RequestKind::Osr { source: ReplaySource::Player { selection, .. } } if selection == expected)
            );
        }
        assert!(parse_args(["--type=osr", "--score-id=6086051920"]).is_ok());
        assert!(parse_args(["--type=osr", "--user-id=1", "--bid=2"]).is_ok());
    }
}
