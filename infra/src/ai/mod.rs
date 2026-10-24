//! AI 后端适配（ADR-006）：OpenAI 兼容 / Anthropic / 本地引擎统一转译 LlmResponse。
use iac_core::config::ProviderProfile;
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{AiPort, AiRequest, LlmResponse};

/// AI 适配器（实现由重建阶段填充）。
pub struct AiAdapter;

impl AiPort for AiAdapter {
    fn ai_complete(
        &self,
        _provider: &ProviderProfile,
        _input: &AiRequest,
        _output: &mut LlmResponse,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1101", "AI 重建中（ai_complete 未接线）");
        false
    }
}
