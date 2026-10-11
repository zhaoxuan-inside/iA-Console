//! 五段签名支撑类型（_06 internal 第 3 节）：Context / Metrics / Evolution。
//! 设计动机见 method-spec：观测统一走 metrics 门面，业务方法禁止手写埋点。

use crate::config::Config;
pub use crate::ports::{AiPort, ProcessPort, StorePort};

/// 会话与追踪上下文（dispatch 边界注入）。
#[derive(Debug, Clone, Default)]
pub struct SessionCtx {
    pub session_id: String,
    pub request_id: String,
    pub actor: Actor,
}

/// 发起方（审计 actor 字段同构）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Actor {
    #[default]
    Human,
    /// 外部 Agent：`agent:<client>`。
    Agent(String),
    System,
}

impl Actor {
    pub fn as_tag(&self) -> String {
        match self {
            Actor::Human => "human".into(),
            Actor::Agent(c) => format!("agent:{c}"),
            Actor::System => "system".into(),
        }
    }
}

/// 语言与时区等环境项。
#[derive(Debug, Clone, Default)]
pub struct Locale {
    pub lang: String,
    pub timezone: String,
}

/// infra Adapter 句柄集合：随任务卡逐步挂载（store/ai/process/config…）。
/// store 以 Box 持有，使 Context 可按共享引用贯穿整条调用链。
pub struct Deps {
    pub store: Box<dyn StorePort>,
    pub ai: Box<dyn AiPort>,
    pub process: Box<dyn ProcessPort>,
}

impl Default for Deps {
    fn default() -> Self {
        Self {
            store: Box::new(crate::ports::NullStore),
            ai: Box::new(crate::ports::NullAi),
            process: Box::new(crate::ports::NullProcess),
        }
    }
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps").field("store", &"Box<dyn StorePort>").finish()
    }
}

/// 运行环境（方法内只读）。五段签名第 3 段。
pub struct Context<'a> {
    pub session: &'a SessionCtx,
    pub config: &'a Config,
    pub deps: &'a Deps,
    pub locale: &'a Locale,
}

/// 系统观测门面：框架层统一收集计时与日志；业务方法只写业务指标。
#[derive(Debug, Default)]
pub struct Metrics {
    events: Vec<MetricEvent>,
}

/// 一条观测记录（测试可断言；运行时后端为 tracing）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricEvent {
    Counter { name: String, n: u64 },
    Mark { key: String, value: String },
    Log { level: Level, msg: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Metrics {
    /// 业务计数（词典约定：业务方法只写业务指标）。
    pub fn counter(&mut self, name: &str, n: u64) {
        self.events.push(MetricEvent::Counter { name: name.into(), n });
    }

    /// 自定义标记（缓存命中、降级触发等）。
    pub fn mark(&mut self, key: &str, value: impl Into<String>) {
        self.events.push(MetricEvent::Mark { key: key.into(), value: value.into() });
    }

    /// 统一日志收集（禁止业务代码 printf/echo）。
    pub fn log(&mut self, level: Level, msg: impl Into<String>) {
        self.events.push(MetricEvent::Log { level, msg: msg.into() });
    }

    /// 测试与后端导出用：只读快照。
    pub fn events(&self) -> &[MetricEvent] {
        &self.events
    }
}

/// 反馈与优化：方法结束前回填（false 时必须携带原因）。
#[derive(Debug, Clone, Default)]
pub struct Evolution {
    pub ok: bool,
    pub reason_code: Option<String>,
    pub reason: Option<String>,
    pub retryable: bool,
    pub suggestions: Vec<String>,
    pub cause: Vec<CauseLink>,
}

/// 原因链节点：记录"哪一步、什么原因"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CauseLink {
    pub step: String,
    pub reason_code: Option<String>,
    pub reason: String,
}

impl Evolution {
    /// 方法失败回填（返回值与 evolution.ok 保持一致的约定由调用方维护）。
    pub fn fail(&mut self, reason_code: &str, reason: impl Into<String>) {
        self.ok = false;
        self.reason_code = Some(reason_code.into());
        self.reason = Some(reason.into());
    }

    /// 短路传播：以当前 reason_code 为因，追加"哪一步失败"进 cause 链。
    pub fn propagate(&mut self, step: &str, reason: impl Into<String>) {
        let code = self.reason_code.clone();
        self.push_cause(step, code.as_deref(), reason);
    }

    /// orchestration 短路前追加失败步骤（子方法原因已写入，这里补"哪一步"）。
    pub fn push_cause(&mut self, step: &str, reason_code: Option<&str>, reason: impl Into<String>) {
        self.cause.push(CauseLink {
            step: step.into(),
            reason_code: reason_code.map(str::to_string),
            reason: reason.into(),
        });
    }

    /// 入口重置：约定每个五段方法开头调用，保证 evolution.ok 与返回值一致。
    pub fn succeed(&mut self) {
        self.ok = true;
        self.reason_code = None;
        self.reason = None;
        self.retryable = false;
    }
}

/// 测试夹具：组装一套缺省环境供单测构造 Context。
#[derive(Debug, Default)]
#[doc(hidden)]
pub struct TestEnv {
    pub session: SessionCtx,
    pub config: Config,
    pub deps: Deps,
    pub locale: Locale,
}

impl TestEnv {
    pub fn new() -> Self {
        Self {
            session: SessionCtx {
                session_id: "sess-test".into(),
                request_id: "req-test".into(),
                actor: Actor::Human,
            },
            config: Config::defaults(),
            deps: Deps::default(),
            locale: Locale { lang: "zh-CN".into(), timezone: "Asia/Shanghai".into() },
        }
    }

    pub fn context(&self) -> Context<'_> {
        Context {
            session: &self.session,
            config: &self.config,
            deps: &self.deps,
            locale: &self.locale,
        }
    }

    /// 换装真实 Adapter（集成测试用）：`TestEnv::new().with_store(Box::new(Storage::open_in_memory()?))`。
    pub fn with_store(mut self, store: Box<dyn StorePort>) -> Self {
        self.deps.store = store;
        self
    }

    /// 换装 AI Adapter。
    pub fn with_ai(mut self, ai: Box<dyn AiPort>) -> Self {
        self.deps.ai = ai;
        self
    }

    /// 换装进程 Adapter。
    pub fn with_process(mut self, process: Box<dyn ProcessPort>) -> Self {
        self.deps.process = process;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_records_business_events() {
        let mut m = Metrics::default();
        m.counter("config.loaded", 1);
        m.mark("hit", "knowledge");
        m.log(Level::Warn, "degraded");
        assert_eq!(m.events().len(), 3);
    }

    #[test]
    fn evolution_fail_then_cause_chain() {
        let mut e = Evolution::default();
        e.succeed();
        assert!(e.ok);
        e.fail("IAC-5101", "bad key");
        assert!(!e.ok);
        assert_eq!(e.reason_code.as_deref(), Some("IAC-5101"));
        e.push_cause("load_config", Some("IAC-5101"), "top");
        assert_eq!(e.cause.len(), 1);
    }

    #[test]
    fn actor_tag_matches_audit_format() {
        assert_eq!(Actor::Human.as_tag(), "human");
        assert_eq!(Actor::Agent("zcode".into()).as_tag(), "agent:zcode");
    }
}
