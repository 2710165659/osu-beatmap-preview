//! core 的类型化绘制配置。
//!
//! YAML/JSON 文件的读取、合并和校验由平台层负责；core 只接收已经反序列化的
//! `CoreConfig`。内嵌默认值仅用于没有显式配置的调用方。

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

include!("generated_config.rs");

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
    pub render: RenderConfig,
    pub skin: SkinConfig,
}

static DEFAULT_CONFIG: OnceLock<Arc<CoreConfig>> = OnceLock::new();

thread_local! {
    static SCOPED_CONFIGS: RefCell<Vec<Arc<CoreConfig>>> = const { RefCell::new(Vec::new()) };
}

impl Default for CoreConfig {
    fn default() -> Self {
        serde_json::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/default_config.json"
        )))
        .expect("内嵌默认 core 配置必须有效")
    }
}

/// 返回当前调用作用域的配置；没有作用域时使用内嵌默认值。
pub(crate) fn current() -> Arc<CoreConfig> {
    SCOPED_CONFIGS
        .with(|configs| configs.borrow().last().cloned())
        .unwrap_or_else(|| {
            Arc::clone(DEFAULT_CONFIG.get_or_init(|| Arc::new(CoreConfig::default())))
        })
}

/// 在指定配置下执行 core 计算。配置栈允许同一线程嵌套不同会话。
pub fn with_config<T>(config: Arc<CoreConfig>, run: impl FnOnce() -> T) -> T {
    struct ScopeGuard;
    impl Drop for ScopeGuard {
        fn drop(&mut self) {
            SCOPED_CONFIGS.with(|configs| {
                configs.borrow_mut().pop();
            });
        }
    }

    SCOPED_CONFIGS.with(|configs| configs.borrow_mut().push(config));
    let _guard = ScopeGuard;
    run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 配置作用域使用宿主配置并在退出后恢复默认值() {
        let default_scale = current().render.standard.png.SCALE;
        let mut custom = CoreConfig::default();
        custom.render.standard.png.SCALE = default_scale + 0.25;

        let scoped_scale = with_config(Arc::new(custom), || current().render.standard.png.SCALE);

        assert_eq!(scoped_scale, default_scale + 0.25);
        assert_eq!(current().render.standard.png.SCALE, default_scale);
    }
}
