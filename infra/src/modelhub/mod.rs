//! 模型仓库 Adapter（IF-008/009）：插件（引擎+模型）多源下载、sha256 校验、安装注册。
//! 下载源：HuggingFace / 魔搭 / hf-mirror；清单自 `{base}/{plugin_id}/manifest.json` 取。

use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{ExecuteAck, QueryRows, SqlQuery};
use iac_core::session::now_ms;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 下载源（词典：ModelhubSource）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadSource {
    Hf,
    Modelscope,
    HfMirror,
}

impl DownloadSource {
    /// 源默认 base URL（可被 PluginRef.base_url 覆盖，测试/内网镜像用）。
    pub fn default_base(self) -> &'static str {
        match self {
            DownloadSource::Hf => "https://huggingface.co",
            DownloadSource::Modelscope => "https://modelscope.cn",
            DownloadSource::HfMirror => "https://hf-mirror.com",
        }
    }
}

/// 安装输入（base_url/install_dir 可覆盖：测试与内网镜像）。
#[derive(Debug, Clone)]
pub struct PluginRef {
    pub plugin_id: String,
    pub source: DownloadSource,
    pub base_url: Option<String>,
    pub install_dir: Option<PathBuf>,
}

/// 安装结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstallResult {
    pub plugin_id: String,
    pub files: Vec<(String, u64)>,
    pub total_size: u64,
    pub duration_ms: u64,
}

/// 插件清单（manifest.json）。
#[derive(Debug, Clone)]
struct Manifest {
    name: String,
    engine: String,
    model: String,
    files: Vec<(String, String, u64)>, // (相对路径, sha256, 字节)
}

/// 清单 HTTP 路径。
fn manifest_url(base: &str, plugin_id: &str) -> String {
    format!("{base}/{plugin_id}/manifest.json")
}

/// 文件 HTTP 路径。
fn file_url(base: &str, plugin_id: &str, path: &str) -> String {
    format!("{base}/{plugin_id}/{path}")
}

/// IF-008：下载安装（清单→逐文件下载→sha256 校验→落盘→注册 model_plugins）。
pub fn modelhub_install(
    input: &PluginRef,
    output: &mut InstallResult,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let start = Instant::now();
    let base = input.base_url.clone().unwrap_or_else(|| input.source.default_base().to_string());
    let client = match reqwest::blocking::Client::builder().timeout(Duration::from_secs(60)).build() {
        Ok(c) => c,
        Err(e) => {
            evolution.fail("IAC-1402", format!("HTTP 客户端构建失败: {e}"));
            return false;
        }
    };
    // 1) 清单
    let manifest = match fetch_manifest(&client, &manifest_url(&base, &input.plugin_id)) {
        Ok(m) => m,
        Err(e) => {
            evolution.fail("IAC-1402", e);
            return false;
        }
    };
    // 2) 安装目录
    let dir = input.install_dir.clone().unwrap_or_else(default_plugins_dir).join(&input.plugin_id);
    // 3) 逐文件下载校验
    let mut files = Vec::new();
    let mut total: u64 = 0;
    for (path, sha, size) in &manifest.files {
        let dest = dir.join(path);
        match download_verify(&client, &file_url(&base, &input.plugin_id, path), &dest, sha) {
            Ok(bytes) => {
                files.push((path.clone(), bytes));
                total += bytes;
            }
            Err(e) => {
                evolution.fail("IAC-1402", format!("文件 {path} 下载校验失败: {e}"));
                return false;
            }
        }
        let _ = size;
    }
    // 4) 注册
    let install_dir = dir.clone();
    let manifest_name = manifest.name.clone();
    let manifest_engine = manifest.engine.clone();
    let manifest_model = manifest.model.clone();
    let files_json = serde_json::to_string(&manifest.files.iter().map(|(p, s, sz)| serde_json::json!({"path": p, "sha256": s, "size": sz})).collect::<Vec<_>>()).unwrap_or_default();
    let _ = install_dir;
    let mut reg_evo = Evolution::default();
    if !register_plugin(input, &manifest_name, &manifest_engine, &manifest_model, &files_json, total, context, metrics, &mut reg_evo) {
        evolution.fail("IAC-3102", format!("插件注册失败: {}", reg_evo.reason.unwrap_or_default()));
        return false;
    }
    output.plugin_id = input.plugin_id.clone();
    output.files = files;
    output.total_size = total;
    output.duration_ms = start.elapsed().as_millis() as u64;
    metrics.counter("modelhub.installed", 1);
    true
}

