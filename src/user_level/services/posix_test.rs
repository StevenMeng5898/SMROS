//! Bounded guest-side parser for the host-generated POSIX test manifest.

#![allow(dead_code)]

use crate::alloc::collections::BTreeSet;
use crate::alloc::string::String;
use crate::alloc::vec::Vec;

#[cfg(not(test))]
use core as core_compat;
#[cfg(not(test))]
use core::cell::UnsafeCell;
#[cfg(test)]
use std as core_compat;

use super::posix_test_logic_shared::{PosixCoverageResult, PosixCoverageSnapshot};
use super::{fxfs, posix_test_logic_shared};

#[cfg(not(test))]
use super::posix_test_logic_shared::PosixCoverageTracker;

#[cfg(not(test))]
use super::run_elf::{self, RunObserver, RunOutcome, RunTermination};

#[cfg(not(test))]
use crate::syscall::PosixResourceSnapshot;

pub const POSIX_MANIFEST_PATH: &str = "/shared/posixtest/manifest.tsv";
pub const POSIX_MANIFEST_SCHEMA: u32 = 1;
pub const POSIX_MANIFEST_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const POSIX_MANIFEST_MAX_TESTS: usize = 4_096;
pub const POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES: usize = 1_024;
pub const POSIX_MANIFEST_MAX_TEST_ID_BYTES: usize = 256;
pub const POSIX_MANIFEST_MAX_GROUP_BYTES: usize = 96;
pub const POSIX_MANIFEST_MAX_API_BYTES: usize = 96;
pub const POSIX_MANIFEST_MAX_STAGED_PATH_BYTES: usize = 512;
pub const POSIX_FILTER_MAX_BYTES: usize = 256;
pub const POSIX_EVENT_PREFIX: &str = "SMROS_POSIX_EVENT ";
pub const POSIX_EVENT_SCHEMA: u32 = 1;

