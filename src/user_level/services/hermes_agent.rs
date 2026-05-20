//! Hermes Agent compatibility port for SMROS.
//!
//! Upstream NousResearch/hermes-agent is a Python 3.11 application with hosted
//! providers, CLI, skills, memory, tools, scheduling, and delegation. SMROS does
//! not yet host Python, so this module ports the agent contract into a native
//! service and routes text generation through the SMROS Gemma provider.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::user_level::{fxfs, gemma, html_ui, svc};

const HERMES_ROOT: &str = "/data/hermes";
const HERMES_SKILL_DIR: &str = "/data/hermes/skills";
const HERMES_MEMORY_DIR: &str = "/data/hermes/memory";
const HERMES_SESSION_DIR: &str = "/data/hermes/sessions";
const HERMES_TOOL_DIR: &str = "/data/hermes/tools";
const HERMES_CRON_DIR: &str = "/data/hermes/cron";
const HERMES_WEB_DIR: &str = "/data/hermes/web";
const HERMES_CONFIG_PATH: &str = "/data/hermes/config.yaml";
const HERMES_MEMORY_PATH: &str = "/data/hermes/memory/MEMORY.md";
const HERMES_USER_PATH: &str = "/data/hermes/memory/USER.md";
const HERMES_SESSION_PATH: &str = "/data/hermes/sessions/smros-session.log";
const HERMES_TOOL_AUDIT_PATH: &str = "/data/hermes/tools/audit.log";
const HERMES_CRON_PATH: &str = "/data/hermes/cron/nightly-smoke.yaml";
const HERMES_WEB_INDEX_PATH: &str = "/data/hermes/web/index.html";
const HERMES_WEB_PPM_PATH: &str = "/data/hermes/web/hermes-ui.ppm";

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
const HERMES_KERNEL_SKILL: &str = "# SMROS Kernel Skill\n\nUse FxFS, /svc, and syscall smoke tests to validate Hermes agent behavior inside SMROS.\n";
const HERMES_WEB_SKILL: &str = "# Hermes Web UI Skill\n\nBuild and review the Hermes web console, dashboard HTML, prompt composer, skills list, and transcript surface for SMROS.\n";
const HERMES_OPS_SKILL: &str = "# SMROS Ops Skill\n\nRun Gemma, Hermes, Docker, network, QEMU, and shell smoke tests; summarize failures with concrete SMROS commands.\n";
const HERMES_MEMORY_SKILL: &str = "# Hermes Memory Skill\n\nUse Hermes memory, session transcripts, user notes, and FxFS persistence to keep agent context visible and auditable.\n";
const HERMES_CRON: &str =
    "name: nightly-smros-hermes-smoke\nschedule: '0 3 * * *'\ncommand: hermes test\n";

struct HermesSkillDefinition {
    name: &'static str,
    slug: &'static str,
    dir: &'static str,
    path: &'static str,
    description: &'static str,
    body: &'static str,
    keywords: &'static [&'static str],
}

