//! DeepSeek provider — the sole backend for Aili.

pub const BASE_URL: &str = "https://api.deepseek.com/v1";
pub const API_KEY_ENV: &str = "DEEPSEEK_API_KEY";
pub const CONTEXT_WINDOW: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeepSeekModel {
    pub id: &'static str,
    pub name: &'static str,
    pub context_window: usize,
}

pub const V4_FLASH: DeepSeekModel = DeepSeekModel {
    id: "deepseek-v4-flash",
    name: "DeepSeek V4 Flash",
    context_window: CONTEXT_WINDOW,
};

pub const V4_PRO: DeepSeekModel = DeepSeekModel {
    id: "deepseek-v4-pro",
    name: "DeepSeek V4 Pro",
    context_window: CONTEXT_WINDOW,
};

pub fn model_info(model_id: &str) -> &'static DeepSeekModel {
    if model_id.contains("pro") {
        &V4_PRO
    } else {
        &V4_FLASH
    }
}

pub const MODEL_PRESETS: &[DeepSeekModel] = &[V4_FLASH, V4_PRO];
pub const DEFAULT_MODEL: &DeepSeekModel = &V4_FLASH;
