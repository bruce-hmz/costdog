//! DeepSeek Harness（DSH）数据源。
//!
//! 会话记录：`~/.dsh/sessions/<project-slug>/<session-id>/session.v4.jsonl.zstd`
//! （zstd 压缩的 JSONL，一行一条事件）。只关心三类事件：
//!
//! ```text
//! {"type":"session", "id":.., "createdAt":.., "cwd":.., "origin":"subagent"?, "parentSession":..?}
//! {"type":"request/header",  "data":{"header":{"config":{"provider":..,"model":..}}}}
//! {"type":"request/context", "data":{"provider":..,"model":..,"contextWindow":..}}
//! {"type":"assistant/message", "time":.., "data":{"usage":{"inputTokens":..,"outputTokens":..,
//!                              "cacheReadTokens":?,"cacheWriteTokens":?}, "stream":[{"time":..},..]}}
//! ```
//!
//! 语义（用真实会话逐条校验过）：
//! * `inputTokens` 是**未缓存输入**——`input + cacheRead == totalTokens - output` 恒成立，
//!   与 Codex 同族，所以命中率分母必须是 `cacheRead / (input + cacheRead)`。
//! * `totalTokens` 是**该步**的合计（含 cacheRead），不是会话累计；会话累计要自己按步求和。
//! * 每一步都会把整个上下文重新计费一次，所以逐条求和就是真实计费口径。
//! * 会话可能跨天（实测一个会话 22:14 → 次日 09:12），因此必须按消息时间戳分到**本地日期**，
//!   否则"今日花费"会把整个跨天会话算进今天。
//! * 首条 stream 元素的 `time` 到消息 `time` 之间就是这次生成的耗时，可用于实时 TPS。

use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// sessions 表里的 source 值。
pub const SOURCE: &str = "dsh";

/// DSH 数据目录：`DSH_HOME` 可覆盖（与其它源一致地用环境变量兜底）。
pub fn dsh_home() -> PathBuf {
    std::env::var("DSH_HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            dirs::home_dir()
                .map(|home| home.join(".dsh"))
                .ok_or(())
        })
        .unwrap_or_else(|_| PathBuf::from(".dsh"))
}

pub fn sessions_root() -> PathBuf {
    dsh_home().join("sessions")
}

// ---- 原始记录（只声明用得到的字段，其余交给 serde 跳过：message 正文/stream 的 dt 数组很大）----

#[derive(Debug, Default, Clone, Copy, Deserialize)]
pub struct Usage {
    #[serde(rename = "inputTokens", default)]
    pub input: u64,
    #[serde(rename = "outputTokens", default)]
    pub output: u64,
    #[serde(rename = "cacheReadTokens", default)]
    pub cache_read: u64,
    #[serde(rename = "cacheWriteTokens", default)]
    pub cache_write: u64,
}

#[derive(Debug, Deserialize)]
struct StreamItem {
    #[serde(default)]
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MessageData {
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    stream: Vec<StreamItem>,
}

#[derive(Debug, Deserialize)]
struct MessageRecord {
    #[serde(default)]
    time: Option<i64>,
    #[serde(default)]
    data: Option<MessageData>,
}

#[derive(Debug, Default, Deserialize)]
struct SessionData {
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(rename = "parentSession", default)]
    parent: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SessionRecord {
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(rename = "parentSession", default)]
    parent: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct HeaderConfig {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Header {
    #[serde(default)]
    config: Option<HeaderConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct HeaderData {
    #[serde(default)]
    header: Option<Header>,
}

#[derive(Debug, Default, Deserialize)]
struct HeaderRecord {
    #[serde(default)]
    data: Option<HeaderData>,
}

#[derive(Debug, Default, Deserialize)]
struct ContextRecord {
    #[serde(default)]
    data: Option<ContextData>,
}

#[derive(Debug, Default, Deserialize)]
struct ContextData {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "contextWindow", default)]
    context_window: Option<u64>,
}

// ---- 聚合结果 ----

#[derive(Debug, Clone, Copy)]
pub struct Message {
    pub time_ms: i64,
    pub usage: Usage,
    /// 这次生成的耗时（首块 → 消息落库），用于实时 TPS。
    pub gen_span_ms: Option<i64>,
}

impl Message {
    /// 这次请求的上下文规模 = 未缓存输入 + 缓存读 + 缓存写。
    pub fn context_tokens(&self) -> u64 {
        self.usage.input + self.usage.cache_read + self.usage.cache_write
    }
}

#[derive(Debug, Clone)]
pub struct SessionUsage {
    pub id: String,
    pub cwd: String,
    pub project: String,
    pub model: String,
    pub provider: String,
    pub origin: String,
    pub parent: Option<String>,
    pub context_window: Option<u64>,
    pub created_at_ms: i64,
    pub messages: Vec<Message>,
}

impl SessionUsage {
    pub fn last_message(&self) -> Option<&Message> {
        self.messages.last()
    }