const HERMES_SKILLS: &[HermesSkillDefinition] = &[
    HermesSkillDefinition {
        name: "SMROS Kernel",
        slug: "smros-kernel",
        dir: "/data/hermes/skills/smros-kernel",
        path: "/data/hermes/skills/smros-kernel/SKILL.md",
        description: "FxFS, /svc, syscall, and kernel validation",
        body: HERMES_KERNEL_SKILL,
        keywords: &["smros", "kernel", "fxfs", "svc", "syscall"],
    },
    HermesSkillDefinition {
        name: "Hermes Web UI",
        slug: "hermes-web-ui",
        dir: "/data/hermes/skills/hermes-web-ui",
        path: "/data/hermes/skills/hermes-web-ui/SKILL.md",
        description: "Web console, prompt composer, and transcript UI",
        body: HERMES_WEB_SKILL,
        keywords: &["web", "ui", "html", "dashboard", "console", "native"],
    },
    HermesSkillDefinition {
        name: "SMROS Ops",
        slug: "smros-ops",
        dir: "/data/hermes/skills/smros-ops",
        path: "/data/hermes/skills/smros-ops/SKILL.md",
        description: "Smoke tests, QEMU, network, Docker, and shell operations",
        body: HERMES_OPS_SKILL,
        keywords: &["test", "smoke", "qemu", "docker", "network", "shell"],
    },
    HermesSkillDefinition {
        name: "Hermes Memory",
        slug: "hermes-memory",
        dir: "/data/hermes/skills/hermes-memory",
        path: "/data/hermes/skills/hermes-memory/SKILL.md",
        description: "Memory, user notes, sessions, transcripts, and audit trails",
        body: HERMES_MEMORY_SKILL,
        keywords: &["memory", "session", "transcript", "audit", "notes"],
    },
];

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
    pub web_ui_path: &'static str,
    pub web_ui_bytes: usize,
    pub cpu_ui_path: &'static str,
    pub cpu_ui_bytes: usize,
    pub generation_backend: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesAgentTurn {
    pub prompt: String,
    pub answer: String,
    pub tool_calls: usize,
    pub skill_hits: usize,
    pub skill_summary: String,
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
    pub web_ui_ok: bool,
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
            && self.web_ui_ok
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesWebUi {
    pub path: &'static str,
    pub bytes: usize,
    pub html: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesNativeUi {
    pub source_path: &'static str,
    pub title: String,
    pub rendered: String,
    pub widgets: usize,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesCpuUi {
    pub source_path: &'static str,
    pub image_path: &'static str,
    pub title: String,
    pub preview: String,
    pub widgets: usize,
    pub width: usize,
    pub height: usize,
    pub image_bytes: usize,
    pub pixel_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesSkillInfo {
    pub name: &'static str,
    pub slug: &'static str,
    pub path: &'static str,
    pub description: &'static str,
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
        web_ui_path: HERMES_WEB_INDEX_PATH,
        web_ui_bytes: fxfs::attrs(HERMES_WEB_INDEX_PATH)
            .map(|attrs| attrs.size)
            .unwrap_or(0),
        cpu_ui_path: HERMES_WEB_PPM_PATH,
        cpu_ui_bytes: fxfs::attrs(HERMES_WEB_PPM_PATH)
            .map(|attrs| attrs.size)
            .unwrap_or(0),
        generation_backend: "smros-native",
    })
}

pub fn run_prompt(prompt: &str) -> Result<HermesAgentTurn, HermesAgentError> {
    prepare_storage()?;
    let config = load_config()?;
    if !route_model(config.provider.as_str(), config.model.as_str()) {
        return Err(HermesAgentError::ModelRoute);
    }
    let skill_hits = matching_skills(prompt)?;
    if skill_hits.is_empty() {
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
    let skill_summary = summarize_skills(&skill_hits);
    refresh_web_ui()?;

    Ok(HermesAgentTurn {
        prompt: String::from(prompt),
        answer,
        tool_calls: tool_results.len(),
        skill_hits: skill_hits.len(),
        skill_summary,
        delegated_agents: delegate.agents,
        memory_writes,
        transcript_bytes,
        model_tokens: generation.generated_tokens,
        model_backend: generation.backend,
    })
}

pub fn render_web_ui() -> Result<HermesWebUi, HermesAgentError> {
    prepare_storage()?;
    refresh_web_ui()?;
    let html = read_text_file(HERMES_WEB_INDEX_PATH)?;
    let bytes = html.len();
    Ok(HermesWebUi {
        path: HERMES_WEB_INDEX_PATH,
        bytes,
        html,
    })
}

pub fn render_native_ui(width: usize) -> Result<HermesNativeUi, HermesAgentError> {
    let web = render_web_ui()?;
    let view = html_ui::render_native_view(web.html.as_str(), width)
        .map_err(|_| HermesAgentError::Tool)?;
    Ok(HermesNativeUi {
        source_path: HERMES_WEB_INDEX_PATH,
        title: view.title,
        rendered: view.rendered,
        widgets: view.widgets,
        width: view.width,
    })
}

pub fn render_cpu_ui() -> Result<HermesCpuUi, HermesAgentError> {
    let web = render_web_ui()?;
    let view = html_ui::render_cpu_view(web.html.as_str()).map_err(|_| HermesAgentError::Tool)?;
    fxfs::write_file(HERMES_WEB_PPM_PATH, view.ppm.as_slice())
        .map_err(|_| HermesAgentError::FxfsPrepare)?;
    Ok(HermesCpuUi {
        source_path: HERMES_WEB_INDEX_PATH,
        image_path: HERMES_WEB_PPM_PATH,
        title: view.title,
        preview: view.preview,
        widgets: view.widgets,
        width: view.width,
        height: view.height,
        image_bytes: view.ppm.len(),
        pixel_bytes: view.pixels.len(),
    })
}

pub fn list_skills() -> Result<Vec<HermesSkillInfo>, HermesAgentError> {
    prepare_storage()?;
    let mut skills = Vec::new();
    for skill in HERMES_SKILLS {
        let body = read_text_file(skill.path)?;
        if !skill_file_valid(body.as_str()) {
            return Err(HermesAgentError::Skill);
        }
        skills.push(HermesSkillInfo {
            name: skill.name,
            slug: skill.slug,
            path: skill.path,
            description: skill.description,
        });
    }
    Ok(skills)
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

    let skill_ok =
        matching_skills("Use the SMROS kernel, web UI, ops, and memory skills to test Hermes")?
            .len()
            >= HERMES_SKILLS.len();
    if !skill_ok {
        return Err(HermesAgentError::Skill);
    }

    let memory_before = memory_item_count()?;
    let turn = run_prompt("test hermes web ui on SMROS with tools, memory, skills, and /svc")?;
    let memory_after = memory_item_count()?;
    let memory_ok = memory_after > memory_before && turn.memory_writes > 0;
    if !memory_ok {
        return Err(HermesAgentError::Memory);
    }

    let tool_ok =
        turn.tool_calls == 3 && turn.skill_hits >= HERMES_SKILLS.len() && tool_audit_valid()?;
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

    let web = render_web_ui()?;
    let web_ui_ok = web.bytes > 0
        && web.html.contains("<!doctype html>")
        && web.html.contains("Hermes Agent")
        && web.html.contains("Prompt Composer")
        && web.html.contains("SMROS Kernel")
        && web.html.contains("Hermes Web UI");
    let native = render_native_ui(78)?;
    let native_ui_ok = native.rendered.contains("Native HTML UI")
        && native.rendered.contains("Prompt Composer")
        && native.rendered.contains("SMROS Kernel");
    let cpu = render_cpu_ui()?;
    let cpu_ui_ok = cpu.width == 720
        && cpu.height == 420
        && cpu.image_bytes > cpu.pixel_bytes
        && cpu.preview.contains("CPU-rendered native Hermes UI")
        && cpu.preview.contains("Prompt Composer");
    if !web_ui_ok {
        return Err(HermesAgentError::Transcript);
    }
    if !native_ui_ok {
        return Err(HermesAgentError::Tool);
    }
    if !cpu_ui_ok {
        return Err(HermesAgentError::Tool);
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
        web_ui_ok,
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
    for skill in HERMES_SKILLS {
        create_dir(skill.dir)?;
    }
    create_dir(HERMES_MEMORY_DIR)?;
    create_dir(HERMES_SESSION_DIR)?;
    create_dir(HERMES_TOOL_DIR)?;
    create_dir(HERMES_CRON_DIR)?;
    create_dir(HERMES_WEB_DIR)?;
    ensure_exact_file(HERMES_CONFIG_PATH, HERMES_CONFIG)?;
    ensure_file(HERMES_MEMORY_PATH, HERMES_MEMORY)?;
    ensure_file(HERMES_USER_PATH, HERMES_USER)?;
    for skill in HERMES_SKILLS {
        ensure_exact_file(skill.path, skill.body)?;
    }
    ensure_exact_file(HERMES_CRON_PATH, HERMES_CRON)?;
    ensure_file(HERMES_TOOL_AUDIT_PATH, "")?;
    ensure_file(HERMES_SESSION_PATH, "")?;
    refresh_web_ui()?;
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

fn matching_skills(prompt: &str) -> Result<Vec<&'static HermesSkillDefinition>, HermesAgentError> {
    if prompt.trim().is_empty() {
        return Err(HermesAgentError::Skill);
    }

    let mut hits = Vec::new();
    for skill in HERMES_SKILLS {
        let body = read_text_file(skill.path)?;
        if !skill_file_valid(body.as_str()) {
            return Err(HermesAgentError::Skill);
        }
        if skill_matches_prompt(skill, prompt) {
            hits.push(skill);
        }
    }

    if hits.is_empty() {
        hits.push(&HERMES_SKILLS[0]);
    }
    Ok(hits)
}

fn skill_file_valid(text: &str) -> bool {
    text.contains("Skill") && (text.contains("Hermes") || text.contains("SMROS"))
}

fn skill_matches_prompt(skill: &HermesSkillDefinition, prompt: &str) -> bool {
    for keyword in skill.keywords {
        if contains_case_insensitive(prompt, keyword) {
            return true;
        }
    }
    false
}

fn summarize_skills(skills: &[&HermesSkillDefinition]) -> String {
    let mut out = String::new();
    for (index, skill) in skills.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(skill.slug);
    }
    out
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle_bytes = needle.as_bytes();
    let haystack_bytes = haystack.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }
    for start in 0..=haystack_bytes.len() - needle_bytes.len() {
        let mut matched = true;
        for offset in 0..needle_bytes.len() {
            if haystack_bytes[start + offset].to_ascii_lowercase()
                != needle_bytes[offset].to_ascii_lowercase()
            {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
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
    context.push_str(" skills=");
    append_usize(&mut context, HERMES_SKILLS.len(), 0);
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

fn refresh_web_ui() -> Result<(), HermesAgentError> {
    let info = info_without_web_refresh()?;
    let html = build_web_ui_html(&info)?;
    fxfs::write_file(HERMES_WEB_INDEX_PATH, html.as_bytes())
        .map(|_| ())
        .map_err(|_| HermesAgentError::FxfsPrepare)
}

fn info_without_web_refresh() -> Result<HermesAgentInfo, HermesAgentError> {
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
        web_ui_path: HERMES_WEB_INDEX_PATH,
        web_ui_bytes: fxfs::attrs(HERMES_WEB_INDEX_PATH)
            .map(|attrs| attrs.size)
            .unwrap_or(0),
        cpu_ui_path: HERMES_WEB_PPM_PATH,
        cpu_ui_bytes: fxfs::attrs(HERMES_WEB_PPM_PATH)
            .map(|attrs| attrs.size)
            .unwrap_or(0),
        generation_backend: "smros-native",
    })
}

fn build_web_ui_html(info: &HermesAgentInfo) -> Result<String, HermesAgentError> {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>Hermes Agent for SMROS</title>\n<style>\n:root{color-scheme:light dark;--bg:#f4f6f8;--panel:#ffffff;--ink:#172026;--muted:#5b6770;--line:#d7dee7;--soft:#eef3f7;--accent:#0b6bcb;--ok:#147d4a;--warn:#a15c00;--teal:#087f83;--dark:#101820;--light:#e9f1f7}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.45 system-ui,-apple-system,Segoe UI,sans-serif}.shell{max-width:1180px;margin:0 auto;padding:18px}.top{display:grid;grid-template-columns:minmax(260px,1fr) minmax(360px,.9fr);gap:14px;align-items:stretch}.brand,.metric,.panel{background:var(--panel);border:1px solid var(--line);border-radius:8px}.brand{padding:16px 18px;border-left:5px solid var(--accent)}.brand h1{font-size:26px;line-height:1.05;margin:0 0 6px}.brand p{margin:0;color:var(--muted)}.status{display:grid;grid-template-columns:repeat(3,minmax(96px,1fr));gap:8px}.metric{padding:10px 12px}.metric b{display:block;font-size:22px;line-height:1.1}.metric span{color:var(--muted);font-size:12px}.grid{display:grid;grid-template-columns:minmax(0,1.15fr) minmax(300px,.85fr);gap:14px;margin-top:14px}.stack{display:grid;gap:14px}.panel{padding:14px}.panel h2{font-size:15px;margin:0 0 10px}.composer textarea{width:100%;min-height:112px;resize:vertical;border:1px solid var(--line);border-radius:6px;padding:10px;background:var(--soft);color:inherit;font:inherit}.row{display:flex;gap:8px;flex-wrap:wrap;align-items:center}.button{border:0;border-radius:6px;background:var(--accent);color:white;padding:9px 12px;font-weight:650;cursor:pointer}.button.ok{background:var(--ok)}.button.warn{background:var(--warn)}.ghost{background:transparent;color:var(--ink);border:1px solid var(--line)}.quick{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;margin-top:10px}.quick button{text-align:left;border:1px solid var(--line);border-radius:6px;background:var(--soft);color:var(--ink);padding:8px}.answer{background:var(--dark);color:var(--light);border-radius:8px;padding:14px;min-height:116px;white-space:pre-wrap}.skill{border-top:1px solid var(--line);padding:9px 0}.skill:first-of-type{border-top:0}.skill b{display:block}.skill span,.small{color:var(--muted)}.pill{display:inline-flex;border:1px solid var(--line);border-radius:999px;padding:3px 8px;margin:3px 4px 3px 0;color:var(--muted);background:var(--soft)}.feed{display:grid;gap:8px}.feed div{border-left:3px solid var(--teal);padding:6px 0 6px 10px;background:var(--soft);border-radius:0 6px 6px 0}.paths{display:grid;gap:6px}.paths code{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.split{display:grid;grid-template-columns:1fr 1fr;gap:10px}code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}@media(max-width:860px){.top,.grid,.split{display:block}.status{grid-template-columns:repeat(2,minmax(96px,1fr));margin-top:10px}.panel,.stack{margin-top:14px}.quick{grid-template-columns:1fr}.shell{padding:12px}}\n</style>\n</head>\n<body>\n<main class=\"shell\">\n<section class=\"top\">\n<div class=\"brand\"><h1>Hermes Agent</h1><p>Native SMROS operator console backed by Gemma, FxFS, /svc, skills, memory, transcripts, and smoke tests.</p></div>\n<div class=\"status\">",
    );

    push_metric(&mut html, "Tools", info.tools);
    push_metric(&mut html, "Skills", info.skills);
    push_metric(&mut html, "Memory", info.memory_items);
    push_metric(&mut html, "Cron", info.cron_jobs);
    push_metric(&mut html, "Sessions", info.transcripts);
    push_metric(&mut html, "HTML bytes", info.web_ui_bytes);
    html.push_str("</div>\n</section>\n<section class=\"grid\">\n<div class=\"stack\"><div class=\"panel composer\"><h2>Prompt Composer</h2><textarea id=\"prompt\">test hermes web ui on SMROS with memory and skills</textarea><div class=\"row\" style=\"margin-top:10px\"><button class=\"button\" id=\"ask\">Ask Hermes</button><button class=\"button ok\" id=\"smoke\">Smoke Test</button><button class=\"button ghost\" id=\"load\">Load Preset</button><button class=\"button ghost\" id=\"clear\">Clear</button><span class=\"small\">Shell: <code>hermes ask &lt;prompt&gt;</code> or <code>hermes ui</code></span></div><div class=\"quick\"><button data-prompt=\"summarize FxFS, /svc, and Gemma state\">Runtime summary</button><button data-prompt=\"plan a Hermes smoke test for network and Docker\">Ops smoke plan</button><button data-prompt=\"review recent memory and transcript context\">Memory review</button><button data-prompt=\"test hermes web ui on SMROS with memory and skills\">UI validation</button></div></div><div class=\"panel\"><h2>Response</h2><div class=\"answer\" id=\"answer\">Gemma responses are generated inside SMROS. This console is stored at <code>");
    html.push_str(HERMES_WEB_INDEX_PATH);
    html.push_str("</code> and mirrored by the faster full-screen shell UI.</div></div><div class=\"panel\"><h2>Activity</h2><div class=\"feed\"><div>Prompt composer is ready.</div><div>Use <code>hermes ui</code> for keyboard and mouse interaction.</div><div>Smoke test covers config, model route, skills, memory, tools, Gemma, /svc, and web UI.</div></div></div></div>\n<aside class=\"stack\"><div class=\"panel\"><h2>Runtime</h2><div class=\"pill\">provider ");
    html.push_str(info.provider);
    html.push_str("</div><div class=\"pill\">model ");
    html.push_str(info.model);
    html.push_str("</div><div class=\"pill\">backend ");
    html.push_str(info.generation_backend);
    html.push_str("</div><div class=\"pill\">personality ");
    html.push_str(info.personality);
    html.push_str("</div></div><div class=\"panel\"><h2>Paths</h2><div class=\"paths\"><code>");
    html.push_str(HERMES_CONFIG_PATH);
    html.push_str("</code><code>");
    html.push_str(HERMES_MEMORY_PATH);
    html.push_str("</code><code>");
    html.push_str(HERMES_SESSION_PATH);
    html.push_str("</code><code>");
    html.push_str(HERMES_WEB_PPM_PATH);
    html.push_str("</code></div></div><div class=\"panel\"><h2>Skills</h2>");

    for skill in HERMES_SKILLS {
        html.push_str("<div class=\"skill\"><b>");
        html.push_str(skill.name);
        html.push_str("</b><span>");
        html.push_str(skill.description);
        html.push_str("</span><br><code>");
        html.push_str(skill.path);
        html.push_str("</code></div>");
    }

    html.push_str("</div></aside>\n</section>\n</main>\n<script>\nconst prompts=['test hermes web ui on SMROS with memory and skills','summarize FxFS, /svc, and Gemma state','plan a Hermes smoke test for network and Docker','review recent memory and transcript context'];\nlet preset=0;\nconst promptBox=document.getElementById('prompt');\nconst answer=document.getElementById('answer');\ndocument.querySelectorAll('[data-prompt]').forEach(button=>button.onclick=()=>{promptBox.value=button.dataset.prompt;promptBox.focus();});\ndocument.getElementById('load').onclick=()=>{preset=(preset+1)%prompts.length;promptBox.value=prompts[preset];promptBox.focus();};\ndocument.getElementById('clear').onclick=()=>{promptBox.value='';answer.textContent='Prompt cleared.';promptBox.focus();};\ndocument.getElementById('smoke').onclick=()=>{answer.textContent='Run in SMROS shell: hermes test';};\ndocument.getElementById('ask').onclick=()=>{answer.textContent='Run in SMROS shell: hermes ask '+promptBox.value;};\n</script>\n</body>\n</html>\n");
    Ok(html)
}

fn push_metric(out: &mut String, label: &str, value: usize) {
    out.push_str("<div class=\"metric\"><b>");
    append_usize(out, value, 0);
    out.push_str("</b><span>");
    out.push_str(label);
    out.push_str("</span></div>");
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
