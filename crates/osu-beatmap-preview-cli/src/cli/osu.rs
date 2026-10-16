//! 普通 osu 输入的命令行参数。

use crate::RenderRequest;

#[derive(Debug, Default)]
pub(super) struct Arguments {
    pub(super) bid: Option<String>,
    pub(super) osu_file: Option<String>,
    pub(super) osz_file: Option<String>,
    pub(super) convert: Option<String>,
    pub(super) mods: Vec<String>,
}

impl Arguments {
    /// 把 CLI 解析结果写入请求；OSR 仍复用 BID 作为玩家查询的谱面参数。
    pub(super) fn apply(self, request: &mut RenderRequest) {
        request.source.bid = self.bid.unwrap_or_default();
        request.source.osu_file = self.osu_file;
        request.source.osz_file = self.osz_file;
        request.ruleset.convert = self.convert;
        request.ruleset.mods = self.mods;
    }
}