    /// 最近一次真实生成的 TPS：输出 token / 生成耗时。
    pub fn last_tps(&self) -> Option<f64> {
        let last = self.messages.last()?;
        let span = last.gen_span_ms? as f64;
        if span <= 0.0 || last.usage.output == 0 {
            return None;
        }
        Some(last.usage.output as f64 / (span / 1000.0))
    }

    pub fn last_context_tokens(&self) -> u64 {
        self.messages.last().map(|m| m.context_tokens()).unwrap_or(0)
    }
}

/// 单个本地日期的用量合计。
#[derive(Debug, Clone)]
pub struct DayBucket {
    pub date: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// 按本地日期聚合（日期由调用方注入，便于单测与复用 lib.rs 的本地时区口径）。
pub fn day_buckets<F>(session: &SessionUsage, date_of: F) -> Vec<DayBucket>
where
    F: Fn(i64) -> String,
{
    let mut out: Vec<DayBucket> = Vec::new();
    for m in &session.messages {
        if m.usage.input + m.usage.output + m.usage.cache_read + m.usage.cache_write == 0 {
            continue;
        }
        let date = date_of(m.time_ms);
        match out.last_mut() {
            // messages 已按时间排序，同一天的消息必然相邻。
            Some(bucket) if bucket.date == date => {
                bucket.input += m.usage.input;
                bucket.output += m.usage.output;
                bucket.cache_read += m.usage.cache_read;
                bucket.cache_write += m.usage.cache_write;
                bucket.end_ms = bucket.end_ms.max(m.time_ms);
                bucket.start_ms = bucket.start_ms.min(m.time_ms);
            }
            _ => out.push(DayBucket {
                date,
                input: m.usage.input,
                output: m.usage.output,
                cache_read: m.usage.cache_read,
                cache_write: m.usage.cache_write,
                start_ms: m.time_ms,
                end_ms: m.time_ms,
            }),
        }
    }
    out
}

/// 从 cwd 推断项目名（与其它源一致：取末级目录名）。
pub fn project_from_cwd(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "未命名".to_string())
}

/// 解析一段（已解压的）会话 JSONL。纯函数，便于单测。
pub fn parse_jsonl(id_hint: &str, text: &str) -> Option<SessionUsage> {
    let mut session = SessionUsage {
        id: id_hint.to_string(),
        cwd: String::new(),
        project: String::new(),
        model: String::new(),
        provider: String::new(),
        origin: "main".to_string(),
        parent: None,
        context_window: None,
        created_at_ms: 0,
        messages: Vec::new(),
    };
    let mut saw_header = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // 先按裸文本判类型再定向反序列化：assistant/message 里的 message 正文与 stream.dt
        // 数组占了文件绝大部分，定向结构体能让 serde 直接跳过，省内存也省时间。
        if line.contains("\"type\":\"assistant/message\"") || line.contains("\"type\": \"assistant/message\"") {
            if let Ok(record) = serde_json::from_str::<MessageRecord>(line) {
                let Some(data) = record.data else { continue };
                let Some(usage) = data.usage else { continue };
                if usage.input + usage.output + usage.cache_read + usage.cache_write == 0 {
                    continue;
                }
                let time_ms = record.time.unwrap_or(0);
                let first_chunk = data.stream.first().and_then(|item| item.time);
                let gen_span_ms = first_chunk
                    .filter(|start| time_ms > *start)
                    .map(|start| time_ms - start);
                session.messages.push(Message {
                    time_ms,
                    usage,
                    gen_span_ms,
                });
            }
        } else if line.contains("\"type\":\"session\"") || line.contains("\"type\": \"session\"") {
            if let Ok(record) = serde_json::from_str::<SessionRecord>(line) {
                if let Some(id) = record.id.filter(|id| !id.is_empty()) {
                    session.id = id;
                }
                session.created_at_ms = record.created_at.unwrap_or(0);
                session.cwd = record.cwd.unwrap_or_default();
                session.project = project_from_cwd(&session.cwd);
                session.origin = record.origin.unwrap_or_else(|| "main".to_string());
                session.parent = record.parent.filter(|parent| !parent.is_empty());
            }
        } else if line.contains("\"type\":\"request/header\"") || line.contains("\"type\": \"request/header\"") {
            if let Ok(record) = serde_json::from_str::<HeaderRecord>(line) {
                let config = record.data.and_then(|data| data.header).and_then(|header| header.config);
                if let Some(config) = config {
                    if let Some(model) = config.model.filter(|model| !model.is_empty()) {
                        session.model = model;
                    }
                    if let Some(provider) = config.provider.filter(|provider| !provider.is_empty()) {
                        session.provider = provider;
                    }
                    saw_header = true;
                }
            }
        } else if line.contains("\"type\":\"request/context\"") || line.contains("\"type\": \"request/context\"") {
            if let Ok(record) = serde_json::from_str::<ContextRecord>(line) {
                if let Some(data) = record.data {
                    if session.model.is_empty() {
                        session.model = data.model.unwrap_or_default();
                    }
                    if session.provider.is_empty() {
                        session.provider = data.provider.unwrap_or_default();
                    }
                    if data.context_window.is_some() {
                        session.context_window = data.context_window;
                    }
                }
            }
        }
    }