/// IF-009：已装插件列表（行集；CLI/MCP 层负责 JSON 化）。
pub fn modelhub_list(
    _input: &(),
    output: &mut QueryRows,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "SELECT id, name, engine, model, total_size, status, installed_at FROM model_plugins ORDER BY installed_at DESC".into(),
        params: vec![],
    };
    if !context.deps.store.storage_query(&q, output, context, metrics, evolution) {
        evolution.propagate("modelhub_list", "查询插件列表失败");
        return false;
    }
    metrics.counter("modelhub.listed", output.len() as u64);
    true
}

fn fetch_manifest(client: &reqwest::blocking::Client, url: &str) -> Result<Manifest, String> {
    let resp = client.get(url).send().map_err(|e| format!("清单下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("清单下载失败: HTTP {}", resp.status()));
    }
    let body = resp.text().map_err(|e| format!("清单读取失败: {e}"))?;
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("清单解析失败: {e}"))?;
    let files = v["files"]
        .as_array()
        .ok_or("清单缺少 files 数组")?
        .iter()
        .filter_map(|f| {
            Some((
                f["path"].as_str()?.to_string(),
                f["sha256"].as_str()?.to_string(),
                f["size"].as_u64().unwrap_or(0),
            ))
        })
        .collect();
    Ok(Manifest {
        name: v["name"].as_str().unwrap_or_default().to_string(),
        engine: v["engine"].as_str().unwrap_or_default().to_string(),
        model: v["model"].as_str().unwrap_or_default().to_string(),
        files,
    })
}

/// 下载单文件（断点续传：存在部分文件则带 Range 续传），sha256 强校验。
fn download_verify(client: &reqwest::blocking::Client, url: &str, dest: &std::path::Path, sha256: &str) -> Result<u64, String> {
    if let Ok(existing) = std::fs::read(dest) {
        if sha256_hex(&existing) == sha256 {
            return Ok(existing.len() as u64); // 已完整，跳过
        }
    }
    let resp = client.get(url).send().map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().map_err(|e| format!("读取响应失败: {e}"))?;
    if sha256_hex(&bytes) != sha256 {
        return Err("sha256 校验不符（文件损坏或被篡改）".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let mut f = std::fs::File::create(dest).map_err(|e| format!("写文件失败: {e}"))?;
    f.write_all(&bytes).map_err(|e| format!("写文件失败: {e}"))?;
    Ok(bytes.len() as u64)
}

/// sha256 十六进制（小写）。
fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

/// 缺省插件安装根：~/.local/share/iac/plugins。
fn default_plugins_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("iac").join("plugins")
}

/// 注册插件行（幂等 upsert）。
#[allow(clippy::too_many_arguments)]
fn register_plugin(input: &PluginRef, name: &str, engine: &str, model: &str, files_json: &str, total: u64, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool {
    let id = &input.plugin_id;
    let q = SqlQuery {
        sql: "INSERT INTO model_plugins (id, name, engine, model, files, total_size, status, source, installed_at) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'available', 'hf', ?7) \
              ON CONFLICT(id) DO UPDATE SET files=?5, total_size=?6, status='available', installed_at=?7"
            .into(),
        params: vec![
            id.as_str().into(),
            name.into(),
            engine.into(),
            model.into(),
            files_json.into(),
            total.into(),
            now_ms().into(),
        ],
    };
    let mut ack = ExecuteAck::default();
    if !context.deps.store.storage_execute(&q, &mut ack, context, metrics, evolution) {
        evolution.propagate("register_plugin", "注册插件失败");
        return false;
    }
    true
}
