//! 与输出像素尺寸无关的 Standard/Taiko/Catch 回放谱面逻辑准备。
pub mod catch;
pub mod standard;
pub mod taiko;

use crate::{
    gameplay::timeline::GameplayTimeline,
    model::Beatmap,
    replay::{mods::ReplayMods, ReplayDocument},
    Result,
};
use std::sync::Arc;

pub enum PreparedReplay {
    Standard(Arc<standard::PreparedStandard>),
    Taiko(Arc<taiko::PreparedTaiko>),
    Catch(Arc<catch::PreparedCatch>),
}

impl PreparedReplay {
    pub fn prepare(beatmap: &Beatmap, document: &ReplayDocument) -> Result<Self> {
        match crate::replay::compatibility::select_replay(document)? {
            crate::replay::compatibility::ReplayPolicy::Standard(_) => Ok(Self::Standard(
                Arc::new(standard::prepare_replay(beatmap, document)?),
            )),
            crate::replay::compatibility::ReplayPolicy::Catch(_) => Ok(Self::Catch(Arc::new(
                catch::prepare_replay(beatmap, document)?,
            ))),
            crate::replay::compatibility::ReplayPolicy::Taiko(_) => Ok(Self::Taiko(Arc::new(
                taiko::prepare_replay(beatmap, document)?,
            ))),
        }
    }
    pub fn compile(&self, document: Arc<ReplayDocument>) -> Result<GameplayTimeline> {
        match self {
            Self::Standard(p) => crate::gameplay::engine::compile_standard(p, document),
            Self::Taiko(p) => crate::gameplay::engine::compile_taiko(p, document),
            Self::Catch(p) => crate::gameplay::engine::compile_catch(p, document),
        }
    }
    pub fn mods(&self) -> &ReplayMods {
        match self {
            Self::Standard(p) => &p.mods,
            Self::Taiko(p) => &p.mods,
            Self::Catch(p) => &p.mods,
        }
    }
    pub fn first(&self) -> i64 {
        match self {
            Self::Standard(p) => p.objects[0].source.start_time,
            Self::Taiko(p) => p.objects[0].source.start_time,
            Self::Catch(p) => p.first,
        }
    }
    pub fn last_end(&self) -> f64 {
        match self {
            Self::Standard(p) => p.last_end,
            Self::Taiko(p) => p.last_end,
            Self::Catch(p) => p.last_end,
        }
    }
}