    // 没有任何带用量消息的会话（空壳/未使用）直接丢掉，避免 0 成本噪音。
    if session.messages.is_empty() {
        return None;
    }
    if session.project.is_empty() {
        session.project = project_from_cwd(&session.cwd);
    }
    if session.model.is_empty() && !saw_header {
        session.model = "unknown".to_string();
    }
    session.messages.sort_by_key(|message| message.time_ms);
    Some(session)
}

/// 解压 DSH 的会话文件。
///
/// 注意：这些文件是**多帧 zstd** —— DSH 每写一条事件就追加一个独立帧，实测单文件可达
/// 1000+ 帧。`ruzstd::decoding::StreamingDecoder` 只解第一帧（真实文件里第一帧只有
/// 301 字节，正好是那条 `{"type":"session"}` 记录），用它会让每个会话都被判成"没有用量"
/// 而静默跳过。所以这里自己按帧循环：init → decode_blocks(All) → collect。
pub fn read_zstd_frames<R: std::io::Read>(source: R) -> Result<String, String> {
    use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
    let mut source = std::io::BufReader::new(source);
    let mut decoder = FrameDecoder::new();
    let mut out: Vec<u8> = Vec::new();
    let mut frames = 0usize;
    loop {
        // init 失败 = 没有下一帧了（正常结束），或尾巴被截断（写到一半）——已解出的照常用。
        if decoder.init(&mut source).is_err() {
            break;
        }
        loop {
            match decoder.decode_blocks(&mut source, BlockDecodingStrategy::All) {
                Ok(true) => break,
                Ok(false) => continue,
                Err(error) => return Err(format!("zstd frame decode failed: {error:?}")),
            }
        }
        decoder
            .collect_to_writer(&mut out)
            .map_err(|error| format!("zstd collect failed: {error:?}"))?;
        frames += 1;
    }
    if frames == 0 {
        return Err("no zstd frame found".to_string());
    }
    String::from_utf8(out).map_err(|error| error.to_string())
}

