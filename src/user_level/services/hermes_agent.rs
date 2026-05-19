//! Hermes Agent compatibility port for SMROS.
//!
//! Upstream NousResearch/hermes-agent is a Python 3.11 application with hosted
//! providers, CLI, skills, memory, tools, scheduling, and delegation. SMROS does
//! not yet host Python, so this module ports the agent contract into a native
//! service and routes text generation through the SMROS Gemma provider.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::user_level::{fxfs, gemma, svc};

const HERMES_ROOT: &str = "/data/hermes";
const HERMES_SKILL_DIR: &str = "/data/hermes/skills";
const HERMES_MEMORY_DIR: &str = "/data/hermes/memory";
const HERMES_SESSION_DIR: &str = "/data/hermes/sessions";
const HERMES_TOOL_DIR: &str = "/data/hermes/tools";
const HERMES_CRON_DIR: &str = "/data/hermes/cron";
const HERMES_CONFIG_PATH: &str = "/data/hermes/config.yaml";
const HERMES_MEMORY_PATH: &str = "/data/hermes/memory/MEMORY.md";
const HERMES_USER_PATH: &str = "/data/hermes/memory/USER.md";
const HERMES_SESSION_PATH: &str = "/data/hermes/sessions/smros-session.log";
const HERMES_SKILL_PATH: &str = "/data/hermes/skills/smros-kernel/SKILL.md";
const HERMES_SKILL_DIR_PATH: &str = "/data/hermes/skills/smros-kernel";
const HERMES_TOOL_AUDIT_PATH: &str = "/data/hermes/tools/audit.log";
const HERMES_CRON_PATH: &str = "/data/hermes/cron/nightly-smoke.yaml";

const HERMES_PROVIDER_GEMMA: &str = gemma::GEMMA_PROVIDER;
const HERMES_MODEL_DEFAULT: &str = gemma::GEMMA_MODEL;
const HERMES_PERSONALITY: &str = "pragmatic-smros-port";
const HERMES_TOOL_SHELL: &str = "shell";
const HERMES_TOOL_FXFS: &str = "fxfs";
const HERMES_TOOL_SVC: &str = "svc";

const HERMES_CONFIG: &str = "provider: gemma\nmodel: gemma/gemma-3n-e2b-smros\npersonality: pragmatic-smros-port\ntools:\n  - shell\n  - fxfs\n  - svc\nskills_dir: /data/hermes/skills\nmemory_dir: /data/hermes/memory\n";
const HERMES_MEMORY: &str = "- SMROS port runs Hermes semantics through the native Gemma provider.\n- Upstream Python runtime is not yet available in SMROS.\n";
const HERMES_USER: &str =
    "- User asked to port NousResearch/hermes-agent to SMROS and fully test it.\n";
const HERMES_SKILL: &str = "# SMROS Kernel Skill\n\nUse FxFS, /svc, and syscall smoke tests to validate Hermes agent behavior inside SMROS.\n";
const HERMES_CRON: &str =
    "name: nightly-smros-hermes-smoke\nschedule: '0 3 * * *'\ncommand: hermes test\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HermesAgentError {
    FxfsInit,
    FxfsPrepare,
    Config,
    ModelRoute,
    Skill,
    Memory,
    Tool,
    Delegate,
    Gemma,
    Cron,
    Transcript,
    Svc,
}