const POSIX_COMPAT_PRELOAD_ENV: &str = "LD_PRELOAD=/shared/posixtest/lib/libsmros-posix-compat.so";
const POSIX_COMPAT_DIAG_ENV: &str = "SMROS_PTHREAD_DIAG=1";
const MAX_TIMEOUT_MS: u32 = i32::MAX as u32;
const EMPTY_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MANIFEST_HEADER: &str = "SMROS_POSIX_MANIFEST\t1";
const POSIX_EVENT_ARCHITECTURE: &str = "aarch64";
const METADATA_KEYS: [&str; 9] = [
    "source",
    "revision",
    "architecture",
    "compiler",
    "libc",
    "patch_sha256",
    "build_results_sha256",
    "manifest_sha256",
    "smros_commit",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixTestError {
    FxfsPrepare,
    FxfsRead,
    ManifestTooLarge,
    InvalidUtf8,
    InvalidLineEndings,
    InvalidHeader,
    UnknownRowType,
    InvalidMetadataRow,
    UnknownMetadata,
    MissingMetadata,
    DuplicateMetadata,
    MetadataOutOfOrder,
    InvalidTestRow,
    TooManyTests,
    DuplicateTestId,
    DuplicateTestPath,
    InvalidAtom,
    InvalidPath,
    UnknownKind,
    UnknownDisposition,
    InvalidKindDisposition,
    InvalidChecksum,
    InvalidTimeout,
    InvalidProvenance,
    NonCanonicalManifest,
    ManifestChecksumMismatch,
    InvalidFilter,
    AlreadyRunning,
    EmptySelection,
    LaunchError,
    InfrastructureError,
}

impl PosixTestError {
    fn as_str(self) -> &'static str {
        match self {
            PosixTestError::FxfsPrepare => "host-share-prepare",
            PosixTestError::FxfsRead => "manifest-read",
            PosixTestError::ManifestTooLarge => "manifest-too-large",
            PosixTestError::InvalidUtf8 => "invalid-utf8",
            PosixTestError::InvalidLineEndings => "invalid-line-endings",
            PosixTestError::InvalidHeader => "invalid-header",
            PosixTestError::UnknownRowType => "unknown-row-type",
            PosixTestError::InvalidMetadataRow => "invalid-metadata-row",
            PosixTestError::UnknownMetadata => "unknown-metadata",
            PosixTestError::MissingMetadata => "missing-metadata",
            PosixTestError::DuplicateMetadata => "duplicate-metadata",
            PosixTestError::MetadataOutOfOrder => "metadata-out-of-order",
            PosixTestError::InvalidTestRow => "invalid-test-row",
            PosixTestError::TooManyTests => "too-many-tests",
            PosixTestError::DuplicateTestId => "duplicate-test-id",
            PosixTestError::DuplicateTestPath => "duplicate-test-path",
            PosixTestError::InvalidAtom => "invalid-atom",
            PosixTestError::InvalidPath => "invalid-path",
            PosixTestError::UnknownKind => "unknown-kind",
            PosixTestError::UnknownDisposition => "unknown-disposition",
            PosixTestError::InvalidKindDisposition => "invalid-kind-disposition",
            PosixTestError::InvalidChecksum => "invalid-checksum",
            PosixTestError::InvalidTimeout => "invalid-timeout",
            PosixTestError::InvalidProvenance => "invalid-provenance",
            PosixTestError::NonCanonicalManifest => "non-canonical-manifest",
            PosixTestError::ManifestChecksumMismatch => "manifest-checksum-mismatch",
            PosixTestError::InvalidFilter => "invalid-filter",
            PosixTestError::AlreadyRunning => "already-running",
            PosixTestError::EmptySelection => "empty-selection",
            PosixTestError::LaunchError => "launch-error",
            PosixTestError::InfrastructureError => "infrastructure-error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PosixLaunchLoopResult {
    Running(usize),
    Completed(usize),
    InfrastructureError(usize),
}

fn start_result_after_launch(result: PosixLaunchLoopResult) -> Result<(), PosixTestError> {
    match result {
        PosixLaunchLoopResult::Running(_) | PosixLaunchLoopResult::Completed(0) => Ok(()),
        PosixLaunchLoopResult::Completed(_) => Err(PosixTestError::LaunchError),
        PosixLaunchLoopResult::InfrastructureError(_) => Err(PosixTestError::InfrastructureError),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PosixFilter {
    All,
    Group(String),
    Api(String),
    Test(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixTestKind {
    Runnable,
    Definition,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixDisposition {
    Complete,
    DefinitionOnly,
    ExcludedUpstreamStub,
    CompileFailed,
    LinkFailed,
    NotBuiltShellTest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixManifestMetadata {
    pub source: String,
    pub revision: String,
    pub architecture: String,
    pub compiler: String,
    pub libc: String,
    pub patch_sha256: String,
    pub build_results_sha256: String,
    pub manifest_sha256: String,
    pub smros_commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixManifestTest {
    pub test_id: String,
    pub group: String,
    pub api: String,
    pub kind: PosixTestKind,
    pub disposition: PosixDisposition,
    pub binary_path: Option<String>,
    pub timeout_ms: u32,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixManifest {
    pub metadata: PosixManifestMetadata,
    pub tests: Vec<PosixManifestTest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosixRunnerStatus {
    pub running: bool,
    pub run_id: Option<String>,
    pub filter: Option<PosixFilter>,
    pub current_test: Option<String>,
    pub completed: usize,
    pub selected: usize,
    pub status_counts: PosixStatusCounts,
    pub coverage: PosixCoverageSnapshot,
}

pub type PosixStatusCounts = posix_test_logic_shared::PosixCoverageStatusCounts;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PosixRuntimeStatus {
    Pass,
    Fail,
    Unresolved,
    Unsupported,
    Untested,
    LaunchError,
}

impl PosixRuntimeStatus {
    fn as_str(self) -> &'static str {
        match self {
            PosixRuntimeStatus::Pass => "pass",
            PosixRuntimeStatus::Fail => "fail",
            PosixRuntimeStatus::Unresolved => "unresolved",
            PosixRuntimeStatus::Unsupported => "unsupported",
            PosixRuntimeStatus::Untested => "untested",
            PosixRuntimeStatus::LaunchError => "launch-error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedTestAction {
    Launch,
    EmitWithoutLaunch,
    Ignore,
}

fn test_matches_filter(test: &PosixManifestTest, filter: &PosixFilter) -> bool {
    let (kind, value) = match filter {
        PosixFilter::All => (posix_test_logic_shared::PosixFilterKind::All, ""),
        PosixFilter::Group(value) => (
            posix_test_logic_shared::PosixFilterKind::Group,
            value.as_str(),
        ),
        PosixFilter::Api(value) => (
            posix_test_logic_shared::PosixFilterKind::Api,
            value.as_str(),
        ),
        PosixFilter::Test(value) => (
            posix_test_logic_shared::PosixFilterKind::Test,
            value.as_str(),
        ),
    };
    posix_test_logic_shared::filter_matches(
        kind,
        value,
        test.test_id.as_str(),
        test.group.as_str(),
        test.api.as_str(),
        test.kind == PosixTestKind::Runnable,
        test.disposition == PosixDisposition::Complete,
    )
}

fn selected_test_action(test: &PosixManifestTest) -> SelectedTestAction {
    if test.disposition == PosixDisposition::ExcludedUpstreamStub {
        return SelectedTestAction::EmitWithoutLaunch;
    }
    match (test.kind, test.disposition) {
        (PosixTestKind::Runnable, PosixDisposition::Complete) => SelectedTestAction::Launch,
        (PosixTestKind::Definition, _) => SelectedTestAction::Ignore,
        _ => SelectedTestAction::Ignore,
    }
}

fn pts_status(exit_code: i32) -> PosixRuntimeStatus {
    match posix_test_logic_shared::pts_status(exit_code) {
        posix_test_logic_shared::POSIX_STATUS_PASS => PosixRuntimeStatus::Pass,
        posix_test_logic_shared::POSIX_STATUS_FAIL => PosixRuntimeStatus::Fail,
        posix_test_logic_shared::POSIX_STATUS_UNRESOLVED => PosixRuntimeStatus::Unresolved,
        posix_test_logic_shared::POSIX_STATUS_UNSUPPORTED => PosixRuntimeStatus::Unsupported,
        posix_test_logic_shared::POSIX_STATUS_UNTESTED => PosixRuntimeStatus::Untested,
        _ => PosixRuntimeStatus::Fail,
    }
}

fn coverage_result(status: PosixRuntimeStatus) -> PosixCoverageResult {
    match status {
        PosixRuntimeStatus::Pass => PosixCoverageResult::Pass,
        PosixRuntimeStatus::Fail => PosixCoverageResult::Fail,
        PosixRuntimeStatus::Unresolved => PosixCoverageResult::Unresolved,
        PosixRuntimeStatus::Unsupported => PosixCoverageResult::Unsupported,
        PosixRuntimeStatus::Untested => PosixCoverageResult::Untested,
        PosixRuntimeStatus::LaunchError => PosixCoverageResult::LaunchError,
    }
}

#[cfg(not(test))]
struct RunnerState {
    filter: PosixFilter,
    metadata: PosixManifestMetadata,
    selected: Vec<PosixManifestTest>,
    run_id: String,
    build_id: String,
    seq: u64,
    started_tick: u64,
    next_index: usize,
    current_index: Option<usize>,
    current_started_tick: u64,
    resource_before: PosixResourceSnapshot,
    coverage: PosixCoverageTracker,
}

#[cfg(not(test))]
struct RunnerStateCell(UnsafeCell<Option<RunnerState>>);

#[cfg(not(test))]
// SAFETY: Runner entry points run under the repository's serialized scheduler rule.
unsafe impl Sync for RunnerStateCell {}

#[cfg(not(test))]
static RUNNER_STATE: RunnerStateCell = RunnerStateCell(UnsafeCell::new(None));

#[cfg(not(test))]
fn with_runner_state<R>(operation: impl FnOnce(&mut Option<RunnerState>) -> R) -> R {
    // SAFETY: `start` and the pinned ELF completion callback are scheduler-serialized.
    unsafe { operation(&mut *RUNNER_STATE.0.get()) }
}

pub fn parse_filter(args: &[&str]) -> Result<PosixFilter, PosixTestError> {
    match args {
        ["all"] => Ok(PosixFilter::All),
        ["group", value] => parse_filter_value(value).map(PosixFilter::Group),
        ["api", value] => parse_filter_value(value).map(PosixFilter::Api),
        ["test", value] => parse_filter_value(value).map(PosixFilter::Test),
        _ => Err(PosixTestError::InvalidFilter),
    }
}

fn parse_filter_value(value: &str) -> Result<String, PosixTestError> {
    if value.len() > POSIX_FILTER_MAX_BYTES || !posix_test_logic_shared::manifest_atom_valid(value)
    {
        return Err(PosixTestError::InvalidFilter);
    }
    Ok(String::from(value))
}

pub fn load_manifest() -> Result<PosixManifest, PosixTestError> {
    fxfs::ensure_host_share().map_err(|_| PosixTestError::FxfsPrepare)?;
    let attrs = fxfs::attrs(POSIX_MANIFEST_PATH).map_err(|_| PosixTestError::FxfsRead)?;
    if attrs.size > POSIX_MANIFEST_MAX_BYTES {
        return Err(PosixTestError::ManifestTooLarge);
    }
    let mut bytes = Vec::new();
    bytes.resize(attrs.size, 0u8);
    let mut cursor =
        fxfs::open_cursor(POSIX_MANIFEST_PATH).map_err(|_| PosixTestError::FxfsRead)?;
    let read = fxfs::cursor_read_for_mmap(&mut cursor, &mut bytes)
        .map_err(|_| PosixTestError::FxfsRead)?;
    if read > POSIX_MANIFEST_MAX_BYTES {
        return Err(PosixTestError::ManifestTooLarge);
    }
    bytes.truncate(read);
    parse_manifest(&bytes)
}

#[cfg(test)]
pub fn status_snapshot() -> PosixRunnerStatus {
    let coverage = PosixCoverageSnapshot::default();
    PosixRunnerStatus {
        running: false,
        run_id: None,
        filter: None,
        current_test: None,
        completed: coverage.tests_completed,
        selected: coverage.tests_selected,
        status_counts: coverage.status_counts,
        coverage,
    }
}

#[cfg(not(test))]
pub fn status_snapshot() -> PosixRunnerStatus {
    with_runner_state(|slot| match slot.as_ref() {
        Some(state) => {
            let coverage = state.coverage.snapshot();
            PosixRunnerStatus {
                running: true,
                run_id: Some(state.run_id.clone()),
                filter: Some(state.filter.clone()),
                current_test: state
                    .current_index
                    .and_then(|index| state.selected.get(index))
                    .map(|test| test.test_id.clone()),
                completed: coverage.tests_completed,
                selected: coverage.tests_selected,
                status_counts: coverage.status_counts,
                coverage,
            }
        }
        None => {
            let coverage = PosixCoverageSnapshot::default();
            PosixRunnerStatus {
                running: false,
                run_id: None,
                filter: None,
                current_test: None,
                completed: coverage.tests_completed,
                selected: coverage.tests_selected,
                status_counts: coverage.status_counts,
                coverage,
            }
        }
    })
}

fn derive_build_id(metadata: &PosixManifestMetadata) -> String {
    let mut canonical = String::new();
    canonical.push_str("{\"build_results_sha256\":\"");
    canonical.push_str(metadata.build_results_sha256.as_str());
    canonical.push_str("\",\"manifest_sha256\":\"");
    canonical.push_str(metadata.manifest_sha256.as_str());
    canonical.push_str("\",\"patch_sha256\":\"");
    canonical.push_str(metadata.patch_sha256.as_str());
    canonical.push_str("\",\"revision\":\"");
    canonical.push_str(metadata.revision.as_str());
    canonical.push_str("\",\"smros_commit\":\"");
    canonical.push_str(metadata.smros_commit.as_str());
    canonical.push_str("\"}");

    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    digest_hex(&hasher.finish())
}

fn digest_hex(digest: &[u8; 32]) -> String {
    let alphabet = b"0123456789abcdef";
    let mut output = String::new();
    for byte in digest {
        output.push(alphabet[(byte >> 4) as usize] as char);
        output.push(alphabet[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(not(test))]
fn decimal_string(mut value: u64) -> String {
    if value == 0 {
        return String::from("0");
    }
    let mut reversed = [0u8; 20];
    let mut len = 0usize;
    while value != 0 {
        reversed[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    let mut output = String::new();
    while len != 0 {
        len -= 1;
        output.push(reversed[len] as char);
    }
    output
}

#[cfg(not(test))]
fn make_run_id(build_id: &str, tick: u64) -> String {
    let mut run_id = String::from(build_id);
    run_id.push('-');
    run_id.push_str(decimal_string(tick).as_str());
    run_id
}

#[cfg(not(test))]
pub fn start(filter: PosixFilter) -> Result<(), PosixTestError> {
    if with_runner_state(|slot| slot.is_some()) {
        return Err(PosixTestError::AlreadyRunning);
    }

    let manifest = match load_manifest() {
        Ok(manifest) => manifest,
        Err(error) => {
            emit_unbound_infrastructure_error(error.as_str());
            return Err(error);
        }
    };
    let mut selected = Vec::new();
    for test in manifest.tests {
        if test_matches_filter(&test, &filter)
            && selected_test_action(&test) != SelectedTestAction::Ignore
        {
            selected.push(test);
        }
    }
    let mut coverage = PosixCoverageTracker::default();
    for test in &selected {
        if coverage
            .select(test.api.as_str(), test.group.as_str())
            .is_err()
        {
            emit_unbound_infrastructure_error("coverage-selection-invariant");
            return Err(PosixTestError::InfrastructureError);
        }
    }

    let started_tick = crate::kernel_lowlevel::timer::get_tick_count();
    let build_id = derive_build_id(&manifest.metadata);
    let run_id = make_run_id(build_id.as_str(), started_tick);
    let empty = selected.is_empty();
    let state = RunnerState {
        filter,
        metadata: manifest.metadata,
        selected,
        run_id,
        build_id,
        seq: 0,
        started_tick,
        next_index: 0,
        current_index: None,
        current_started_tick: 0,
        resource_before: resource_snapshot(false),
        coverage,
    };

    if with_runner_state(|slot| {
        if slot.is_some() {
            false
        } else {
            *slot = Some(state);
            true
        }
    }) {
        with_runner_state(|slot| {
            if let Some(state) = slot.as_mut() {
                emit_suite_start(state);
                emit_selection_summary(state);
            }
        });
    } else {
        return Err(PosixTestError::AlreadyRunning);
    }

    if empty {
        infrastructure_error(PosixTestError::EmptySelection.as_str());
        return Err(PosixTestError::EmptySelection);
    }
    let launch_result = launch_current_test(false);
    start_result_after_launch(launch_result)
}

#[cfg(not(test))]
fn launch_current_test(harness_launcher_active: bool) -> PosixLaunchLoopResult {
    enum NextLaunch {
        Running,
        Completed,
        MissingState,
        Selected(PosixManifestTest, SelectedTestAction),
    }

    let mut synchronous_launch_errors = 0usize;
    loop {
        let current = with_runner_state(|slot| {
            let Some(state) = slot.as_mut() else {
                return NextLaunch::MissingState;
            };
            if state.current_index.is_some() {
                return NextLaunch::Running;
            }
            if state.next_index >= state.selected.len() {
                return NextLaunch::Completed;
            }
            let index = state.next_index;
            state.next_index += 1;
            state.current_index = Some(index);
            state.current_started_tick = crate::kernel_lowlevel::timer::get_tick_count();
            state.resource_before = resource_snapshot(harness_launcher_active);
            let test = state.selected[index].clone();
            emit_test_start(state, &test);
            NextLaunch::Selected(test.clone(), selected_test_action(&test))
        });

        let (test, action) = match current {
            NextLaunch::Running => {
                return PosixLaunchLoopResult::Running(synchronous_launch_errors);
            }
            NextLaunch::Completed => {
                finish_suite();
                return PosixLaunchLoopResult::Completed(synchronous_launch_errors);
            }
            NextLaunch::MissingState => {
                infrastructure_error("runner-state-missing");
                return PosixLaunchLoopResult::InfrastructureError(synchronous_launch_errors);
            }
            NextLaunch::Selected(test, action) => (test, action),
        };
        if action == SelectedTestAction::EmitWithoutLaunch {
            if !record_unlaunched_test(harness_launcher_active) {
                infrastructure_error("runner-outcome-invariant");
                return PosixLaunchLoopResult::InfrastructureError(synchronous_launch_errors);
            }
            continue;
        }
        let Some(path) = test.binary_path.as_ref().cloned() else {
            infrastructure_error("selected-test-missing-binary");
            return PosixLaunchLoopResult::InfrastructureError(synchronous_launch_errors);
        };
        let mut argv = Vec::new();
        argv.push(path.clone());
        let mut env = Vec::new();
        env.push(String::from(POSIX_COMPAT_PRELOAD_ENV));
        env.push(String::from(POSIX_COMPAT_DIAG_ENV));
        match run_elf::spawn_observed(path.clone(), argv, env, RunObserver::PosixTest) {
            Ok(()) => return PosixLaunchLoopResult::Running(synchronous_launch_errors),
            Err(err) => {
                synchronous_launch_errors = synchronous_launch_errors.saturating_add(1);
                let outcome = RunOutcome {
                    path,
                    termination: RunTermination::LaunchError(err),
                    elapsed_ticks: 0,
                };
                if !record_run_outcome(&outcome, harness_launcher_active) {
                    infrastructure_error("runner-outcome-invariant");
                    return PosixLaunchLoopResult::InfrastructureError(synchronous_launch_errors);
                }
            }
        }
    }
}

#[cfg(not(test))]
pub fn on_run_outcome(outcome: RunOutcome) {
    if let RunTermination::InfrastructureError(error) = outcome.termination {
        infrastructure_error(error.as_str());
        return;
    }
    if !record_run_outcome(&outcome, true) {
        infrastructure_error("runner-outcome-invariant");
        return;
    }
    launch_current_test(true);
}

#[cfg(not(test))]
fn record_unlaunched_test(harness_launcher_active: bool) -> bool {
    let after = resource_snapshot(harness_launcher_active);
    with_runner_state(|slot| {
        let Some(state) = slot.as_mut() else {
            return false;
        };
        let Some(index) = state.current_index else {
            return false;
        };
        let test = state.selected[index].clone();
        if selected_test_action(&test) != SelectedTestAction::EmitWithoutLaunch {
            return false;
        }
        emit_unlaunched_test_end(state, &test, after);
        let update = match state.coverage.record(
            test.api.as_str(),
            test.group.as_str(),
            PosixCoverageResult::Untested,
        ) {
            Ok(update) => update,
            Err(_) => return false,
        };
        state.current_index = None;
        if posix_test_logic_shared::should_emit_progress(
            update.snapshot.tests_completed,
            update.snapshot.tests_selected,
            update.api_completed,
        ) {
            emit_progress(update.snapshot);
        }
        true
    })
}

#[cfg(not(test))]
fn record_run_outcome(outcome: &RunOutcome, harness_launcher_active: bool) -> bool {
    let after = resource_snapshot(harness_launcher_active);
    with_runner_state(|slot| {
        let Some(state) = slot.as_mut() else {
            return false;
        };
        let Some(index) = state.current_index else {
            return false;
        };
        let test = state.selected[index].clone();
        if selected_test_action(&test) == SelectedTestAction::Launch
            && test.binary_path.as_deref() != Some(outcome.path.as_str())
        {
            return false;
        }
        let status = match outcome.termination {
            RunTermination::Exit(exit_code) => pts_status(exit_code),
            RunTermination::LaunchError(_) => PosixRuntimeStatus::LaunchError,
            RunTermination::InfrastructureError(_) => return false,
        };
        emit_test_end(state, &test, &outcome, status, after);
        let update = match state.coverage.record(
            test.api.as_str(),
            test.group.as_str(),
            coverage_result(status),
        ) {
            Ok(update) => update,
            Err(_) => return false,
        };
        state.current_index = None;
        if posix_test_logic_shared::should_emit_progress(
            update.snapshot.tests_completed,
            update.snapshot.tests_selected,
            update.api_completed,
        ) {
            emit_progress(update.snapshot);
        }
        true
    })
}

#[cfg(not(test))]
fn resource_snapshot(harness_launcher_active: bool) -> PosixResourceSnapshot {
    let mut snapshot = crate::syscall::posix_resource_snapshot();
    snapshot.scheduler_threads = posix_test_logic_shared::normalize_scheduler_threads(
        snapshot.scheduler_threads,
        harness_launcher_active,
    );
    snapshot
}

#[cfg(not(test))]
fn finish_suite() {
    with_runner_state(|slot| {
        if let Some(state) = slot.as_mut() {
            let snapshot = state.coverage.snapshot();
            if snapshot.tests_completed == state.selected.len() {
                emit_suite_end(state);
            } else {
                emit_infrastructure_error(state, "coverage-completion-invariant");
            }
        }
        *slot = None;
    });
}

#[cfg(not(test))]
fn infrastructure_error(message: &str) {
    let emitted = with_runner_state(|slot| {
        let Some(state) = slot.as_mut() else {
            return false;
        };
        emit_infrastructure_error(state, message);
        *slot = None;
        true
    });
    if !emitted {
        emit_unbound_infrastructure_error(message);
    }
}

#[cfg(not(test))]
fn emit_suite_start(state: &mut RunnerState) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    begin_event(&mut serial, state, "suite_start");
    serial.write_str(",\"selected_count\":");
    write_u64(&mut serial, state.selected.len() as u64);
    serial.write_str(",\"build_id\":");
    write_json_string(&mut serial, state.build_id.as_str());
    serial.write_str(",\"build_results_sha256\":");
    write_json_string(&mut serial, state.metadata.build_results_sha256.as_str());
    serial.write_str(",\"smros_commit\":");
    write_json_string(&mut serial, state.metadata.smros_commit.as_str());
    serial.write_str(",\"revision\":");
    write_json_string(&mut serial, state.metadata.revision.as_str());
    serial.write_str(",\"patch_sha256\":");
    write_json_string(&mut serial, state.metadata.patch_sha256.as_str());
    serial.write_str(",\"filter\":");
    write_filter(&mut serial, &state.filter);
    serial.write_str(",\"started_ticks\":");
    write_u64(&mut serial, state.started_tick);
    serial.write_str(",\"source\":\"smros-serial\"}");
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn emit_selection_summary(state: &RunnerState) {
    let snapshot = state.coverage.snapshot();
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("posixtest: selection tests=");
    write_u64(&mut serial, snapshot.tests_selected as u64);
    serial.write_str(" apis=");
    write_u64(&mut serial, snapshot.apis_selected as u64);
    serial.write_str(" groups=");
    write_u64(&mut serial, snapshot.groups_selected as u64);
    serial.write_str(" interval=");
    write_u64(
        &mut serial,
        posix_test_logic_shared::POSIX_PROGRESS_INTERVAL as u64,
    );
    serial.write_str(" scope=selected\n");
}

#[cfg(not(test))]
fn emit_progress(snapshot: PosixCoverageSnapshot) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("posixtest: progress tests=");
    write_coverage_ratio(
        &mut serial,
        snapshot.tests_completed,
        snapshot.tests_selected,
    );
    serial.write_str(" apis-complete=");
    write_coverage_ratio(&mut serial, snapshot.apis_complete, snapshot.apis_selected);
    serial.write_str(" apis-pass=");
    write_coverage_ratio(&mut serial, snapshot.apis_pass, snapshot.apis_selected);
    serial.write_str(" groups-complete=");
    write_coverage_ratio(
        &mut serial,
        snapshot.groups_complete,
        snapshot.groups_selected,
    );
    serial.write_str(" groups-pass=");
    write_coverage_ratio(&mut serial, snapshot.groups_pass, snapshot.groups_selected);
    serial.write_str(" pass=");
    write_u64(&mut serial, snapshot.status_counts.passed as u64);
    serial.write_str(" fail=");
    write_u64(&mut serial, snapshot.status_counts.failed as u64);
    serial.write_str(" unresolved=");
    write_u64(&mut serial, snapshot.status_counts.unresolved as u64);
    serial.write_str(" unsupported=");
    write_u64(&mut serial, snapshot.status_counts.unsupported as u64);
    serial.write_str(" untested=");
    write_u64(&mut serial, snapshot.status_counts.untested as u64);
    serial.write_str(" launch-errors=");
    write_u64(&mut serial, snapshot.status_counts.launch_errors as u64);
    serial.write_str(" scope=selected\n");
}

#[cfg(not(test))]
fn emit_test_start(state: &mut RunnerState, test: &PosixManifestTest) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    begin_event(&mut serial, state, "test_start");
    write_test_identity(&mut serial, test);
    serial.write_str(",\"binary_sha256\":");
    write_json_string(&mut serial, test.sha256.as_str());
    serial.write_str(",\"source\":\"smros-serial\",\"started_ticks\":");
    write_u64(&mut serial, state.current_started_tick);
    serial.write_byte(b'}');
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn emit_test_end(
    state: &mut RunnerState,
    test: &PosixManifestTest,
    outcome: &RunOutcome,
    status: PosixRuntimeStatus,
    after: PosixResourceSnapshot,
) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_byte(b'\n');
    begin_event(&mut serial, state, "test_end");
    write_test_identity(&mut serial, test);
    serial.write_str(",\"status\":");
    write_json_string(&mut serial, status.as_str());
    match outcome.termination {
        RunTermination::Exit(exit_code) => {
            serial.write_str(",\"pts_status\":");
            write_json_string(&mut serial, status.as_str());
            serial.write_str(",\"launch_status\":\"launched\",\"exit_code\":");
            write_i64(&mut serial, exit_code as i64);
        }
        RunTermination::LaunchError(error) => {
            serial.write_str(",\"launch_status\":\"launch-error\",\"launch_error\":");
            write_json_string(&mut serial, error.as_str());
        }
        RunTermination::InfrastructureError(_) => return,
    }
    serial.write_str(",\"timed_out\":false,\"elapsed_ticks\":");
    write_u64(&mut serial, outcome.elapsed_ticks);
    serial.write_str(",\"resource_deltas\":{");
    write_resource_deltas(&mut serial, state.resource_before, after);
    serial.write_str("}}");
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn emit_unlaunched_test_end(
    state: &mut RunnerState,
    test: &PosixManifestTest,
    after: PosixResourceSnapshot,
) {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    begin_event(&mut serial, state, "test_end");
    write_test_identity(&mut serial, test);
    serial.write_str(r#","status":"untested","pts_status":null,"launch_status":"not-launched""#);
    serial.write_str(",\"elapsed_ticks\":0,\"resource_deltas\":{");
    write_resource_deltas(&mut serial, state.resource_before, after);
    serial.write_str("}}");
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn emit_suite_end(state: &mut RunnerState) {
    let elapsed =
        crate::kernel_lowlevel::timer::get_tick_count().saturating_sub(state.started_tick);
    let snapshot = state.coverage.snapshot();
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    begin_event(&mut serial, state, "suite_end");
    serial.write_str(",\"complete\":true,\"selected_count\":");
    write_u64(&mut serial, state.selected.len() as u64);
    serial.write_str(",\"completed_count\":");
    write_u64(&mut serial, snapshot.tests_completed as u64);
    serial.write_str(",\"status_counts\":{");
    write_status_counts(&mut serial, snapshot.status_counts);
    serial.write_str("},\"elapsed_ticks\":");
    write_u64(&mut serial, elapsed);
    serial.write_byte(b'}');
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn emit_infrastructure_error(state: &mut RunnerState, message: &str) {
    let current = state
        .current_index
        .and_then(|index| state.selected.get(index))
        .cloned();
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_byte(b'\n');
    begin_event(&mut serial, state, "infrastructure_error");
    serial.write_str(",\"message\":");
    write_json_string(&mut serial, message);
    if let Some(test) = current.as_ref() {
        write_test_identity(&mut serial, test);
    }
    serial.write_byte(b'}');
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn emit_unbound_infrastructure_error(message: &str) {
    let tick = crate::kernel_lowlevel::timer::get_tick_count();
    let mut run_id = String::from("error-");
    run_id.push_str(decimal_string(tick).as_str());
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str(POSIX_EVENT_PREFIX);
    serial.write_str("{\"schema\":");
    write_u64(&mut serial, POSIX_EVENT_SCHEMA as u64);
    serial.write_str(",\"seq\":1,\"event\":\"infrastructure_error\",\"run_id\":");
    write_json_string(&mut serial, run_id.as_str());
    serial.write_str(",\"manifest_sha256\":");
    write_json_string(&mut serial, EMPTY_SHA256);
    serial.write_str(",\"architecture\":\"");
    serial.write_str(POSIX_EVENT_ARCHITECTURE);
    serial.write_str("\",\"message\":");
    write_json_string(&mut serial, message);
    serial.write_byte(b'}');
    serial.write_byte(b'\n');
}

#[cfg(not(test))]
fn begin_event(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    state: &mut RunnerState,
    event: &str,
) {
    state.seq = state.seq.saturating_add(1);
    serial.write_str(POSIX_EVENT_PREFIX);
    serial.write_str("{\"schema\":");
    write_u64(serial, POSIX_EVENT_SCHEMA as u64);
    serial.write_str(",\"seq\":");
    write_u64(serial, state.seq);
    serial.write_str(",\"event\":");
    write_json_string(serial, event);
    serial.write_str(",\"run_id\":");
    write_json_string(serial, state.run_id.as_str());
    serial.write_str(",\"manifest_sha256\":");
    write_json_string(serial, state.metadata.manifest_sha256.as_str());
    serial.write_str(",\"architecture\":\"");
    serial.write_str(POSIX_EVENT_ARCHITECTURE);
    serial.write_byte(b'\"');
}

#[cfg(not(test))]
fn write_test_identity(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    test: &PosixManifestTest,
) {
    serial.write_str(",\"test_id\":");
    write_json_string(serial, test.test_id.as_str());
    serial.write_str(",\"group\":");
    write_json_string(serial, test.group.as_str());
    serial.write_str(",\"api\":");
    write_json_string(serial, test.api.as_str());
}

#[cfg(not(test))]
fn write_filter(serial: &mut crate::kernel_lowlevel::serial::Serial, filter: &PosixFilter) {
    let mut encoded = String::new();
    match filter {
        PosixFilter::All => encoded.push_str("all"),
        PosixFilter::Group(value) => {
            encoded.push_str("group=");
            encoded.push_str(value.as_str());
        }
        PosixFilter::Api(value) => {
            encoded.push_str("api=");
            encoded.push_str(value.as_str());
        }
        PosixFilter::Test(value) => {
            encoded.push_str("test=");
            encoded.push_str(value.as_str());
        }
    }
    write_json_string(serial, encoded.as_str());
}

#[cfg(not(test))]
fn write_json_string(serial: &mut crate::kernel_lowlevel::serial::Serial, value: &str) {
    serial.write_byte(b'\"');
    for byte in value.bytes() {
        write_json_byte(serial, byte);
    }
    serial.write_byte(b'\"');
}

#[cfg(not(test))]
fn write_json_byte(serial: &mut crate::kernel_lowlevel::serial::Serial, byte: u8) {
    match byte {
        b'\"' | b'\\' => {
            serial.write_byte(b'\\');
            serial.write_byte(byte);
        }
        0x20..=0x7e => serial.write_byte(byte),
        _ => serial.write_byte(b'?'),
    }
}

#[cfg(not(test))]
fn write_coverage_ratio(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    numerator: usize,
    denominator: usize,
) {
    write_u64(serial, numerator as u64);
    serial.write_byte(b'/');
    write_u64(serial, denominator as u64);
    serial.write_str(" (");
    let percent = posix_test_logic_shared::coverage_percent_hundredths(numerator, denominator);
    write_u64(serial, (percent / 100) as u64);
    serial.write_byte(b'.');
    serial.write_byte(b'0' + ((percent / 10) % 10) as u8);
    serial.write_byte(b'0' + (percent % 10) as u8);
    serial.write_str("%)");
}

#[cfg(not(test))]
fn write_resource_deltas(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    before: PosixResourceSnapshot,
    after: PosixResourceSnapshot,
) {
    write_delta_field(
        serial,
        "aio_requests",
        before.aio_requests,
        after.aio_requests,
        true,
    );
    write_delta_field(
        serial,
        "ipc_objects",
        before.ipc_objects,
        after.ipc_objects,
        false,
    );
    write_delta_field(
        serial,
        "kernel_handles",
        before.kernel_handles,
        after.kernel_handles,
        false,
    );
    write_delta_field(
        serial,
        "linux_fds",
        before.linux_fds,
        after.linux_fds,
        false,
    );
    write_delta_field(
        serial,
        "linux_mappings",
        before.linux_mappings,
        after.linux_mappings,
        false,
    );
    write_delta_field(
        serial,
        "linux_processes",
        before.linux_processes,
        after.linux_processes,
        false,
    );
    write_delta_field(
        serial,
        "linux_shared_memory",
        before.linux_shared_memory,
        after.linux_shared_memory,
        false,
    );
    write_delta_field(
        serial,
        "linux_zombies",
        before.linux_zombies,
        after.linux_zombies,
        false,
    );
    write_delta_field(
        serial,
        "page_table_pages",
        before.page_table_pages,
        after.page_table_pages,
        false,
    );
    write_delta_field(
        serial,
        "private_pages",
        before.private_pages,
        after.private_pages,
        false,
    );
    write_delta_field(
        serial,
        "processes",
        before.processes,
        after.processes,
        false,
    );
    write_delta_field(
        serial,
        "scheduler_threads",
        before.scheduler_threads,
        after.scheduler_threads,
        false,
    );
    write_delta_field(
        serial,
        "shared_pages",
        before.shared_pages,
        after.shared_pages,
        false,
    );
    write_delta_field(serial, "timers", before.timers, after.timers, false);
}

#[cfg(not(test))]
fn write_delta_field(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    name: &str,
    before: usize,
    after: usize,
    first: bool,
) {
    if !first {
        serial.write_byte(b',');
    }
    write_json_string(serial, name);
    serial.write_byte(b':');
    write_i128(
        serial,
        posix_test_logic_shared::resource_delta(before, after),
    );
}

#[cfg(not(test))]
fn write_status_counts(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    counts: PosixStatusCounts,
) {
    let entries = [
        ("fail", counts.failed),
        ("launch-error", counts.launch_errors),
        ("pass", counts.passed),
        ("unresolved", counts.unresolved),
        ("unsupported", counts.unsupported),
        ("untested", counts.untested),
    ];
    let mut first = true;
    for (name, count) in entries {
        if count == 0 {
            continue;
        }
        if !first {
            serial.write_byte(b',');
        }
        first = false;
        write_json_string(serial, name);
        serial.write_byte(b':');
        write_u64(serial, count as u64);
    }
}

#[cfg(not(test))]
fn write_i64(serial: &mut crate::kernel_lowlevel::serial::Serial, value: i64) {
    write_i128(serial, value as i128);
}

#[cfg(not(test))]
fn write_i128(serial: &mut crate::kernel_lowlevel::serial::Serial, value: i128) {
    if value < 0 {
        serial.write_byte(b'-');
        write_u128(serial, value.wrapping_neg() as u128);
    } else {
        write_u128(serial, value as u128);
    }
}

#[cfg(not(test))]
fn write_u128(serial: &mut crate::kernel_lowlevel::serial::Serial, mut value: u128) {
    if value == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut reversed = [0u8; 39];
    let mut len = 0usize;
    while value != 0 {
        reversed[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len != 0 {
        len -= 1;
        serial.write_byte(reversed[len]);
    }
}

#[cfg(not(test))]
fn write_u64(serial: &mut crate::kernel_lowlevel::serial::Serial, mut value: u64) {
    if value == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut reversed = [0u8; 20];
    let mut len = 0usize;
    while value != 0 {
        reversed[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len != 0 {
        len -= 1;
        serial.write_byte(reversed[len]);
    }
}

fn parse_fixed_fields<const N: usize>(line: &str) -> Option<[&str; N]> {
    let mut fields = [""; N];
    let mut parts = line.split('\t');
    for field in &mut fields {
        *field = parts.next()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(fields)
}

fn parse_manifest(data: &[u8]) -> Result<PosixManifest, PosixTestError> {
    if data.len() > POSIX_MANIFEST_MAX_BYTES {
        return Err(PosixTestError::ManifestTooLarge);
    }
    let text = core_compat::str::from_utf8(data).map_err(|_| PosixTestError::InvalidUtf8)?;
    if text.as_bytes().contains(&b'\r') || !text.ends_with('\n') {
        return Err(PosixTestError::InvalidLineEndings);
    }

    let mut lines = text[..text.len() - 1].split('\n');
    let header = lines.next().ok_or(PosixTestError::InvalidHeader)?;
    if header != MANIFEST_HEADER || POSIX_MANIFEST_SCHEMA != 1 {
        return Err(PosixTestError::InvalidHeader);
    }

    let mut metadata: [Option<String>; METADATA_KEYS.len()] = core_compat::array::from_fn(|_| None);
    let mut metadata_count = 0usize;
    let mut checksum_offset = None;
    let mut tests = Vec::new();
    let mut test_paths: BTreeSet<String> = BTreeSet::new();
    let mut previous_test_id: Option<String> = None;
    let mut saw_test = false;
    let mut line_offset = header.len() + 1;

    for line in lines {
        let current_offset = line_offset;
        line_offset = line_offset.saturating_add(line.len()).saturating_add(1);
        if line == "meta" || line.starts_with("meta\t") {
            let fields = parse_fixed_fields::<3>(line).ok_or(PosixTestError::InvalidMetadataRow)?;
            if saw_test {
                return Err(PosixTestError::InvalidMetadataRow);
            }
            let key = fields[1];
            let value = fields[2];
            let Some(key_index) = METADATA_KEYS.iter().position(|expected| *expected == key) else {
                return Err(PosixTestError::UnknownMetadata);
            };
            if metadata[key_index].is_some() {
                return Err(PosixTestError::DuplicateMetadata);
            }
            if key_index != metadata_count {
                return Err(PosixTestError::MetadataOutOfOrder);
            }
            validate_metadata_atom(value)?;
            if key == "manifest_sha256" {
                checksum_offset = Some(current_offset + "meta\tmanifest_sha256\t".len());
            }
            metadata[key_index] = Some(String::from(value));
            metadata_count += 1;
            continue;
        }

        if line == "test" || line.starts_with("test\t") {
            saw_test = true;
            let fields = parse_fixed_fields::<9>(line).ok_or(PosixTestError::InvalidTestRow)?;
            if tests.len() >= POSIX_MANIFEST_MAX_TESTS {
                return Err(PosixTestError::TooManyTests);
            }
            let test = parse_test_row(&fields)?;
            let previous_order = previous_test_id
                .as_ref()
                .map(|previous| previous.as_str().cmp(test.test_id.as_str()));
            if previous_order == Some(core_compat::cmp::Ordering::Equal) {
                return Err(PosixTestError::DuplicateTestId);
            }
            if let Some(path) = test.binary_path.as_ref() {
                if !test_paths.insert(path.clone()) {
                    return Err(PosixTestError::DuplicateTestPath);
                }
            }
            if previous_order == Some(core_compat::cmp::Ordering::Greater) {
                return Err(PosixTestError::NonCanonicalManifest);
            }
            previous_test_id = Some(test.test_id.clone());
            tests.push(test);
            continue;
        }

        return Err(PosixTestError::UnknownRowType);
    }

    if metadata_count != METADATA_KEYS.len() {
        return Err(PosixTestError::MissingMetadata);
    }
    let metadata = build_metadata(metadata)?;
    validate_provenance(&metadata)?;
    let checksum_offset = checksum_offset.ok_or(PosixTestError::MissingMetadata)?;
    if !manifest_checksum_matches(data, checksum_offset, &metadata.manifest_sha256) {
        return Err(PosixTestError::ManifestChecksumMismatch);
    }

    Ok(PosixManifest { metadata, tests })
}

fn parse_test_row(fields: &[&str]) -> Result<PosixManifestTest, PosixTestError> {
    let test_id = fields[1];
    let group = fields[2];
    let api = fields[3];
    validate_manifest_atom(test_id, POSIX_MANIFEST_MAX_TEST_ID_BYTES)?;
    validate_manifest_atom(group, POSIX_MANIFEST_MAX_GROUP_BYTES)?;
    validate_manifest_atom(api, POSIX_MANIFEST_MAX_API_BYTES)?;

    let kind = match fields[4] {
        "runnable" => PosixTestKind::Runnable,
        "definition" => PosixTestKind::Definition,
        "shell" => PosixTestKind::Shell,
        _ => return Err(PosixTestError::UnknownKind),
    };
    let disposition = match fields[5] {
        "complete" => PosixDisposition::Complete,
        "definition-only" => PosixDisposition::DefinitionOnly,
        "excluded-upstream-stub" => PosixDisposition::ExcludedUpstreamStub,
        "compile-failed" => PosixDisposition::CompileFailed,
        "link-failed" => PosixDisposition::LinkFailed,
        "not-built-shell-test" => PosixDisposition::NotBuiltShellTest,
        _ => return Err(PosixTestError::UnknownDisposition),
    };
    let valid_kind_disposition = matches!(
        (kind, disposition),
        (
            PosixTestKind::Runnable,
            PosixDisposition::Complete
                | PosixDisposition::ExcludedUpstreamStub
                | PosixDisposition::CompileFailed
                | PosixDisposition::LinkFailed
        ) | (
            PosixTestKind::Definition,
            PosixDisposition::DefinitionOnly
                | PosixDisposition::ExcludedUpstreamStub
                | PosixDisposition::CompileFailed
        ) | (PosixTestKind::Shell, PosixDisposition::NotBuiltShellTest)
    );
    if !valid_kind_disposition {
        return Err(PosixTestError::InvalidKindDisposition);
    }
    let timeout_ms = parse_timeout(fields[7])?;
    if !lower_hex(fields[8], 64) {
        return Err(PosixTestError::InvalidChecksum);
    }

    let binary_path = if disposition == PosixDisposition::Complete {
        let relative = fields[6];
        if relative.len() > POSIX_MANIFEST_MAX_STAGED_PATH_BYTES
            || !posix_test_logic_shared::manifest_atom_valid(relative)
        {
            return Err(PosixTestError::InvalidPath);
        }
        let mut guest_path = String::from("/shared/posixtest/");
        guest_path.push_str(relative);
        if !posix_test_logic_shared::staged_binary_path_valid(&guest_path)
            || fields[8] == EMPTY_SHA256
        {
            return Err(PosixTestError::InvalidPath);
        }
        Some(guest_path)
    } else {
        if fields[6] != "-" || fields[8] != EMPTY_SHA256 {
            return Err(PosixTestError::InvalidPath);
        }
        None
    };

    Ok(PosixManifestTest {
        test_id: String::from(test_id),
        group: String::from(group),
        api: String::from(api),
        kind,
        disposition,
        binary_path,
        timeout_ms,
        sha256: String::from(fields[8]),
    })
}

fn validate_manifest_atom(value: &str, maximum: usize) -> Result<(), PosixTestError> {
    if value.len() > maximum || !posix_test_logic_shared::manifest_atom_valid(value) {
        Err(PosixTestError::InvalidAtom)
    } else {
        Ok(())
    }
}

fn validate_metadata_atom(value: &str) -> Result<(), PosixTestError> {
    if value.is_empty()
        || value.len() > POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES
        || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        Err(PosixTestError::InvalidAtom)
    } else {
        Ok(())
    }
}

fn parse_timeout(value: &str) -> Result<u32, PosixTestError> {
    if value.is_empty()
        || (value.len() > 1 && value.as_bytes()[0] == b'0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(PosixTestError::InvalidTimeout);
    }
    let mut timeout = 0u32;
    for byte in value.bytes() {
        timeout = timeout
            .checked_mul(10)
            .and_then(|current| current.checked_add((byte - b'0') as u32))
            .ok_or(PosixTestError::InvalidTimeout)?;
    }
    if timeout == 0 || timeout > MAX_TIMEOUT_MS {
        Err(PosixTestError::InvalidTimeout)
    } else {
        Ok(timeout)
    }
}

fn build_metadata(
    mut values: [Option<String>; METADATA_KEYS.len()],
) -> Result<PosixManifestMetadata, PosixTestError> {
    let mut take = |index: usize| values[index].take().ok_or(PosixTestError::MissingMetadata);
    Ok(PosixManifestMetadata {
        source: take(0)?,
        revision: take(1)?,
        architecture: take(2)?,
        compiler: take(3)?,
        libc: take(4)?,
        patch_sha256: take(5)?,
        build_results_sha256: take(6)?,
        manifest_sha256: take(7)?,
        smros_commit: take(8)?,
    })
}

fn validate_provenance(metadata: &PosixManifestMetadata) -> Result<(), PosixTestError> {
    if metadata.architecture != "aarch64"
        || !lower_hex(&metadata.revision, 40)
        || !lower_hex(&metadata.smros_commit, 40)
        || !lower_hex(&metadata.patch_sha256, 64)
        || !lower_hex(&metadata.build_results_sha256, 64)
        || !lower_hex(&metadata.manifest_sha256, 64)
    {
        Err(PosixTestError::InvalidProvenance)
    } else {
        Ok(())
    }
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn manifest_checksum_matches(data: &[u8], offset: usize, expected: &str) -> bool {
    // Task 3 defines this as SHA-256 with the value replaced by 64 ASCII zeroes.
    if offset.checked_add(64).is_none() || offset + 64 > data.len() {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(&data[..offset]);
    hasher.update(EMPTY_SHA256.as_bytes());
    hasher.update(&data[offset + 64..]);
    let digest = sha256(hasher);
    let hex = b"0123456789abcdef";
    digest.iter().enumerate().all(|(index, byte)| {
        expected.as_bytes()[index * 2] == hex[(byte >> 4) as usize]
            && expected.as_bytes()[index * 2 + 1] == hex[(byte & 0x0f) as usize]
    })
}

fn sha256(hasher: Sha256) -> [u8; 32] {
    hasher.finish()
}

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    bytes: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            block_len: 0,
            bytes: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.bytes = self.bytes.saturating_add(input.len() as u64);
        if self.block_len != 0 {
            let take = core_compat::cmp::min(64 - self.block_len, input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
        while input.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&input[..64]);
            self.compress(&block);
            input = &input[64..];
        }
        if !input.is_empty() {
            self.block[..input.len()].copy_from_slice(input);
            self.block_len = input.len();
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_len = self.bytes.saturating_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
        } else {
            self.block[self.block_len..56].fill(0);
        }
        self.block[56..].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut schedule = [0u32; 64];
        for (index, bytes) in block.chunks_exact(4).enumerate() {
            schedule[index] = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const KNOWN_TASK3_MANIFEST: &str = concat!(
        "SMROS_POSIX_MANIFEST\t1\n",
        "meta\tsource\thttps://example.test/suite.git\n",
        "meta\trevision\t1111111111111111111111111111111111111111\n",
        "meta\tarchitecture\taarch64\n",
        "meta\tcompiler\tcc\n",
        "meta\tlibc\tlibc\n",
        "meta\tpatch_sha256\t2222222222222222222222222222222222222222222222222222222222222222\n",
        "meta\tbuild_results_sha256\t3333333333333333333333333333333333333333333333333333333333333333\n",
        "meta\tmanifest_sha256\t0fa18bad8c314f3633f768f2115c63e1a6ed7b2fe6d4ebca89ab000f25c8758b\n",
        "meta\tsmros_commit\t4444444444444444444444444444444444444444\n",
        "test\tconformance/interfaces/getpid/1-1.c\tbase\tgetpid\trunnable\tcomplete\tbin/getpid.test\t30000\t5555555555555555555555555555555555555555555555555555555555555555\n",
    );

    fn hex_digest(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let mut output = String::new();
        let hex = b"0123456789abcdef";
        for byte in hasher.finish() {
            output.push(hex[(byte >> 4) as usize] as char);
            output.push(hex[(byte & 0x0f) as usize] as char);
        }
        output
    }

    fn manifest(rows: &[String]) -> Vec<u8> {
        manifest_with_source("https://example.test/suite.git", rows)
    }

    fn manifest_with_source(source: &str, rows: &[String]) -> Vec<u8> {
        let mut text = format!(
            concat!(
                "SMROS_POSIX_MANIFEST\t1\n",
                "meta\tsource\t{}\n",
                "meta\trevision\t1111111111111111111111111111111111111111\n",
                "meta\tarchitecture\taarch64\n",
                "meta\tcompiler\tcc\n",
                "meta\tlibc\tlibc\n",
                "meta\tpatch_sha256\t2222222222222222222222222222222222222222222222222222222222222222\n",
                "meta\tbuild_results_sha256\t3333333333333333333333333333333333333333333333333333333333333333\n",
                "meta\tmanifest_sha256\t{}\n",
                "meta\tsmros_commit\t4444444444444444444444444444444444444444\n",
            ),
            source,
            EMPTY_SHA256
        );
        for row in rows {
            text.push_str(row);
            text.push('\n');
        }
        let digest = hex_digest(text.as_bytes());
        text = text.replacen(EMPTY_SHA256, &digest, 1);
        text.into_bytes()
    }

    fn row(
        id: &str,
        group: &str,
        api: &str,
        kind: &str,
        disposition: &str,
        path: &str,
        timeout: &str,
        checksum: &str,
    ) -> String {
        format!("test\t{id}\t{group}\t{api}\t{kind}\t{disposition}\t{path}\t{timeout}\t{checksum}")
    }

    fn runnable_row(id: &str, path: &str) -> String {
        row(
            id,
            "base",
            "getpid",
            "runnable",
            "complete",
            path,
            "30000",
            &"5".repeat(64),
        )
    }

    #[test]
    fn accepts_task3_manifest_and_sha256_interoperates() {
        let parsed = parse_manifest(KNOWN_TASK3_MANIFEST.as_bytes()).expect("canonical manifest");
        assert_eq!(parsed.tests.len(), 1);
        assert_eq!(
            parsed.tests[0].binary_path.as_deref(),
            Some("/shared/posixtest/bin/getpid.test")
        );
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let tampered = KNOWN_TASK3_MANIFEST.replace("\tbase\t", "\ttime\t");
        assert_eq!(
            parse_manifest(tampered.as_bytes()),
            Err(PosixTestError::ManifestChecksumMismatch)
        );
    }

    #[test]
    fn rejects_invalid_kind_disposition_combinations_with_valid_checksums() {
        let kinds = ["runnable", "definition", "shell"];
        let dispositions = [
            "complete",
            "definition-only",
            "excluded-upstream-stub",
            "compile-failed",
            "link-failed",
            "not-built-shell-test",
        ];
        for kind in kinds {
            for disposition in dispositions {
                let allowed = matches!(
                    (kind, disposition),
                    (
                        "runnable",
                        "complete" | "excluded-upstream-stub" | "compile-failed" | "link-failed"
                    ) | (
                        "definition",
                        "definition-only" | "excluded-upstream-stub" | "compile-failed"
                    ) | ("shell", "not-built-shell-test")
                );
                let (path, checksum) = if disposition == "complete" {
                    ("bin/case.test", "6".repeat(64))
                } else {
                    ("-", String::from(EMPTY_SHA256))
                };
                let bytes = manifest(&[row(
                    "case",
                    "base",
                    "api",
                    kind,
                    disposition,
                    path,
                    "30000",
                    &checksum,
                )]);
                assert_eq!(
                    parse_manifest(&bytes).is_ok(),
                    allowed,
                    "kind/disposition decision differs for {kind}/{disposition}"
                );
            }
        }
    }

    #[test]
    fn rejects_malformed_rows_duplicates_and_unsafe_values() {
        let valid = runnable_row("case-a", "bin/a.test");
        let cases = [
            manifest(&[String::from("bogus\tvalue")]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "bogus",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "bogus",
                "-",
                "1",
                EMPTY_SHA256,
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "../a",
                "1",
                &"5".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "bin/a",
                "0",
                &"5".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"A".repeat(64),
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "complete",
                "-",
                "1",
                EMPTY_SHA256,
            )]),
            manifest(&[row(
                "case",
                "base",
                "api",
                "runnable",
                "compile-failed",
                "bin/a",
                "1",
                &"5".repeat(64),
            )]),
            manifest(&[valid.clone(), valid.clone()]),
            manifest(&[
                runnable_row("case-a", "bin/same.test"),
                runnable_row("case-b", "bin/same.test"),
            ]),
            manifest(&[
                runnable_row("case-b", "bin/b.test"),
                runnable_row("case-a", "bin/a.test"),
            ]),
        ];
        for bytes in cases {
            assert!(parse_manifest(&bytes).is_err());
        }

        assert_eq!(parse_manifest(&[0xff]), Err(PosixTestError::InvalidUtf8));
        assert!(parse_manifest(KNOWN_TASK3_MANIFEST.replace('\n', "\r\n").as_bytes()).is_err());
        assert!(parse_manifest(KNOWN_TASK3_MANIFEST.trim_end().as_bytes()).is_err());

        let unknown_metadata = KNOWN_TASK3_MANIFEST.replace("meta\tsource\t", "meta\tunknown\t");
        assert!(parse_manifest(unknown_metadata.as_bytes()).is_err());
        let duplicate_metadata = KNOWN_TASK3_MANIFEST.replace(
            "meta\trevision\t1111111111111111111111111111111111111111\n",
            "meta\tsource\tduplicate\n",
        );
        assert!(parse_manifest(duplicate_metadata.as_bytes()).is_err());
        let missing_metadata = KNOWN_TASK3_MANIFEST.replace("meta\tcompiler\tcc\n", "");
        assert!(parse_manifest(missing_metadata.as_bytes()).is_err());
        let metadata_out_of_order = KNOWN_TASK3_MANIFEST.replace(
            "meta\tcompiler\tcc\nmeta\tlibc\tlibc\n",
            "meta\tlibc\tlibc\nmeta\tcompiler\tcc\n",
        );
        assert!(parse_manifest(metadata_out_of_order.as_bytes()).is_err());
    }

    #[test]
    fn rejects_near_limit_tab_row_with_fixed_field_extraction() {
        let row_len = POSIX_MANIFEST_MAX_BYTES - MANIFEST_HEADER.len() - 2;
        let mut row = String::from("test");
        row.push_str(&"\t".repeat(row_len - row.len()));
        assert!(parse_fixed_fields::<9>(&row).is_none());

        let mut data = String::from(MANIFEST_HEADER);
        data.push('\n');
        data.push_str(&row);
        data.push('\n');
        assert_eq!(data.len(), POSIX_MANIFEST_MAX_BYTES);
        assert_eq!(
            parse_manifest(data.as_bytes()),
            Err(PosixTestError::InvalidTestRow)
        );
    }

    #[test]
    fn parses_maximum_long_prefix_rows_and_rejects_duplicate_path() {
        let id_prefix = "i".repeat(190);
        let path_prefix = "p".repeat(180);
        let mut rows = Vec::new();
        for index in 0..POSIX_MANIFEST_MAX_TESTS {
            rows.push(runnable_row(
                &format!("{id_prefix}{index:04}"),
                &format!("bin/{path_prefix}{index:04}.test"),
            ));
        }

        let bytes = manifest(&rows);
        assert!(bytes.len() <= POSIX_MANIFEST_MAX_BYTES);
        assert_eq!(
            parse_manifest(&bytes)
                .expect("maximum canonical manifest")
                .tests
                .len(),
            POSIX_MANIFEST_MAX_TESTS
        );

        let duplicate_index = POSIX_MANIFEST_MAX_TESTS - 1;
        rows[duplicate_index] = runnable_row(
            &format!("{id_prefix}{duplicate_index:04}"),
            &format!("bin/{path_prefix}{:04}.test", duplicate_index - 1),
        );
        assert_eq!(
            parse_manifest(&manifest(&rows)),
            Err(PosixTestError::DuplicateTestPath)
        );
    }

    #[test]
    fn manifest_and_filter_bounds_are_exact() {
        assert!(parse_manifest(&manifest_with_source(
            &"s".repeat(POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES),
            &[]
        ))
        .is_ok());
        assert!(parse_manifest(&manifest_with_source(
            &"s".repeat(POSIX_MANIFEST_MAX_METADATA_VALUE_BYTES + 1),
            &[]
        ))
        .is_err());

        let exact_id = "i".repeat(POSIX_MANIFEST_MAX_TEST_ID_BYTES);
        let exact_group = "g".repeat(POSIX_MANIFEST_MAX_GROUP_BYTES);
        let exact_api = "a".repeat(POSIX_MANIFEST_MAX_API_BYTES);
        let exact_path = format!(
            "bin/{}",
            "p".repeat(POSIX_MANIFEST_MAX_STAGED_PATH_BYTES - 4)
        );
        let bytes = manifest(&[row(
            &exact_id,
            &exact_group,
            &exact_api,
            "runnable",
            "complete",
            &exact_path,
            "1",
            &"5".repeat(64),
        )]);
        assert!(parse_manifest(&bytes).is_ok());

        for invalid in [
            row(
                &"i".repeat(POSIX_MANIFEST_MAX_TEST_ID_BYTES + 1),
                "g",
                "a",
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            ),
            row(
                "i",
                &"g".repeat(POSIX_MANIFEST_MAX_GROUP_BYTES + 1),
                "a",
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            ),
            row(
                "i",
                "g",
                &"a".repeat(POSIX_MANIFEST_MAX_API_BYTES + 1),
                "runnable",
                "complete",
                "bin/a",
                "1",
                &"5".repeat(64),
            ),
            row(
                "i",
                "g",
                "a",
                "runnable",
                "complete",
                &format!(
                    "bin/{}",
                    "p".repeat(POSIX_MANIFEST_MAX_STAGED_PATH_BYTES - 3)
                ),
                "1",
                &"5".repeat(64),
            ),
        ] {
            assert!(parse_manifest(&manifest(&[invalid])).is_err());
        }

        let mut maximum_rows = Vec::new();
        for index in 0..POSIX_MANIFEST_MAX_TESTS {
            maximum_rows.push(runnable_row(
                &format!("case-{index:04}"),
                &format!("bin/case-{index:04}.test"),
            ));
        }
        assert!(parse_manifest(&manifest(&maximum_rows)).is_ok());
        maximum_rows.push(runnable_row("case-4096", "bin/case-4096.test"));
        assert_eq!(
            parse_manifest(&manifest(&maximum_rows)),
            Err(PosixTestError::TooManyTests)
        );
        let mut oversized = Vec::new();
        oversized.resize(POSIX_MANIFEST_MAX_BYTES + 1, b'x');
        assert_eq!(
            parse_manifest(&oversized),
            Err(PosixTestError::ManifestTooLarge)
        );

        assert_eq!(parse_filter(&["all"]), Ok(PosixFilter::All));
        assert_eq!(
            parse_filter(&["api", "getpid"]),
            Ok(PosixFilter::Api(String::from("getpid")))
        );
        assert!(parse_filter(&[]).is_err());
        assert!(parse_filter(&["all", "extra"]).is_err());
        assert_eq!(
            parse_filter(&["api", "get"]),
            Ok(PosixFilter::Api(String::from("get")))
        );
        assert!(parse_filter(&["group", "../unsafe"]).is_err());
        let oversized = "x".repeat(POSIX_FILTER_MAX_BYTES + 1);
        assert!(parse_filter(&["test", &oversized]).is_err());
    }

    #[test]
    fn start_outcome_distinguishes_completion_launch_and_infrastructure_results() {
        assert_eq!(
            start_result_after_launch(PosixLaunchLoopResult::Running(0)),
            Ok(())
        );
        assert_eq!(
            start_result_after_launch(PosixLaunchLoopResult::Running(1)),
            Ok(())
        );
        assert_eq!(
            start_result_after_launch(PosixLaunchLoopResult::Completed(0)),
            Ok(())
        );
        assert_eq!(
            start_result_after_launch(PosixLaunchLoopResult::Completed(1)),
            Err(PosixTestError::LaunchError)
        );
        assert_eq!(
            start_result_after_launch(PosixLaunchLoopResult::InfrastructureError(0)),
            Err(PosixTestError::InfrastructureError)
        );
        assert_eq!(
            start_result_after_launch(PosixLaunchLoopResult::InfrastructureError(1)),
            Err(PosixTestError::InfrastructureError)
        );
        assert_eq!(PosixTestError::LaunchError.as_str(), "launch-error");
        assert_eq!(
            PosixTestError::InfrastructureError.as_str(),
            "infrastructure-error"
        );
    }

    #[test]
    fn runner_filters_dispositions_and_pts_statuses_are_exact() {
        let parsed = parse_manifest(KNOWN_TASK3_MANIFEST.as_bytes()).expect("canonical manifest");
        let runnable = &parsed.tests[0];
        assert!(test_matches_filter(runnable, &PosixFilter::All));
        assert!(test_matches_filter(
            runnable,
            &PosixFilter::Group(String::from("base"))
        ));
        assert!(!test_matches_filter(
            runnable,
            &PosixFilter::Group(String::from("bas"))
        ));
        assert!(test_matches_filter(
            runnable,
            &PosixFilter::Api(String::from("getpid"))
        ));
        assert!(!test_matches_filter(
            runnable,
            &PosixFilter::Api(String::from("get"))
        ));
        assert_eq!(selected_test_action(runnable), SelectedTestAction::Launch);

        let mut definition = runnable.clone();
        definition.kind = PosixTestKind::Definition;
        definition.disposition = PosixDisposition::DefinitionOnly;
        definition.binary_path = None;
        assert_eq!(
            selected_test_action(&definition),
            SelectedTestAction::Ignore
        );
        definition.disposition = PosixDisposition::ExcludedUpstreamStub;
        assert_eq!(
            selected_test_action(&definition),
            SelectedTestAction::EmitWithoutLaunch
        );
        assert!(!test_matches_filter(&definition, &PosixFilter::All));
        assert!(test_matches_filter(
            &definition,
            &PosixFilter::Group(String::from("base"))
        ));

        for (exit_code, expected) in [
            (0, PosixRuntimeStatus::Pass),
            (1, PosixRuntimeStatus::Fail),
            (2, PosixRuntimeStatus::Unresolved),
            (4, PosixRuntimeStatus::Unsupported),
            (5, PosixRuntimeStatus::Untested),
            (127, PosixRuntimeStatus::Fail),
        ] {
            assert_eq!(pts_status(exit_code), expected);
        }
        assert_eq!(
            coverage_result(PosixRuntimeStatus::Pass),
            PosixCoverageResult::Pass
        );
        assert_eq!(
            coverage_result(PosixRuntimeStatus::LaunchError),
            PosixCoverageResult::LaunchError
        );
    }

    #[test]
    fn guest_build_id_matches_host_canonical_json_digest() {
        let parsed = parse_manifest(KNOWN_TASK3_MANIFEST.as_bytes()).expect("canonical manifest");
        assert_eq!(
            derive_build_id(&parsed.metadata),
            "a6b4a96d5075473a42b6d07ea883139013f65bd45487afaf118bde38f0086e9a"
        );
    }
}