/// 读取并解压一个 session.v4.jsonl.zstd。
///
/// `Ok(None)` = 文件读得了、只是这个会话还没有任何带用量的消息（空壳/种子会话）——
/// 这**不是**畸形文件：调用方应记下指纹跳过它，别每轮重新解压、也别记进
/// `malformed_lines`（诊断面板里那行会变成"假故障"）。`Err` 才是真的读不出来。
pub fn read_session_file(path: &Path, id_hint: &str) -> Result<Option<SessionUsage>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let text = read_zstd_frames(file)?;
    Ok(parse_jsonl(id_hint, &text))
}

/// 找出所有会话文件（`<root>/<project-slug>/<session-id>/session.v4.jsonl.zstd`）。
pub fn find_session_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return out;
    };
    for project in projects.flatten() {
        let Ok(sessions) = std::fs::read_dir(project.path()) else { continue };
        for session in sessions.flatten() {
            let file = session.path().join("session.v4.jsonl.zstd");
            if file.is_file() {
                out.push(file);
            }
        }
    }
    out.sort();
    out
}

// ---- projcache（`~/.dsh/storages/session_projcache/sessions/<id>.json`）----
//
// 小型 JSON（几 KB~几十 KB），DSH 自己用来渲染会话列表/上下文压力。它给出的
// `contextPressure.pressureTokens` 等于最后一步的 totalTokens（实测 443791 vs
// 逐条求和 443731），`sessionStats` 直接就是面板要的会话计时。用它做"实时/单会话"
// 查询可以完全绕开 zstd 解压。

pub fn projcache_dir() -> PathBuf {
    dsh_home().join("storages").join("session_projcache").join("sessions")
}

