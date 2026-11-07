//! 回放来源参数组合；只解析选择，不读取文件或请求网络。
use super::CliError;
use crate::{ReplayMode, ReplaySource, RequestKind, ScoreSelection};

#[derive(Default)]
pub(super) struct Arguments {
    pub file: Option<String>,
    pub score: Option<String>,
    pub user: Option<String>,
    pub selection: Option<String>,
    pub mode: Option<String>,
    pub include_fails: bool,
}
impl Arguments {
    pub fn kind(self, input_type: &str, bid: &str) -> Result<RequestKind, CliError> {
        let player = self.user.is_some();
        let count = usize::from(self.file.is_some())
            + usize::from(self.score.is_some())
            + usize::from(player);
        if matches!(input_type, "" | "osu") {
            if count != 0 || self.selection.is_some() || self.mode.is_some() || self.include_fails {
                return Err(CliError("回放来源参数需要 --type=osr".into()));
            }
            return Ok(RequestKind::Osu);
        }
        if input_type != "osr" {
            return Err(CliError("--type must be osu or osr".into()));
        }
        if count == 0 {
            return Err(CliError(
                "--type=osr 缺少回放来源：请指定 --osr-file、--score-id 或 --user".into(),
            ));
        }
        if count > 1 {
            return Err(CliError(
                "--osr-file、--score-id、--user 互斥，只能指定一种回放来源".into(),
            ));
        }
        if !player {
            for (present, flag) in [
                (self.selection.is_some(), "--score-select"),
                (self.mode.is_some(), "--mode"),
                (self.include_fails, "--include-fails"),
            ] {
                if present {
                    return Err(CliError(format!("{flag} 仅用于 --user 玩家查询")));
                }
            }
        }
        // 指定谱面保持原有最高分默认值；未指定谱面时定位最近一次游玩。
        let default_selection = if bid.is_empty() { "recent" } else { "best" };
        let selection = match self.selection.as_deref().unwrap_or(default_selection) {
            "best" => ScoreSelection::Best,
            "recent" => ScoreSelection::Recent,
            _ => return Err(CliError("--score-select must be best or recent".into())),
        };
        let mode = match self.mode.as_deref() {
            None => None,
            Some("osu" | "std" | "standard") => Some(ReplayMode::Osu),
            Some("taiko") => Some(ReplayMode::Taiko),
            Some("fruits" | "fruit" | "ctb" | "catch") => Some(ReplayMode::Fruits),
            Some("mania") => Some(ReplayMode::Mania),
            _ => {
                return Err(CliError(
                    "--mode 必须为 osu（std、standard）、taiko、fruits（fruit、ctb、catch）或 mania"
                        .into(),
                ));
            }
        };
        let source = if let Some(file) = self.file {
            ReplaySource::File(file)
        } else if let Some(score) = self.score {
            ReplaySource::Score(score)
        } else {
            ReplaySource::Player {
                user: self.user.unwrap(),
                selection,
                mode,
                include_fails: self.include_fails,
            }
        };
        Ok(RequestKind::Osr { source })
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::{parse_args, CliAction};
    use crate::{ReplayMode, ReplaySource, RequestKind, ScoreSelection};

    #[test]
    fn sources_are_exclusive_and_flags_cannot_be_silently_ignored() {
        for args in [
            vec!["--type=osr"],
            vec!["--type=osr", "--osr-file=a.osr", "--score-id=1"],
            vec!["--type=osr", "--score-id=1", "--user=a", "--bid=2"],
            vec!["--type=osr", "--user=a", "--user-id=1", "--bid=2"],
            vec!["--type=osr", "--user=a", "--score-select=best"],
            vec!["--type=osr", "--score-id=1", "--score-select=best"],
            vec!["--type=osr", "--osr-file=a.osr", "--bid=2"],
            vec!["--type=osr", "--score-id=0"],
            vec!["--type=osr", "--user=", "--bid=2"],
            vec!["--score-id=1", "--bid=2"],
            vec!["--type=osr", "--user=a", "--bid=2", "--score-select=pp"],
            vec!["--type=osr", "--user-id=1", "--bid=2"],
            vec!["--type=osr", "--user=a", "--mode=invalid"],
            vec!["--type=osr", "--user=a", "--bid=0"],
            vec!["--type=osr", "--user=a", "--bid=2", "--include-fails"],
            vec!["--type=osr", "--score-id=1", "--mode=osu"],
            vec!["--type=osr", "--osr-file=a.osr", "--include-fails"],
            vec!["--bid=2", "--mode=osu"],
            vec!["--bid=2", "--include-fails"],
        ] {
            assert!(parse_args(args.clone()).is_err(), "{args:?}");
        }
    }

    #[test]
    fn errors_identify_the_invalid_parameter_or_source_combination() {
        for (args, expected) in [
            (vec!["--type=osr"], "缺少回放来源"),
            (
                vec!["--type=osr", "--osr-file=a.osr", "--score-id=1"],
                "互斥",
            ),
            (
                vec!["--type=osr", "--score-id=1", "--mode=osu"],
                "--mode 仅用于 --user",
            ),
            (
                vec!["--type=osr", "--score-id=1", "--score-select=best"],
                "--score-select 仅用于 --user",
            ),
            (
                vec!["--type=osr", "--osr-file=a.osr", "--include-fails"],
                "--include-fails 仅用于 --user",
            ),
            (
                vec!["--type=osr", "--osr-file=a.osr", "--bid=2"],
                "--bid 仅用于 --user",
            ),
            (
                vec!["--type=osr", "--score-id=1", "--convert=taiko"],
                "不接受 --convert",
            ),
            (
                vec!["--type=osr", "--score-id=1", "--mod=NM"],
                "不接受 --mod",
            ),
            (
                vec!["--type=osr", "--user=a", "--score-select=best"],
                "--score-select=best 需要 --bid",
            ),
            (
                vec!["--type=osr", "--user=a", "--bid=2", "--include-fails"],
                "--include-fails 仅用于最近成绩查询",
            ),
        ] {
            let error = parse_args(args.clone()).unwrap_err().to_string();
            assert!(error.contains(expected), "{args:?}: {error}");
        }
    }

    #[test]
    fn username_errors_distinguish_empty_names_from_byte_limit() {
        for (name, expected) in [
            (" ".into(), "非空"),
            ("a".repeat(257), "不能超过 256 字节"),
            ("玩".repeat(86), "不能超过 256 字节"),
        ] {
            let error = parse_args(["--type=osr", &format!("--user={name}")])
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
        for name in ["a".repeat(256), "玩".repeat(85)] {
            assert!(parse_args(["--type=osr", &format!("--user={name}")]).is_ok());
        }
    }

    #[test]
    fn player_defaults_to_best_and_accepts_recent() {
        let CliAction::Render(request) =
            parse_args(["--type=osr", "--user=123", "--bid=2"]).unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            request.kind,
            RequestKind::Osr {
                source: ReplaySource::Player {
                    selection: ScoreSelection::Best,
                    include_fails: false,
                    ..
                }
            }
        ));
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
    }

