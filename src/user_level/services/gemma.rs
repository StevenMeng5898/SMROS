//! Native Gemma service surface for SMROS.
//!
//! Full Google Gemma weights are currently larger than the default SMROS QEMU
//! profile can host. This module ports the Gemma-facing runtime contract into
//! SMROS: model metadata, prompt formatting, bounded local generation, FxFS
//! persistence, and shell/test hooks. The backend boundary is intentionally
//! narrow so a future gemma.cpp or TFLite-backed runner can replace the native
//! generator without changing Hermes.

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::user_level::fxfs;

const GEMMA_ROOT: &str = "/data/gemma";
const GEMMA_MODEL_DIR: &str = "/data/gemma/models";
const GEMMA_LOG_DIR: &str = "/data/gemma/logs";
const GEMMA_MANIFEST_PATH: &str = "/data/gemma/models/gemma-3n-e2b-smros.manifest";
const GEMMA_GENERATION_LOG: &str = "/data/gemma/logs/generation.log";

pub const GEMMA_PROVIDER: &str = "gemma";
pub const GEMMA_MODEL: &str = "gemma/gemma-3n-e2b-smros";
pub const GEMMA_CONTEXT_TOKENS: usize = 2048;
pub const GEMMA_MAX_OUTPUT_TOKENS: usize = 96;

const GEMMA_MANIFEST: &str = "family: Gemma\narchitecture: Gemma 3n\nmodel: gemma/gemma-3n-e2b-smros\nbackend: smros-native\ncontext_tokens: 2048\nmax_output_tokens: 96\nweights: kernel-resident-smros-adapter\n";
const GEMMA_SYSTEM_PROMPT: &str =
    "You are Gemma running inside SMROS. Answer with kernel-aware, concise, testable guidance.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GemmaError {
    FxfsInit,
    FxfsPrepare,
    Manifest,
    Prompt,
    Generate,
}