#[derive(Debug, Default, Deserialize)]
struct CachedRow<T> {
    #[serde(default)]
    val: Option<T>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedIdentity {
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedTotals {
    #[serde(rename = "uncachedInputTokens", default)]
    input: u64,
    #[serde(rename = "outputTokens", default)]
    output: u64,
    #[serde(rename = "cacheReadTokens", default)]
    cache_read: u64,
    #[serde(rename = "cacheWriteTokens", default)]
    cache_write: u64,
}

impl CachedTotals {
    fn to_usage(&self) -> Usage {
        Usage {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct CachedTokenUsage {
    #[serde(default)]
    totals: Option<CachedTotals>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedPressure {
    #[serde(rename = "surfaceTokens", default)]
    surface: u64,
    #[serde(rename = "pressureTokens", default)]
    pressure: u64,
    #[serde(rename = "contextWindow", default)]
    context_window: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedModel {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedModelSelection {
    #[serde(rename = "lastUsed", default)]
    last_used: Option<CachedModel>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedStats {
    #[serde(default)]
    turns: Option<u64>,
    #[serde(default)]
    steps: Option<u64>,
    #[serde(rename = "llmMs", default)]
    llm_ms: Option<i64>,
    #[serde(rename = "toolMs", default)]
    tool_ms: Option<i64>,
    #[serde(rename = "ttftMs", default)]
    ttft_ms: Option<i64>,
    #[serde(rename = "ttftSteps", default)]
    ttft_steps: Option<u64>,
    #[serde(rename = "decodeMs", default)]
    decode_ms: Option<i64>,
    #[serde(rename = "decodeTokens", default)]
    decode_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedListMeta {
    #[serde(rename = "lastPromptAt", default)]
    last_prompt_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedRows {
    #[serde(default)]
    title: Option<CachedRow<String>>,
    #[serde(rename = "tokenUsage", default)]
    token_usage: Option<CachedRow<CachedTokenUsage>>,
    #[serde(rename = "contextPressure", default)]
    context_pressure: Option<CachedRow<CachedPressure>>,
    #[serde(rename = "modelSelection", default)]
    model_selection: Option<CachedRow<CachedModelSelection>>,
    #[serde(rename = "sessionStats", default)]
    session_stats: Option<CachedRow<CachedStats>>,
    #[serde(rename = "sessionListMetadata", default)]
    list_metadata: Option<CachedRow<CachedListMeta>>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedRecord {
    #[serde(default)]
    identity: Option<CachedIdentity>,
    #[serde(default)]
    rows: Option<CachedRows>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjCacheFile {
    #[serde(default)]
    record: Option<CachedRecord>,
}

/// 会话缓存里的一行（用于会话列表 / 8 项指标；字段都做了缺省兜底）。
#[derive(Debug, Clone, Default)]
pub struct CachedSession {
    pub id: String,
    pub cwd: String,
    pub project: String,
    pub title: String,
    pub model: String,
    pub provider: String,
    pub totals: Usage,
    pub context_tokens: u64,
    pub context_window: Option<u64>,
    pub last_active_ms: i64,
    pub llm_ms: i64,
    pub tool_ms: i64,
    pub ttft_ms: i64,
    pub ttft_steps: u64,
    pub decode_ms: i64,
    pub decode_tokens: u64,
    pub turns: u64,
    pub steps: u64,
}

impl CachedSession {
    /// 平均首 token 延迟（DSH 只给总量与步数）。
    pub fn avg_ttft_ms(&self) -> i64 {
        if self.ttft_steps == 0 {
            0
        } else {
            self.ttft_ms / self.ttft_steps as i64
        }
    }

    /// 会话平均解码速度（tok/s）。
    pub fn decode_tps(&self) -> f64 {
        if self.decode_ms <= 0 {
            0.0
        } else {
            self.decode_tokens as f64 / (self.decode_ms as f64 / 1000.0)
        }
    }
}

pub fn parse_projcache(id_hint: &str, text: &str) -> Option<CachedSession> {
    let file: ProjCacheFile = serde_json::from_str(text).ok()?;
    let record = file.record?;
    let cwd = record.identity.and_then(|identity| identity.cwd).unwrap_or_default();
    let rows = record.rows.unwrap_or_default();
    let totals = rows
        .token_usage
        .and_then(|row| row.val)
        .and_then(|usage| usage.totals)
        .map(|totals| totals.to_usage())
        .unwrap_or_default();
    let pressure = rows.context_pressure.and_then(|row| row.val).unwrap_or_default();
    let model = rows
        .model_selection
        .and_then(|row| row.val)
        .and_then(|selection| selection.last_used)
        .unwrap_or_default();
    let stats = rows.session_stats.and_then(|row| row.val).unwrap_or_default();
    let meta = rows.list_metadata.and_then(|row| row.val).unwrap_or_default();
    Some(CachedSession {
        id: id_hint.to_string(),
        project: project_from_cwd(&cwd),
        cwd,
        title: rows.title.and_then(|row| row.val).unwrap_or_default(),
        model: model.model.unwrap_or_default(),
        provider: model.provider.unwrap_or_default(),
        totals,
        // pressureTokens 更贴近"这次请求的上下文"，surfaceTokens 是 DSH 自己的展示口径；
        // 取两者较大值，避免刚开新会话时读到 0。
        context_tokens: pressure.pressure.max(pressure.surface),
        context_window: pressure.context_window,
        last_active_ms: meta.last_prompt_at.unwrap_or(0),
        llm_ms: stats.llm_ms.unwrap_or(0),
        tool_ms: stats.tool_ms.unwrap_or(0),
        ttft_ms: stats.ttft_ms.unwrap_or(0),
        ttft_steps: stats.ttft_steps.unwrap_or(0),
        decode_ms: stats.decode_ms.unwrap_or(0),
        decode_tokens: stats.decode_tokens.unwrap_or(0),
        turns: stats.turns.unwrap_or(0),
        steps: stats.steps.unwrap_or(0),
    })
}

pub fn read_projcache(path: &Path, id_hint: &str) -> Option<CachedSession> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_projcache(id_hint, &text)
}

/// 读取某个会话的缓存（用于面板的 8 项指标）。
pub fn find_cached_session(session_id: &str) -> Option<CachedSession> {
    let path = projcache_dir().join(format!("{session_id}.json"));
    read_projcache(&path, session_id)
}

/// 全部会话缓存，按最近活跃倒序（用于会话列表）。
pub fn list_cached_sessions() -> Vec<CachedSession> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(projcache_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let id = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        if let Some(session) = read_projcache(&path, &id) {
            out.push(session);
        }
    }
    out.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"type":"session","version":4,"id":"session-abc","createdAt":1000,"cwd":"/Users/x/workspace/demo","delegationDepth":0}
{"type":"request/header","seq":1,"time":1100,"data":{"header":{"config":{"provider":"opencode-go","model":"deepseek-v4.1-flash","reasoningEffort":"max"}}}}
{"type":"request/context","seq":2,"time":1110,"data":{"provider":"opencode-go","model":"deepseek-v4.1-flash","contextWindow":1000000}}
{"type":"assistant/message","seq":3,"time":5000,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"reasoning","text":"…"}]},"usage":{"inputTokens":1000,"outputTokens":200,"totalTokens":1200},"stream":[{"type":"chunk","time":2000,"chunk":{"type":"block-start"}}]}}
{"type":"step/end","seq":4,"time":5100,"data":{"turn":1,"step":1}}
{"type":"assistant/message","seq":5,"time":9000,"data":{"turn":1,"step":2,"usage":{"inputTokens":300,"outputTokens":400,"totalTokens":4300,"cacheReadTokens":3600},"stream":[{"time":6000}]}}
{"type":"assistant/message","seq":6,"time":90000000,"data":{"turn":2,"step":1,"usage":{"inputTokens":50,"outputTokens":60,"totalTokens":110,"cacheReadTokens":0},"stream":[{"time":89000000}]}}
"#;

    #[test]
    fn parses_usage_model_and_subagent_metadata() {
        let session = parse_jsonl("hint", FIXTURE).expect("session");
        assert_eq!(session.id, "session-abc");
        assert_eq!(session.project, "demo");
        assert_eq!(session.model, "deepseek-v4.1-flash");
        assert_eq!(session.provider, "opencode-go");
        assert_eq!(session.context_window, Some(1_000_000));
        assert_eq!(session.messages.len(), 3);
        // 生成耗时 = 消息时间 - 首块时间
        assert_eq!(session.messages[0].gen_span_ms, Some(3000));
        assert_eq!(session.messages[1].gen_span_ms, Some(3000));
        // 上下文规模 = 未缓存输入 + 缓存读
        assert_eq!(session.last_context_tokens(), 50);
        assert_eq!(session.messages[1].context_tokens(), 300 + 3600);
        // TPS = 输出 / 生成耗时
        assert!((session.messages[0].context_tokens() as f64 - 1000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn buckets_usage_by_local_date_across_midnight() {
        let session = parse_jsonl("hint", FIXTURE).expect("session");
        // 把 0..10000ms 归到 day1，其余归 day2，模拟跨天会话
        let buckets = day_buckets(&session, |ms| if ms < 10_000 { "day1".into() } else { "day2".into() });
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].date, "day1");
        assert_eq!(buckets[0].input, 1300);
        assert_eq!(buckets[0].output, 600);
        assert_eq!(buckets[0].cache_read, 3600);
        assert_eq!(buckets[0].start_ms, 5000);
        assert_eq!(buckets[0].end_ms, 9000);
        assert_eq!(buckets[1].date, "day2");
        assert_eq!(buckets[1].input, 50);
        assert_eq!(buckets[1].output, 60);
    }

    #[test]
    fn empty_session_without_usage_is_dropped() {
        let text = r#"{"type":"session","id":"session-empty","createdAt":1,"cwd":"/tmp/x"}
{"type":"assistant/message","seq":1,"time":2,"data":{"turn":1,"step":1,"usage":{"inputTokens":0,"outputTokens":0,"totalTokens":0}}}
"#;
        assert!(parse_jsonl("hint", text).is_none());
    }

    #[test]
    fn last_tps_uses_decode_span() {
        let session = parse_jsonl("hint", FIXTURE).expect("session");
        // 最后一条：60 token / (90000000-89000000)ms = 60 / 1000s = 0.06 token/s
        let tps = session.last_tps().expect("tps");
        assert!((tps - 0.06).abs() < 1e-9, "tps={tps}");
    }

    const CACHE_FIXTURE: &str = r#"{"version":7,"record":{"identity":{"formatVersion":4,"createdAt":1790691230932,"cwd":"/Users/x/workspace/demo"},"rows":{
"title":{"ver":1,"seq":9,"val":"Fix the thing"},
"tokenUsage":{"ver":2,"seq":1474,"val":{"totals":{"uncachedInputTokens":4940157,"outputTokens":298450,"cacheReadTokens":62115968,"cacheWriteTokens":0},"last":{"turn":9,"step":6,"buckets":{"uncachedInputTokens":862,"outputTokens":501,"cacheReadTokens":442368,"cacheWriteTokens":0}}}},
"contextPressure":{"ver":1,"val":{"surfaceTokens":342099,"contextWindow":1000000,"pressureTokens":443791}},
"modelSelection":{"ver":1,"val":{"lastUsed":{"provider":"opencode-go","model":"deepseek-v4.1-flash"}}},
"sessionStats":{"ver":1,"val":{"turns":9,"steps":246,"llmMs":5277384,"toolMs":1262680,"ttftMs":2572855,"ttftSteps":245,"decodeMs":2704529,"decodeTokens":299000}},
"sessionListMetadata":{"ver":1,"val":{"blank":false,"lastPromptAt":1790730500033}}}}}"#;

    #[test]
    fn reads_projcache_snapshot() {
        let cached = parse_projcache("session-x", CACHE_FIXTURE).expect("cache");
        assert_eq!(cached.id, "session-x");
        assert_eq!(cached.project, "demo");
        assert_eq!(cached.title, "Fix the thing");
        assert_eq!(cached.model, "deepseek-v4.1-flash");
        assert_eq!(cached.provider, "opencode-go");
        assert_eq!(cached.totals.cache_read, 62_115_968);
        assert_eq!(cached.totals.input, 4_940_157);
        // pressureTokens 与 surfaceTokens 取较大者
        assert_eq!(cached.context_tokens, 443_791);
        assert_eq!(cached.context_window, Some(1_000_000));
        assert_eq!(cached.last_active_ms, 1_790_730_500_033);
        assert_eq!(cached.avg_ttft_ms(), 2_572_855 / 245);
        assert!((cached.decode_tps() - 299_000.0 / 2704.529).abs() < 0.5);
    }

    /// DSH 的会话文件是多帧 zstd（每条事件一帧）。这里用 ruzstd 自带的压缩器造一个
    /// 两帧文件，验证多帧循环能全部解出来 —— 用 StreamingDecoder 时只会拿到第一帧。
    #[test]
    fn decodes_multi_frame_zstd_like_dsh_writes_it() {
        use ruzstd::encoding::{compress_to_vec, CompressionLevel};
        let (head, tail) = FIXTURE.split_at(FIXTURE.len() / 2);
        let mut blob = compress_to_vec(head.as_bytes(), CompressionLevel::Uncompressed);
        blob.extend(compress_to_vec(tail.as_bytes(), CompressionLevel::Uncompressed));
        let text = read_zstd_frames(&blob[..]).expect("multi-frame decode");
        assert!(text.contains("90000000"), "解出的文本缺少尾部记录");
        let session = parse_jsonl("hint", &text).expect("session");
        // 两帧里的记录都要在（首帧只有 session/header/context + 第一条消息）
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.model, "deepseek-v4.1-flash");
    }

    #[test]
    fn empty_zstd_input_is_an_error() {
        assert!(read_zstd_frames(&b""[..]).is_err());
    }

    #[test]
    fn tolerates_projcache_without_optional_rows() {
        let text = r#"{"version":7,"record":{"identity":{"cwd":"/tmp/solo"},"rows":{"tokenUsage":{"val":{"totals":{"uncachedInputTokens":10,"outputTokens":2}}}}}}"#;
        let cached = parse_projcache("solo", text).expect("cache");
        assert_eq!(cached.project, "solo");
        assert_eq!(cached.totals.output, 2);
        assert_eq!(cached.context_tokens, 0);
        assert_eq!(cached.avg_ttft_ms(), 0);
        assert_eq!(cached.decode_tps(), 0.0);
    }
}