    #[test]
    fn latest_player_score_has_optional_official_mode_and_opt_in_fails() {
        let CliAction::Render(request) = parse_args(["--type=osr", "--user=123"]).unwrap() else {
            panic!()
        };
        assert!(request.source.bid.is_empty());
        assert!(matches!(request.kind, RequestKind::Osr {
            source: ReplaySource::Player { user, selection: ScoreSelection::Recent, mode: None, include_fails: false }
        } if user == "123"));
        for (name, expected) in [
            ("osu", ReplayMode::Osu),
            ("std", ReplayMode::Osu),
            ("standard", ReplayMode::Osu),
            ("taiko", ReplayMode::Taiko),
            ("fruits", ReplayMode::Fruits),
            ("fruit", ReplayMode::Fruits),
            ("ctb", ReplayMode::Fruits),
            ("catch", ReplayMode::Fruits),
            ("mania", ReplayMode::Mania),
        ] {
            let CliAction::Render(request) = parse_args([
                "--type=osr",
                "--user=a",
                &format!("--mode={name}"),
                "--include-fails",
            ])
            .unwrap() else {
                panic!()
            };
            assert!(matches!(request.kind, RequestKind::Osr {
                source: ReplaySource::Player { selection: ScoreSelection::Recent, mode: Some(mode), include_fails: true, .. }
            } if mode == expected));
        }
        assert!(parse_args([
            "--type=osr",
            "--user=a",
            "--bid=2",
            "--score-select=recent",
            "--include-fails"
        ])
        .is_ok());
    }
}