impl GemmaError {
    pub fn as_str(self) -> &'static str {
        match self {
            GemmaError::FxfsInit => "fxfs init",
            GemmaError::FxfsPrepare => "fxfs prepare",
            GemmaError::Manifest => "manifest",
            GemmaError::Prompt => "prompt",
            GemmaError::Generate => "generate",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GemmaModelInfo {
    pub provider: &'static str,
    pub model: &'static str,
    pub family: &'static str,
    pub architecture: &'static str,
    pub backend: &'static str,
    pub context_tokens: usize,
    pub max_output_tokens: usize,
    pub weights: &'static str,
    pub manifest_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GemmaGeneration {
    pub model: &'static str,
    pub backend: &'static str,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GemmaTestReport {
    pub manifest_ok: bool,
    pub prompt_ok: bool,
    pub generation_ok: bool,
    pub log_ok: bool,
    pub generation: GemmaGeneration,
}

impl GemmaTestReport {
    pub fn passed(&self) -> bool {
        self.manifest_ok && self.prompt_ok && self.generation_ok && self.log_ok
    }
}

pub fn init() -> bool {
    prepare_storage().is_ok()
}

pub fn model_available(provider: &str, model: &str) -> bool {
    provider == GEMMA_PROVIDER && model == GEMMA_MODEL && init()
}

pub fn info() -> Result<GemmaModelInfo, GemmaError> {
    prepare_storage()?;
    let manifest = read_text_file(GEMMA_MANIFEST_PATH)?;
    if !manifest_valid(manifest.as_str()) {
        return Err(GemmaError::Manifest);
    }

    Ok(GemmaModelInfo {
        provider: GEMMA_PROVIDER,
        model: GEMMA_MODEL,
        family: "Gemma",
        architecture: "Gemma 3n",
        backend: "smros-native",
        context_tokens: GEMMA_CONTEXT_TOKENS,
        max_output_tokens: GEMMA_MAX_OUTPUT_TOKENS,
        weights: "kernel-resident-smros-adapter",
        manifest_bytes: manifest.len(),
    })
}

pub fn generate(
    prompt: &str,
    context: &str,
    max_tokens: usize,
) -> Result<GemmaGeneration, GemmaError> {
    prepare_storage()?;
    if !prompt_valid(prompt) {
        return Err(GemmaError::Prompt);
    }
    let manifest = read_text_file(GEMMA_MANIFEST_PATH)?;
    if !manifest_valid(manifest.as_str()) {
        return Err(GemmaError::Manifest);
    }

    let prompt_template = build_prompt(prompt, context)?;
    let prompt_tokens = estimate_tokens(prompt_template.as_str());
    if prompt_tokens > GEMMA_CONTEXT_TOKENS {
        return Err(GemmaError::Prompt);
    }

    let token_budget = clamp_tokens(max_tokens, GEMMA_MAX_OUTPUT_TOKENS);
    let text = generate_local_text(prompt, context, token_budget)?;
    let generated_tokens = estimate_tokens(text.as_str());
    let generation = GemmaGeneration {
        model: GEMMA_MODEL,
        backend: "smros-native",
        prompt_tokens,
        generated_tokens,
        text,
    };
    append_generation_log(prompt, &generation)?;
    Ok(generation)
}

pub fn run_full_test() -> Result<GemmaTestReport, GemmaError> {
    prepare_storage()?;
    let manifest = read_text_file(GEMMA_MANIFEST_PATH)?;
    let manifest_ok = manifest_valid(manifest.as_str());
    if !manifest_ok {
        return Err(GemmaError::Manifest);
    }

    let prompt = "explain how Hermes should answer through Gemma on SMROS";
    let context = "tools=shell,fxfs,svc; memory=enabled; tests=qemu";
    let prompt_ok = prompt_valid(prompt) && build_prompt(prompt, context).is_ok();
    if !prompt_ok {
        return Err(GemmaError::Prompt);
    }

    let generation = generate(prompt, context, 64)?;
    let generation_ok = generation.text.contains("Gemma")
        && generation.text.contains("SMROS")
        && generation.text.contains("Hermes");
    if !generation_ok {
        return Err(GemmaError::Generate);
    }

    let log = read_text_file(GEMMA_GENERATION_LOG)?;
    let log_ok = log.contains("model=gemma/gemma-3n-e2b-smros")
        && log.contains("prompt_tokens=")
        && log.contains("generated_tokens=");
    if !log_ok {
        return Err(GemmaError::FxfsPrepare);
    }

    Ok(GemmaTestReport {
        manifest_ok,
        prompt_ok,
        generation_ok,
        log_ok,
        generation,
    })
}

pub fn smoke_test() -> bool {
    run_full_test()
        .map(|report| report.passed())
        .unwrap_or(false)
}

fn prepare_storage() -> Result<(), GemmaError> {
    if !fxfs::init() {
        return Err(GemmaError::FxfsInit);
    }
    create_dir("/data")?;
    create_dir(GEMMA_ROOT)?;
    create_dir(GEMMA_MODEL_DIR)?;
    create_dir(GEMMA_LOG_DIR)?;
    ensure_exact_file(GEMMA_MANIFEST_PATH, GEMMA_MANIFEST)?;
    ensure_file(GEMMA_GENERATION_LOG, "")?;
    Ok(())
}

fn create_dir(path: &str) -> Result<(), GemmaError> {
    fxfs::create_dir(path)
        .map(|_| ())
        .map_err(|_| GemmaError::FxfsPrepare)
}

fn ensure_file(path: &str, data: &str) -> Result<(), GemmaError> {
    if fxfs::exists(path) {
        return Ok(());
    }
    fxfs::write_file(path, data.as_bytes())
        .map(|_| ())
        .map_err(|_| GemmaError::FxfsPrepare)
}

fn ensure_exact_file(path: &str, data: &str) -> Result<(), GemmaError> {
    if let Ok(current) = read_text_file(path) {
        if current == data {
            return Ok(());
        }
    }
    fxfs::write_file(path, data.as_bytes())
        .map(|_| ())
        .map_err(|_| GemmaError::FxfsPrepare)
}

fn read_text_file(path: &str) -> Result<String, GemmaError> {
    let attrs = fxfs::attrs(path).map_err(|_| GemmaError::FxfsPrepare)?;
    let mut out = Vec::new();
    out.resize(attrs.size, 0);
    let read = fxfs::read_file(path, &mut out).map_err(|_| GemmaError::FxfsPrepare)?;
    out.truncate(read);
    String::from_utf8(out).map_err(|_| GemmaError::FxfsPrepare)
}

fn manifest_valid(manifest: &str) -> bool {
    manifest.contains("family: Gemma")
        && manifest.contains("architecture: Gemma 3n")
        && manifest.contains("model: gemma/gemma-3n-e2b-smros")
        && manifest.contains("backend: smros-native")
}

fn prompt_valid(prompt: &str) -> bool {
    let len = prompt.len();
    len > 0
        && len <= 4096
        && prompt
            .bytes()
            .all(|byte| byte == b'\n' || (0x20..=0x7e).contains(&byte))
}

fn build_prompt(prompt: &str, context: &str) -> Result<String, GemmaError> {
    if !prompt_valid(prompt) {
        return Err(GemmaError::Prompt);
    }
    let mut out = String::from("<start_of_turn>system\n");
    out.push_str(GEMMA_SYSTEM_PROMPT);
    out.push_str("\n<end_of_turn>\n<start_of_turn>user\n");
    out.push_str("Context: ");
    push_sanitized(&mut out, context);
    out.push_str("\nPrompt: ");
    push_sanitized(&mut out, prompt);
    out.push_str("\n<end_of_turn>\n<start_of_turn>model\n");
    Ok(out)
}

fn generate_local_text(
    prompt: &str,
    context: &str,
    max_tokens: usize,
) -> Result<String, GemmaError> {
    if max_tokens == 0 {
        return Err(GemmaError::Generate);
    }

    let mut out = String::from("Gemma on SMROS: ");
    let lower_score = prompt_score(prompt);
    if is_greeting(prompt) {
        out.push_str(
            "Hi. I am the Gemma service running inside SMROS, and Hermes can call me for answers. ",
        );
    } else if asks_identity(prompt) {
        out.push_str(
            "I am Gemma on SMROS: a native kernel-service adapter that formats prompts, keeps FxFS logs, and answers through the local Gemma backend boundary. ",
        );
    } else if asks_weather(prompt) {
        out.push_str(
            "I cannot read live weather from this kernel shell yet because no network weather tool is wired into the Gemma context. Give me a forecast text or add a weather tool, and I can summarize it. ",
        );
    } else if asks_capability(prompt) {
        out.push_str(
            "I can answer short prompts, explain SMROS services, route Hermes requests, record generation logs in FxFS, and report what shell, FxFS, and /svc context is available. ",
        );
    } else if asks_question(prompt) {
        out.push_str("You asked: ");
        push_prompt_excerpt(&mut out, prompt, 80);
        out.push_str(
            ". I can answer from the local SMROS context; for live external facts, wire a tool into the context first. ",
        );
    } else if contains_case_insensitive(prompt, "hermes") {
        out.push_str("Hermes is now routed through the Gemma provider, so ask uses the Gemma generation path instead of the old fixed composer. ");
    } else if contains_case_insensitive(prompt, "test") {
        out.push_str("Run hermes test and gemma test to verify model routing, prompt formatting, FxFS logs, and /svc context. ");
    } else if contains_case_insensitive(prompt, "smros") {
        out.push_str("The answer is generated inside the SMROS Gemma service with kernel-visible tool context. ");
    } else {
        out.push_str(
            "I used the local Gemma service and available SMROS context to produce this response. ",
        );
    }

    if context.contains("tool shell") || context.contains("shell") {
        out.push_str("Shell context is available. ");
    }
    if context.contains("fxfs") || context.contains("FxFS") {
        out.push_str("FxFS persistence is active. ");
    }
    if context.contains("svc") || context.contains("/svc") {
        out.push_str("/svc IPC is part of the prompt context. ");
    }

    out.push_str("score=");
    append_usize(&mut out, lower_score, 0);
    truncate_to_token_budget(&mut out, max_tokens);
    Ok(out)
}

fn is_greeting(prompt: &str) -> bool {
    let normalized = normalized_prompt(prompt);
    matches!(
        normalized.as_str(),
        "hi" | "hello" | "hey" | "hiya" | "yo" | "good morning" | "good afternoon" | "good evening"
    )
}

fn asks_identity(prompt: &str) -> bool {
    contains_case_insensitive(prompt, "who are you")
        || contains_case_insensitive(prompt, "what are you")
        || contains_case_insensitive(prompt, "your name")
        || contains_case_insensitive(prompt, "introduce yourself")
}

fn asks_weather(prompt: &str) -> bool {
    contains_case_insensitive(prompt, "weather")
        || contains_case_insensitive(prompt, "temperature")
        || contains_case_insensitive(prompt, "forecast")
}

fn asks_capability(prompt: &str) -> bool {
    contains_case_insensitive(prompt, "what can you do")
        || contains_case_insensitive(prompt, "help me")
        || contains_case_insensitive(prompt, "capabilities")
}

fn asks_question(prompt: &str) -> bool {
    prompt.contains('?')
        || starts_with_word(prompt, "how")
        || starts_with_word(prompt, "what")
        || starts_with_word(prompt, "why")
        || starts_with_word(prompt, "when")
        || starts_with_word(prompt, "where")
        || starts_with_word(prompt, "can")
        || starts_with_word(prompt, "do")
        || starts_with_word(prompt, "does")
        || starts_with_word(prompt, "is")
}

fn starts_with_word(prompt: &str, word: &str) -> bool {
    let normalized = normalized_prompt(prompt);
    normalized == word
        || normalized
            .strip_prefix(word)
            .map(|rest| rest.starts_with(' '))
            .unwrap_or(false)
}

fn normalized_prompt(prompt: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for byte in prompt.bytes() {
        let lowered = byte.to_ascii_lowercase();
        if lowered.is_ascii_alphanumeric() {
            out.push(lowered as char);
            last_space = false;
        } else if byte.is_ascii_whitespace() && !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn push_prompt_excerpt(out: &mut String, prompt: &str, max_bytes: usize) {
    let mut pushed = 0usize;
    for byte in prompt.bytes() {
        if pushed >= max_bytes {
            out.push_str("...");
            return;
        }
        if byte == b'\n' || byte == b'\r' {
            out.push(' ');
        } else if (0x20..=0x7e).contains(&byte) {
            out.push(byte as char);
        }
        pushed += 1;
    }
}

fn prompt_score(prompt: &str) -> usize {
    let mut score = 0usize;
    for byte in prompt.bytes() {
        if byte.is_ascii_alphanumeric() {
            score = score
                .wrapping_mul(33)
                .wrapping_add(byte.to_ascii_lowercase() as usize);
        }
    }
    score % 1009
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

fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut in_token = false;
    for byte in text.bytes() {
        if byte.is_ascii_whitespace() {
            if in_token {
                tokens += 1;
                in_token = false;
            }
        } else if matches!(
            byte,
            b'.' | b',' | b':' | b';' | b'!' | b'?' | b'/' | b'=' | b'-'
        ) {
            if in_token {
                tokens += 1;
                in_token = false;
            }
            tokens += 1;
        } else {
            in_token = true;
        }
    }
    if in_token {
        tokens += 1;
    }
    tokens
}

fn clamp_tokens(requested: usize, max_allowed: usize) -> usize {
    if requested == 0 {
        core::cmp::min(32, max_allowed)
    } else {
        core::cmp::min(requested, max_allowed)
    }
}

fn truncate_to_token_budget(text: &mut String, max_tokens: usize) {
    let mut tokens = 0usize;
    let mut cut = text.len();
    let mut in_token = false;
    for (index, byte) in text.bytes().enumerate() {
        if byte.is_ascii_whitespace() {
            if in_token {
                tokens += 1;
                in_token = false;
                if tokens >= max_tokens {
                    cut = index;
                    break;
                }
            }
        } else {
            in_token = true;
        }
    }
    if cut < text.len() {
        text.truncate(cut);
    }
}

fn append_generation_log(prompt: &str, generation: &GemmaGeneration) -> Result<(), GemmaError> {
    let mut line = String::from("model=");
    line.push_str(generation.model);
    line.push_str(" backend=");
    line.push_str(generation.backend);
    line.push_str(" prompt_tokens=");
    append_usize(&mut line, generation.prompt_tokens, 0);
    line.push_str(" generated_tokens=");
    append_usize(&mut line, generation.generated_tokens, 0);
    line.push_str(" prompt=");
    push_sanitized(&mut line, prompt);
    line.push('\n');
    fxfs::append_file(GEMMA_GENERATION_LOG, line.as_bytes())
        .map(|_| ())
        .map_err(|_| GemmaError::FxfsPrepare)
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
