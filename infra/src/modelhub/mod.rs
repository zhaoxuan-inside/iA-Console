//! 模型仓库 Adapter（IF-008/009）：插件（引擎+模型）多源下载、sha256 校验、安装注册。
//! 下载源：HuggingFace / 魔搭 / hf-mirror；清单自 `{base}/{plugin_id}/manifest.json` 取。
//! 下载为流式落盘（.part 承载，支持 Range 断点续传），无总超时，连接/读分时超时防假死。

use crate::fsutil;
use crate::httpx;
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{ExecuteAck, QueryRows, SqlQuery};
use iac_core::session::now_ms;
use serde_json::Value;
use std::io::Read;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::time::Instant;

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

/// IF-008：下载安装（清单→逐文件流式下载+Range 续传→sha256 校验→落盘→注册 model_plugins）。
pub fn modelhub_install(
    input: &PluginRef,
    output: &mut InstallResult,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let start = Instant::now();
    let base = input
        .base_url
        .clone()
        .unwrap_or_else(|| input.source.default_base().to_string());
    let Some(client) = build_client(evolution) else {
        return false; // evolution 已写明 IAC-1402
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
    let dir = input
        .install_dir
        .clone()
        .unwrap_or_else(default_plugins_dir)
        .join(&input.plugin_id);
    // 3) 逐文件下载校验
    let Some((files, total)) = install_files(
        &client,
        &base,
        &input.plugin_id,
        &dir,
        &manifest,
        metrics,
        evolution,
    ) else {
        return false; // evolution 已写明 IAC-1402
    };
    // 4) 注册
    let mut reg_evo = Evolution::default();
    if !register_plugin(
        input,
        &manifest.name,
        &manifest.engine,
        &manifest.model,
        &files_json(&manifest),
        total,
        context,
        metrics,
        &mut reg_evo,
    ) {
        evolution.fail(
            "IAC-3102",
            format!("插件注册失败: {}", reg_evo.reason.unwrap_or_default()),
        );
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
        sql: "SELECT id, name, engine, model, total_size, status, source, installed_at FROM model_plugins ORDER BY installed_at DESC".into(),
        params: vec![],
    };
    if !context
        .deps
        .store
        .storage_query(&q, output, context, metrics, evolution)
    {
        evolution.propagate("modelhub_list", "查询插件列表失败");
        return false;
    }
    metrics.counter("modelhub.listed", output.len() as u64);
    true
}

/// HTTP 客户端构建（modelhub_install 子步骤）：连接/读分时超时，无总超时（大模型下载）。
fn build_client(evolution: &mut Evolution) -> Option<reqwest::blocking::Client> {
    httpx::download_client()
        .map_err(|e| evolution.fail("IAC-1402", e))
        .ok()
}

/// 清单 files → JSON 字符串（注册行用）。
fn files_json(manifest: &Manifest) -> String {
    serde_json::to_string(
        &manifest
            .files
            .iter()
            .map(|(p, s, sz)| serde_json::json!({"path": p, "sha256": s, "size": sz}))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
}

/// 逐文件下载校验（modelhub_install 子步骤）；失败返回 None。
fn install_files(
    client: &reqwest::blocking::Client,
    base: &str,
    plugin_id: &str,
    dir: &std::path::Path,
    manifest: &Manifest,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<(Vec<(String, u64)>, u64)> {
    let mut files = Vec::new();
    let mut total: u64 = 0;
    for (path, sha, _size) in &manifest.files {
        let dest = dir.join(path);
        // (落盘大小, 本次网络传输量)：total_size 记文件实际大小，metrics 记传输量
        match download_verify(client, &file_url(base, plugin_id, path), &dest, sha) {
            Ok((size, transferred)) => {
                metrics.counter("modelhub.downloaded_bytes", transferred);
                files.push((path.clone(), size));
                total += size;
            }
            Err(e) => {
                // e 透出内层原因（哈希不符/网络/HTTP 状态），否则只有笼统的"下载校验失败"
                evolution.fail("IAC-1402", format!("文件 {path} 下载校验失败: {e}"));
                return None;
            }
        }
    }
    Some((files, total))
}

/// 清单强解析：缺失字段报错（此前 filter_map 静默跳过会少装文件）；路径拒绝穿越。
fn parse_manifest(body: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("清单解析失败: {e}"))?;
    let files_v = v["files"].as_array().ok_or("清单缺少 files 数组")?;
    let mut files = Vec::with_capacity(files_v.len());
    for f in files_v {
        let path = f["path"].as_str().ok_or("清单 files[].path 缺失")?;
        let sha = f["sha256"].as_str().ok_or("清单 files[].sha256 缺失")?;
        let p = std::path::Path::new(path);
        if path.is_empty()
            || p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("清单文件路径非法: {path}"));
        }
        files.push((
            path.to_string(),
            sha.to_string(),
            f["size"].as_u64().unwrap_or(0),
        ));
    }
    Ok(Manifest {
        name: v["name"].as_str().unwrap_or_default().to_string(),
        engine: v["engine"].as_str().unwrap_or_default().to_string(),
        model: v["model"].as_str().unwrap_or_default().to_string(),
        files,
    })
}