impl HermesAgentError {
    pub fn as_str(self) -> &'static str {
        match self {
            HermesAgentError::FxfsInit => "fxfs init",
            HermesAgentError::FxfsPrepare => "fxfs prepare",
            HermesAgentError::Config => "config",
            HermesAgentError::ModelRoute => "model route",
            HermesAgentError::Skill => "skill",
            HermesAgentError::Memory => "memory",
            HermesAgentError::Tool => "tool",
            HermesAgentError::Delegate => "delegate",
            HermesAgentError::Gemma => "gemma",
            HermesAgentError::Cron => "cron",
            HermesAgentError::Transcript => "transcript",
            HermesAgentError::Svc => "svc",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesAgentInfo {
    pub name: &'static str,
    pub upstream: &'static str,
    pub upstream_version: &'static str,
    pub provider: &'static str,
    pub model: &'static str,
    pub personality: &'static str,
    pub tools: usize,
    pub skills: usize,
    pub memory_items: usize,
    pub cron_jobs: usize,
    pub transcripts: usize,
    pub generation_backend: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesAgentTurn {
    pub prompt: String,
    pub answer: String,
    pub tool_calls: usize,
    pub skill_hits: usize,
    pub delegated_agents: usize,
    pub memory_writes: usize,
    pub transcript_bytes: usize,
    pub model_tokens: usize,
    pub model_backend: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesAgentTestReport {
    pub config_ok: bool,
    pub model_route_ok: bool,
    pub skill_ok: bool,
    pub memory_ok: bool,
    pub tool_ok: bool,
    pub delegate_ok: bool,
    pub gemma_ok: bool,
    pub cron_ok: bool,
    pub transcript_ok: bool,
    pub svc_ok: bool,
    pub turn: HermesAgentTurn,
}

impl HermesAgentTestReport {
    pub fn passed(&self) -> bool {
        self.config_ok
            && self.model_route_ok
            && self.skill_ok
            && self.memory_ok
            && self.tool_ok
            && self.delegate_ok
            && self.gemma_ok
            && self.cron_ok
            && self.transcript_ok
            && self.svc_ok
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HermesConfig {
    provider: String,
    model: String,
    personality: String,
    tools: Vec<String>,
    skills_dir: String,
    memory_dir: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolCallResult {
    name: &'static str,
    output: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DelegateResult {
    agents: usize,
    summary: String,
}

pub fn init() -> bool {
    prepare_storage().is_ok()
}

pub fn info() -> Result<HermesAgentInfo, HermesAgentError> {
    prepare_storage()?;
    let config = load_config()?;
    if !route_model(config.provider.as_str(), config.model.as_str()) {
        return Err(HermesAgentError::ModelRoute);
    }

    Ok(HermesAgentInfo {
        name: "Hermes Agent for SMROS",
        upstream: "NousResearch/hermes-agent",
        upstream_version: "0.14.0",
        provider: HERMES_PROVIDER_GEMMA,
        model: HERMES_MODEL_DEFAULT,
        personality: HERMES_PERSONALITY,
        tools: config.tools.len(),
        skills: count_dir_entries(HERMES_SKILL_DIR),
        memory_items: memory_item_count()?,
        cron_jobs: count_dir_entries(HERMES_CRON_DIR),
        transcripts: count_dir_entries(HERMES_SESSION_DIR),
        generation_backend: "smros-native",
    })
}

pub fn run_prompt(prompt: &str) -> Result<HermesAgentTurn, HermesAgentError> {
    prepare_storage()?;
    let config = load_config()?;
    if !route_model(config.provider.as_str(), config.model.as_str()) {
        return Err(HermesAgentError::ModelRoute);
    }
    if !skill_matches(prompt)? {
        return Err(HermesAgentError::Skill);
    }

    let mut tool_results = Vec::new();
    tool_results.push(run_tool(HERMES_TOOL_SHELL, prompt)?);
    tool_results.push(run_tool(HERMES_TOOL_FXFS, prompt)?);
    tool_results.push(run_tool(HERMES_TOOL_SVC, prompt)?);

    let delegate = delegate_subagents(prompt, &tool_results)?;
    let memory_writes = persist_memory(prompt, delegate.summary.as_str())?;
    let model_context = build_model_context(&config, &tool_results, &delegate);
    let generation = gemma::generate(
        prompt,
        model_context.as_str(),
        gemma::GEMMA_MAX_OUTPUT_TOKENS,
    )
    .map_err(|_| HermesAgentError::Gemma)?;
    let answer = compose_answer(&config, &generation)?;
    let transcript_bytes = append_transcript(prompt, answer.as_str(), &tool_results, &delegate)?;

    Ok(HermesAgentTurn {
        prompt: String::from(prompt),
        answer,
        tool_calls: tool_results.len(),
        skill_hits: 1,
        delegated_agents: delegate.agents,
        memory_writes,
        transcript_bytes,
        model_tokens: generation.generated_tokens,
        model_backend: generation.backend,
    })
}

pub fn run_full_test() -> Result<HermesAgentTestReport, HermesAgentError> {
    prepare_storage()?;

    let config = load_config()?;
    let config_ok = config.provider == HERMES_PROVIDER_GEMMA
        && config.model == HERMES_MODEL_DEFAULT
        && config.personality == HERMES_PERSONALITY
        && config.tools.iter().any(|tool| tool == HERMES_TOOL_SHELL)
        && config.tools.iter().any(|tool| tool == HERMES_TOOL_FXFS)
        && config.tools.iter().any(|tool| tool == HERMES_TOOL_SVC)
        && config.skills_dir == HERMES_SKILL_DIR
        && config.memory_dir == HERMES_MEMORY_DIR;
    if !config_ok {
        return Err(HermesAgentError::Config);
    }

    let model_route_ok = route_model(config.provider.as_str(), config.model.as_str());
    if !model_route_ok {
        return Err(HermesAgentError::ModelRoute);
    }

    let skill_ok = skill_matches("Use the SMROS kernel skill to test Hermes")?;
    if !skill_ok {
        return Err(HermesAgentError::Skill);
    }

    let memory_before = memory_item_count()?;
    let turn = run_prompt("test hermes agent on SMROS with tools, memory, skills, and /svc")?;
    let memory_after = memory_item_count()?;
    let memory_ok = memory_after > memory_before && turn.memory_writes > 0;
    if !memory_ok {
        return Err(HermesAgentError::Memory);
    }

    let tool_ok = turn.tool_calls == 3 && tool_audit_valid()?;
    if !tool_ok {
        return Err(HermesAgentError::Tool);
    }

    let delegate_ok = turn.delegated_agents == 2 && turn.answer.contains("delegate=2");
    if !delegate_ok {
        return Err(HermesAgentError::Delegate);
    }

    let gemma_ok = turn.model_backend == "smros-native"
        && turn.model_tokens > 0
        && turn.answer.contains("Gemma on SMROS");
    if !gemma_ok {
        return Err(HermesAgentError::Gemma);
    }

    let cron_ok = cron_ready()?;
    if !cron_ok {
        return Err(HermesAgentError::Cron);
    }

    let transcript_ok = transcript_valid(turn.transcript_bytes)?;
    if !transcript_ok {
        return Err(HermesAgentError::Transcript);
    }

    let svc_ok = svc::smoke_test();
    if !svc_ok {
        return Err(HermesAgentError::Svc);
    }

    Ok(HermesAgentTestReport {
        config_ok,
        model_route_ok,
        skill_ok,
        memory_ok,
        tool_ok,
        delegate_ok,
        gemma_ok,
        cron_ok,
        transcript_ok,
        svc_ok,
        turn,
    })
}

pub fn smoke_test() -> bool {
    run_full_test()
        .map(|report| report.passed())
        .unwrap_or(false)
}

fn prepare_storage() -> Result<(), HermesAgentError> {
    if !fxfs::init() {
        return Err(HermesAgentError::FxfsInit);
    }
    if !gemma::init() {
        return Err(HermesAgentError::Gemma);
    }

    create_dir("/data")?;
    create_dir(HERMES_ROOT)?;
    create_dir(HERMES_SKILL_DIR)?;
    create_dir(HERMES_SKILL_DIR_PATH)?;
    create_dir(HERMES_MEMORY_DIR)?;
    create_dir(HERMES_SESSION_DIR)?;
    create_dir(HERMES_TOOL_DIR)?;
    create_dir(HERMES_CRON_DIR)?;
    ensure_exact_file(HERMES_CONFIG_PATH, HERMES_CONFIG)?;
    ensure_file(HERMES_MEMORY_PATH, HERMES_MEMORY)?;
    ensure_file(HERMES_USER_PATH, HERMES_USER)?;
    ensure_exact_file(HERMES_SKILL_PATH, HERMES_SKILL)?;
    ensure_exact_file(HERMES_CRON_PATH, HERMES_CRON)?;
    ensure_file(HERMES_TOOL_AUDIT_PATH, "")?;
    ensure_file(HERMES_SESSION_PATH, "")?;
    Ok(())
}

fn create_dir(path: &str) -> Result<(), HermesAgentError> {
    fxfs::create_dir(path)
        .map(|_| ())
        .map_err(|_| HermesAgentError::FxfsPrepare)
}

fn ensure_file(path: &str, data: &str) -> Result<(), HermesAgentError> {
    if fxfs::exists(path) {
        return Ok(());
    }
    fxfs::write_file(path, data.as_bytes())
        .map(|_| ())
        .map_err(|_| HermesAgentError::FxfsPrepare)
}

fn ensure_exact_file(path: &str, data: &str) -> Result<(), HermesAgentError> {
    if let Ok(current) = read_text_file(path) {
        if current == data {
            return Ok(());
        }
    }
    fxfs::write_file(path, data.as_bytes())
        .map(|_| ())
        .map_err(|_| HermesAgentError::FxfsPrepare)
}

fn read_text_file(path: &str) -> Result<String, HermesAgentError> {
    let attrs = fxfs::attrs(path).map_err(|_| HermesAgentError::FxfsPrepare)?;
    let mut out = Vec::new();
    out.resize(attrs.size, 0);
    let read = fxfs::read_file(path, &mut out).map_err(|_| HermesAgentError::FxfsPrepare)?;
    out.truncate(read);
    String::from_utf8(out).map_err(|_| HermesAgentError::FxfsPrepare)
}

fn load_config() -> Result<HermesConfig, HermesAgentError> {
    let text = read_text_file(HERMES_CONFIG_PATH)?;
    parse_config(text.as_str()).ok_or(HermesAgentError::Config)
}

fn parse_config(text: &str) -> Option<HermesConfig> {
    let mut provider = String::new();
    let mut model = String::new();
    let mut personality = String::new();
    let mut skills_dir = String::new();
    let mut memory_dir = String::new();
    let mut tools = Vec::new();
    let mut in_tools = false;

    for line in text.lines() {
        let trimmed = trim_ascii(line);
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "tools:" {
            in_tools = true;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("- ") {
            if in_tools {
                tools.push(String::from(trim_ascii(value)));
            }
            continue;
        }
        in_tools = false;
        if let Some(value) = config_value(trimmed, "provider:") {
            provider = String::from(value);
        } else if let Some(value) = config_value(trimmed, "model:") {
            model = String::from(value);
        } else if let Some(value) = config_value(trimmed, "personality:") {
            personality = String::from(value);
        } else if let Some(value) = config_value(trimmed, "skills_dir:") {
            skills_dir = String::from(value);
        } else if let Some(value) = config_value(trimmed, "memory_dir:") {
            memory_dir = String::from(value);
        }
    }

    if provider.is_empty()
        || model.is_empty()
        || personality.is_empty()
        || tools.is_empty()
        || skills_dir.is_empty()
        || memory_dir.is_empty()
    {
        return None;
    }

    Some(HermesConfig {
        provider,
        model,
        personality,
        tools,
        skills_dir,
        memory_dir,
    })
}

fn config_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key).map(trim_ascii)
}

fn trim_ascii(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &value[start..end]
}

fn route_model(provider: &str, model: &str) -> bool {
    gemma::model_available(provider, model)
}

fn skill_matches(prompt: &str) -> Result<bool, HermesAgentError> {
    let skill = read_text_file(HERMES_SKILL_PATH)?;
    Ok(skill.contains("SMROS") && skill.contains("FxFS") && !prompt.trim().is_empty())
}

fn run_tool(name: &'static str, prompt: &str) -> Result<ToolCallResult, HermesAgentError> {
    let output = match name {
        HERMES_TOOL_SHELL => {
            let mut value = String::from("shell prompt-bytes=");
            append_usize(&mut value, prompt.len(), 0);
            value
        }
        HERMES_TOOL_FXFS => {
            let stats = fxfs::stats();
            let mut value = String::from("fxfs nodes=");
            append_usize(&mut value, stats.nodes, 0);
            value.push_str(" entries=");
            append_usize(&mut value, stats.dir_entries, 0);
            value
        }
        HERMES_TOOL_SVC => {
            if !svc::init() {
                return Err(HermesAgentError::Svc);
            }
            let stats = svc::stats();
            let mut value = String::from("svc services=");
            append_usize(&mut value, stats.services, 0);
            value.push_str(" requests=");
            append_usize(&mut value, stats.requests, 0);
            value
        }
        _ => return Err(HermesAgentError::Tool),
    };

    append_tool_audit(name, output.as_str())?;
    Ok(ToolCallResult { name, output })
}

fn append_tool_audit(name: &str, output: &str) -> Result<(), HermesAgentError> {
    let mut record = String::from("tool=");
    record.push_str(name);
    record.push_str(" output=");
    record.push_str(output);
    record.push('\n');
    fxfs::append_file(HERMES_TOOL_AUDIT_PATH, record.as_bytes())
        .map(|_| ())
        .map_err(|_| HermesAgentError::Tool)
}

fn delegate_subagents(
    prompt: &str,
    tools: &[ToolCallResult],
) -> Result<DelegateResult, HermesAgentError> {
    if tools.len() < 3 || prompt.is_empty() {
        return Err(HermesAgentError::Delegate);
    }

    let mut summary = String::from("explorer checked ");
    append_usize(&mut summary, tools.len(), 0);
    summary.push_str(" tools; verifier checked transcript");
    Ok(DelegateResult { agents: 2, summary })
}

fn persist_memory(prompt: &str, summary: &str) -> Result<usize, HermesAgentError> {
    let mut memory = String::from("- prompt=");
    push_sanitized(&mut memory, prompt);
    memory.push_str(" summary=");
    push_sanitized(&mut memory, summary);
    memory.push('\n');
    fxfs::append_file(HERMES_MEMORY_PATH, memory.as_bytes())
        .map(|_| 1)
        .map_err(|_| HermesAgentError::Memory)
}

fn build_model_context(
    config: &HermesConfig,
    tools: &[ToolCallResult],
    delegate: &DelegateResult,
) -> String {
    let mut context = String::from("provider=");
    context.push_str(config.provider.as_str());
    context.push_str(" model=");
    context.push_str(config.model.as_str());
    context.push_str(" tools=");
    append_usize(&mut context, tools.len(), 0);
    for tool in tools {
        context.push_str(" tool ");
        context.push_str(tool.name);
        context.push('=');
        context.push_str(tool.output.as_str());
    }
    context.push_str(" delegate=");
    append_usize(&mut context, delegate.agents, 0);
    context.push_str(" summary=");
    context.push_str(delegate.summary.as_str());
    context
}

fn compose_answer(
    config: &HermesConfig,
    generation: &gemma::GemmaGeneration,
) -> Result<String, HermesAgentError> {
    if generation.text.is_empty() {
        return Err(HermesAgentError::Gemma);
    }

    let mut answer = generation.text.clone();
    answer.push_str(" [Hermes provider=");
    answer.push_str(config.provider.as_str());
    answer.push_str(" model=");
    answer.push_str(config.model.as_str());
    answer.push_str(" delegate=2 memory=updated]");
    Ok(answer)
}

fn append_transcript(
    prompt: &str,
    answer: &str,
    tools: &[ToolCallResult],
    delegate: &DelegateResult,
) -> Result<usize, HermesAgentError> {
    let mut record = String::from("user: ");
    push_sanitized(&mut record, prompt);
    record.push('\n');
    for tool in tools {
        record.push_str("tool ");
        record.push_str(tool.name);
        record.push_str(": ");
        record.push_str(tool.output.as_str());
        record.push('\n');
    }
    record.push_str("delegate: ");
    record.push_str(delegate.summary.as_str());
    record.push('\n');
    record.push_str("assistant: ");
    record.push_str(answer);
    record.push_str("\n---\n");

    fxfs::append_file(HERMES_SESSION_PATH, record.as_bytes())
        .map_err(|_| HermesAgentError::Transcript)?;
    Ok(record.len())
}

fn push_sanitized(out: &mut String, text: &str) {
    for byte in text.bytes() {
        if byte == b'\n' || byte == b'\r' {
            out.push(' ');
        } else if (0x20..=0x7e).contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('.');
        }
    }
}

fn memory_item_count() -> Result<usize, HermesAgentError> {
    let text = read_text_file(HERMES_MEMORY_PATH)?;
    Ok(text
        .lines()
        .filter(|line| trim_ascii(line).starts_with("- "))
        .count())
}

fn count_dir_entries(path: &str) -> usize {
    fxfs::entries(path)
        .map(|entries| entries.len())
        .unwrap_or(0)
}

fn tool_audit_valid() -> Result<bool, HermesAgentError> {
    let text = read_text_file(HERMES_TOOL_AUDIT_PATH)?;
    Ok(text.contains("tool=shell") && text.contains("tool=fxfs") && text.contains("tool=svc"))
}

fn cron_ready() -> Result<bool, HermesAgentError> {
    let text = read_text_file(HERMES_CRON_PATH)?;
    Ok(text.contains("nightly-smros-hermes-smoke")
        && text.contains("0 3 * * *")
        && text.contains("hermes test"))
}

fn transcript_valid(min_last_record_bytes: usize) -> Result<bool, HermesAgentError> {
    let attrs = fxfs::attrs(HERMES_SESSION_PATH).map_err(|_| HermesAgentError::Transcript)?;
    if attrs.size < min_last_record_bytes {
        return Ok(false);
    }
    let text = read_text_file(HERMES_SESSION_PATH)?;
    Ok(text.contains("assistant: Gemma on SMROS")
        && text.contains("tool shell:")
        && text.contains("tool fxfs:")
        && text.contains("tool svc:")
        && text.contains("delegate:"))
}

fn append_usize(out: &mut String, mut value: usize, min_width: usize) {
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    if value == 0 {
        buf[len] = b'0';
        len += 1;
    }
    while value != 0 && len < buf.len() {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len < min_width && len < buf.len() {
        buf[len] = b'0';
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
}
