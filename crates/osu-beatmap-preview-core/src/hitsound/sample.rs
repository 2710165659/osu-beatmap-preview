//! 打击音样本：PCM 数据、样本库与名字解析。
//!
//! 样本 PCM 由宿主提供（CLI 解码内嵌音频、Web 宿主用浏览器解码），core 只负责
//! 按名字查找，因此这里不接触文件系统、网络或音频设备。

use std::collections::HashMap;

/// 样本的声道布局。
#[derive(Debug, Clone, PartialEq)]
pub enum Channels {
    /// 单声道采样，左右声道共用。
    Mono(Vec<f32>),
    /// 交错存储的左右声道采样。
    Stereo(Vec<f32>),
}

/// 一段已解码的打击音样本。
#[derive(Debug, Clone, PartialEq)]
pub struct SampleData {
    /// 单声道样本或交错双声道样本。
    pub channels: Channels,
    pub sample_rate: u32,
    /// 循环音（`*-sliderslide`）在样本内的采样帧数；0 表示不循环。
    pub loop_len: usize,
}

impl SampleData {
    pub fn mono(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            channels: Channels::Mono(samples),
            sample_rate,
            loop_len: 0,
        }
    }

    pub fn stereo(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            channels: Channels::Stereo(samples),
            sample_rate,
            loop_len: 0,
        }
    }

    pub fn with_loop(mut self, loop_len: usize) -> Self {
        self.loop_len = loop_len;
        self
    }

    /// 采样帧数（双声道交错数据为长度的一半）。
    pub fn frames(&self) -> usize {
        match &self.channels {
            Channels::Mono(samples) => samples.len(),
            Channels::Stereo(samples) => samples.len() / 2,
        }
    }

    /// 采样帧 `frame` 的左右声道；越界返回静音。
    pub fn frame(&self, frame: usize) -> (f32, f32) {
        match &self.channels {
            Channels::Mono(samples) => {
                let value = samples.get(frame).copied().unwrap_or(0.0);
                (value, value)
            }
            Channels::Stereo(samples) => {
                let left = samples.get(frame * 2).copied().unwrap_or(0.0);
                let right = samples.get(frame * 2 + 1).copied().unwrap_or(0.0);
                (left, right)
            }
        }
    }

    /// 采样帧 `position`（可为小数）的左右声道，按相邻帧线性插值。
    ///
    /// 混音器的输出采样率通常与样本采样率不同（内嵌样本是 44.1kHz，MP4 导出是 48kHz），
    /// 取最近帧（零阶保持）会把高频镜像当成有效信号，鼓声听起来发毛、发刺；音乐路径本来
    /// 就做线性插值，osu!（BASS）也会重采样，这里保持一致。
    pub fn frame_at(&self, position: f64) -> (f32, f32) {
        let base = position.floor();
        if !base.is_finite() || base < 0.0 {
            return (0.0, 0.0);
        }
        let index = base as usize;
        let fraction = (position - base) as f32;
        let first = self.frame(index);
        if fraction <= 0.0 {
            return first;
        }
        // 越界时 `frame` 返回静音，插值自然变成淡出。
        let second = self.frame(index + 1);
        (
            first.0 + (second.0 - first.0) * fraction,
            first.1 + (second.1 - first.1) * fraction,
        )
    }
}

/// 已解码的打击音样本库。
///
/// 缺失的样本名不会出现在索引中，事件生成阶段直接跳过，因此「资源缺失」或
/// 「音频文件无法解码」都按静音处理，不会中断渲染。
#[derive(Debug, Clone, Default)]
pub struct SampleLibrary {
    pub(crate) sources: Vec<SampleData>,
    index: HashMap<String, usize>,
}

impl SampleLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一段已解码的样本；同名后写入的覆盖前一个。
    pub fn insert(&mut self, name: impl Into<String>, data: SampleData) {
        let name = name.into();
        match self.index.get(&name) {
            Some(&existing) => self.sources[existing] = data,
            None => {
                let id = self.sources.len();
                self.sources.push(data);
                self.index.insert(name, id);
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&SampleData> {
        self.index.get(name).map(|&id| &self.sources[id])
    }

    /// 按名字取回样本 id；名字不在库里时返回 `None`。
    ///
    /// 与 [`SampleLibrary::name_of`] 互为反向查询，供宿主把「按名字触发」的调用
    /// 落到具体样本上（见 [`HitsoundMixer::trigger`](super::HitsoundMixer::trigger)）。
    pub fn id_of(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// 按样本 id 取回名称。
    pub fn name_of(&self, id: usize) -> Option<&str> {
        self.index
            .iter()
            .find(|(_, &value)| value == id)
            .map(|(name, _)| name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// 候选文件名到样本的解析器。
///
/// 时间轴生成与「收集需要的样本名」共用同一套遍历逻辑：前者用样本库解析，
/// 后者用只记录名字的解析器，因此两者永远不会走偏。
///
/// 候选名以字节序列传入（事件生成走栈缓冲，不分配 `String`）；返回的是内嵌资源
/// 字节切片，因此默认实现要求调用方传入 `'static` 数据。
pub trait SampleResolver {
    /// 返回第一个可用候选的样本 id；全部缺失时返回 `None`（按静音处理）。
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize>;
}

impl SampleResolver for SampleLibrary {
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize> {
        candidates
            .filter_map(|candidate| std::str::from_utf8(candidate).ok())
            .find_map(|candidate| self.index.get(candidate).copied())
    }
}

/// 允许以 `&SampleLibrary` 形式复用同一个样本库（只读解析）。
impl SampleResolver for &SampleLibrary {
    fn resolve<'a, I: Iterator<Item = &'a [u8]>>(&mut self, candidates: I) -> Option<usize> {
        candidates
            .filter_map(|candidate| std::str::from_utf8(candidate).ok())
            .find_map(|candidate| self.index.get(candidate).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 采样帧按小数位置线性插值() {
        // 回归：混音器此前按 `position as usize` 取最近帧，44.1kHz 样本进 48kHz 输出会
        // 引入混叠（鼓声发毛）；音乐路径本来就是线性插值，这里保持一致。
        let sample = SampleData::mono(vec![0.0, 1.0, 0.0, -1.0], 1000);
        assert_eq!(sample.frame_at(0.0), (0.0, 0.0));
        assert_eq!(sample.frame_at(1.0), (1.0, 1.0));
        let (value, _) = sample.frame_at(0.5);
        assert!((value - 0.5).abs() < 1e-6, "value={value}");
        let (value, _) = sample.frame_at(2.75);
        assert!((value + 0.75).abs() < 1e-6, "value={value}");
        // 末帧之后插值到静音，不会 panic。
        let (value, _) = sample.frame_at(3.5);
        assert!((value + 0.5).abs() < 1e-6, "value={value}");
        // 非法位置按静音处理。
        assert_eq!(sample.frame_at(-0.5), (0.0, 0.0));
        assert_eq!(sample.frame_at(f64::NAN), (0.0, 0.0));
    }
}