fn fetch_manifest(client: &reqwest::blocking::Client, url: &str) -> Result<Manifest, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("清单下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("清单下载失败: HTTP {}", resp.status()));
    }
    let body = resp.text().map_err(|e| format!("清单读取失败: {e}"))?;
    parse_manifest(&body)
}

/// 下载单文件：流式落盘 + Range 断点续传 + sha256 强校验。
/// .part 承载部分内容：服务器 206 → 续传追加；200 → 从头重下；网络中断保留 .part（下次续传）；
/// 整档 sha256 不符 → 删除 .part 报错（内容不可信）。成功后原子改名落位。
/// 返回 (落盘文件大小, 本次网络传输量)。
fn download_verify(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &std::path::Path,
    sha256: &str,
) -> Result<(u64, u64), String> {
    // 已安装且校验通过 → 跳过；不符（损坏/被篡改）→ 删除重装
    if dest.exists() {
        let ok = fsutil::sha256_file(dest)
            .map(|h| h == sha256)
            .unwrap_or(false);
        if ok {
            let size = dest
                .metadata()
                .map_err(|e| format!("读取已装文件失败: {e}"))?
                .len();
            return Ok((size, 0));
        }
        let _ = std::fs::remove_file(dest);
    }
    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = dest
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("{file_name}.part"));
    // 安装目录（含 .part 所在目录）先于下载建好
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let partial: u64 = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let mut req = client.get(url);
    if partial > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={partial}-"));
    }
    let mut resp = req.send().map_err(|e| format!("下载失败: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && partial > 0 {
        // 416：.part 长度已达全量（上次中断在「校验通过→落位」窗口，或落位失败）。
        // 整档校验通过 → 直接落位；不符 → 删除 .part 重头下载（partial=0 不再触发 416）。
        if fsutil::sha256_file(&part)
            .map(|h| h == sha256)
            .unwrap_or(false)
        {
            std::fs::rename(&part, dest).map_err(|e| format!("落位失败: {e}"))?;
            return Ok((partial, 0));
        }
        let _ = std::fs::remove_file(&part);
        return download_verify(client, url, dest, sha256);
    }
    let append = if partial > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
        true // 206：续传
    } else if status.is_success() {
        false // 200（或首下）：从头写
    } else {
        return Err(format!("HTTP {status}"));
    };
    let mut f = if append {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .map_err(|e| format!("打开部分文件失败: {e}"))?
    } else {
        std::fs::File::create(&part).map_err(|e| format!("创建下载文件失败: {e}"))?
    };
    let mut size: u64 = if append { partial } else { 0 };
    let mut transferred: u64 = 0;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        match resp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                f.write_all(&buf[..n])
                    .map_err(|e| format!("写盘失败: {e}"))?;
                size += n as u64;
                transferred += n as u64;
            }
            Err(e) => return Err(format!("下载中断: {e}")), // .part 保留，下次续传
        }
    }
    if fsutil::sha256_file(&part).map_err(|e| format!("校验读取失败: {e}"))? != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("sha256 校验不符（文件损坏或被篡改）".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    std::fs::rename(&part, dest).map_err(|e| format!("落位失败: {e}"))?;
    Ok((size, transferred))
}

/// 缺省插件安装根：~/.local/share/iac/plugins。
fn default_plugins_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("iac")
        .join("plugins")
}

/// 下载源 → 注册表 source 标签。
fn source_tag(s: DownloadSource) -> &'static str {
    match s {
        DownloadSource::Hf => "hf",
        DownloadSource::Modelscope => "modelscope",
        DownloadSource::HfMirror => "hf-mirror",
    }
}

/// 注册插件行（幂等 upsert）。
#[allow(clippy::too_many_arguments)]
fn register_plugin(
    input: &PluginRef,
    name: &str,
    engine: &str,
    model: &str,
    files_json: &str,
    total: u64,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let id = &input.plugin_id;
    let source = source_tag(input.source);
    let q = SqlQuery {
        sql: "INSERT INTO model_plugins (id, name, engine, model, files, total_size, status, source, installed_at) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'available', ?7, ?8) \
              ON CONFLICT(id) DO UPDATE SET files=?5, total_size=?6, status='available', source=?7, installed_at=?8"
            .into(),
        params: vec![
            id.as_str().into(),
            name.into(),
            engine.into(),
            model.into(),
            files_json.into(),
            total.into(),
            source.into(),
            now_ms().into(),
        ],
    };
    let mut ack = ExecuteAck::default();
    if !context
        .deps
        .store
        .storage_execute(&q, &mut ack, context, metrics, evolution)
    {
        evolution.propagate("register_plugin", "注册插件失败");
        return false;
    }
    true
}
