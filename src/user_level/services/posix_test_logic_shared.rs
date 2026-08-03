use crate::alloc::collections::BTreeMap;
use crate::alloc::string::String;

pub const POSIX_STATUS_PASS: u8 = 0;
pub const POSIX_STATUS_FAIL: u8 = 1;
pub const POSIX_STATUS_UNRESOLVED: u8 = 2;
pub const POSIX_STATUS_UNSUPPORTED: u8 = 3;
pub const POSIX_STATUS_UNTESTED: u8 = 4;
pub const POSIX_STATUS_INTERRUPTED: u8 = 5;

pub const POSIX_STAGE_BIN_PREFIX: &str = "/shared/posixtest/bin/";
pub const POSIX_PROGRESS_INTERVAL: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixCoverageResult {
    Pass,
    Fail,
    Unresolved,
    Unsupported,
    Untested,
    LaunchError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixCoverageError {
    CounterOverflow,
    UnknownUnit,
    TestOverComplete,
    UnitOverComplete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PosixCoverageStatusCounts {
    pub passed: usize,
    pub failed: usize,
    pub unresolved: usize,
    pub unsupported: usize,
    pub untested: usize,
    pub launch_errors: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PosixCoverageSnapshot {
    pub tests_completed: usize,
    pub tests_selected: usize,
    pub apis_complete: usize,
    pub apis_pass: usize,
    pub apis_selected: usize,
    pub groups_complete: usize,
    pub groups_pass: usize,
    pub groups_selected: usize,
    pub status_counts: PosixCoverageStatusCounts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PosixCoverageUpdate {
    pub snapshot: PosixCoverageSnapshot,
    pub api_completed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PosixCoverageUnit {
    selected: usize,
    completed: usize,
    all_pass: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosixCoverageTracker {
    tests_selected: usize,
    tests_completed: usize,
    apis: BTreeMap<String, PosixCoverageUnit>,
    groups: BTreeMap<String, PosixCoverageUnit>,
    status_counts: PosixCoverageStatusCounts,
}

impl PosixCoverageStatusCounts {
    fn with_result(
        self,
        result: PosixCoverageResult,
    ) -> Result<Self, PosixCoverageError> {
        let mut next = self;
        let counter = match result {
            PosixCoverageResult::Pass => &mut next.passed,
            PosixCoverageResult::Fail => &mut next.failed,
            PosixCoverageResult::Unresolved => &mut next.unresolved,
            PosixCoverageResult::Unsupported => &mut next.unsupported,
            PosixCoverageResult::Untested => &mut next.untested,
            PosixCoverageResult::LaunchError => &mut next.launch_errors,
        };
        *counter = counter
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        Ok(next)
    }
}

impl PosixCoverageTracker {
    pub fn select(&mut self, api: &str, group: &str) -> Result<(), PosixCoverageError> {
        let tests_selected = self
            .tests_selected
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        let api_selected = Self::next_selected(&self.apis, api)?;
        let group_selected = Self::next_selected(&self.groups, group)?;

        self.tests_selected = tests_selected;
        Self::set_selected(&mut self.apis, api, api_selected);
        Self::set_selected(&mut self.groups, group, group_selected);
        Ok(())
    }

    pub fn record(
        &mut self,
        api: &str,
        group: &str,
        result: PosixCoverageResult,
    ) -> Result<PosixCoverageUpdate, PosixCoverageError> {
        if self.tests_completed >= self.tests_selected {
            return Err(PosixCoverageError::TestOverComplete);
        }
        let tests_completed = self
            .tests_completed
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        let (api_completed, api_now_complete) = Self::next_completed(&self.apis, api)?;
        let (group_completed, _) = Self::next_completed(&self.groups, group)?;
        let status_counts = self.status_counts.with_result(result)?;

        self.tests_completed = tests_completed;
        self.status_counts = status_counts;
        Self::set_completed(&mut self.apis, api, api_completed, result);
        Self::set_completed(&mut self.groups, group, group_completed, result);
        Ok(PosixCoverageUpdate {
            snapshot: self.snapshot(),
            api_completed: api_now_complete,
        })
    }

    pub fn snapshot(&self) -> PosixCoverageSnapshot {
        let (apis_complete, apis_pass, apis_selected) = unit_summary(&self.apis);
        let (groups_complete, groups_pass, groups_selected) = unit_summary(&self.groups);
        PosixCoverageSnapshot {
            tests_completed: self.tests_completed,
            tests_selected: self.tests_selected,
            apis_complete,
            apis_pass,
            apis_selected,
            groups_complete,
            groups_pass,
            groups_selected,
            status_counts: self.status_counts,
        }
    }

    fn next_selected(
        units: &BTreeMap<String, PosixCoverageUnit>,
        name: &str,
    ) -> Result<usize, PosixCoverageError> {
        units
            .get(name)
            .map_or(Some(1), |unit| unit.selected.checked_add(1))
            .ok_or(PosixCoverageError::CounterOverflow)
    }

    fn set_selected(
        units: &mut BTreeMap<String, PosixCoverageUnit>,
        name: &str,
        selected: usize,
    ) {
        match units.get_mut(name) {
            Some(unit) => unit.selected = selected,
            None => {
                units.insert(
                    String::from(name),
                    PosixCoverageUnit {
                        selected,
                        completed: 0,
                        all_pass: true,
                    },
                );
            }
        }
    }

    fn next_completed(
        units: &BTreeMap<String, PosixCoverageUnit>,
        name: &str,
    ) -> Result<(usize, bool), PosixCoverageError> {
        let unit = units.get(name).ok_or(PosixCoverageError::UnknownUnit)?;
        let completed = unit
            .completed
            .checked_add(1)
            .ok_or(PosixCoverageError::CounterOverflow)?;
        if completed > unit.selected {
            return Err(PosixCoverageError::UnitOverComplete);
        }
        Ok((completed, completed == unit.selected))
    }

    fn set_completed(
        units: &mut BTreeMap<String, PosixCoverageUnit>,
        name: &str,
        completed: usize,
        result: PosixCoverageResult,
    ) {
        let unit = units
            .get_mut(name)
            .expect("coverage unit was validated before mutation");
        unit.completed = completed;
        if result != PosixCoverageResult::Pass {
            unit.all_pass = false;
        }
    }
}

fn unit_summary(units: &BTreeMap<String, PosixCoverageUnit>) -> (usize, usize, usize) {
    let mut complete = 0usize;
    let mut pass = 0usize;
    for unit in units.values() {
        if unit.completed == unit.selected {
            complete += 1;
            if unit.all_pass {
                pass += 1;
            }
        }
    }
    (complete, pass, units.len())
}

pub fn coverage_percent_hundredths(numerator: usize, denominator: usize) -> usize {
    if denominator == 0 {
        0
    } else {
        numerator.saturating_mul(10_000) / denominator
    }
}

pub fn should_emit_progress(
    completed: usize,
    selected: usize,
    api_completed: bool,
) -> bool {
    completed > 0
        && (completed % POSIX_PROGRESS_INTERVAL == 0
            || api_completed
            || completed == selected)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixFilterKind {
    All,
    Group,
    Api,
    Test,
}

macro_rules! smros_posix_manifest_atom_valid_body {
    ($atom:expr) => {{
        let atom = $atom;
        !atom.is_empty()
            && !atom.contains('\\')
            && !atom.contains("//")
            && atom.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
            && atom
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
    }};
}

macro_rules! smros_posix_staged_binary_path_valid_body {
    ($path:expr) => {{
        match $path.strip_prefix(POSIX_STAGE_BIN_PREFIX) {
            Some(atom) => smros_posix_manifest_atom_valid_body!(atom),
            None => false,
        }
    }};
}

macro_rules! smros_posix_filter_matches_body {
    (
        $kind:expr,
        $value:expr,
        $test_id:expr,
        $group:expr,
        $api:expr,
        $runnable:expr,
        $complete:expr
    ) => {{
        match $kind {
            PosixFilterKind::All => $runnable && $complete,
            PosixFilterKind::Group => $value == $group,
            PosixFilterKind::Api => $value == $api,
            PosixFilterKind::Test => $value == $test_id,
        }
    }};
}

macro_rules! smros_posix_pts_status_body {
    ($exit_code:expr) => {{
        match $exit_code {
            0 => POSIX_STATUS_PASS,
            1 => POSIX_STATUS_FAIL,
            2 => POSIX_STATUS_UNRESOLVED,
            4 => POSIX_STATUS_UNSUPPORTED,
            5 => POSIX_STATUS_UNTESTED,
            _ => POSIX_STATUS_FAIL,
        }
    }};
}

macro_rules! smros_posix_resource_delta_body {
    ($before:expr, $after:expr) => {{
        // SMROS targets use a 64-bit usize, which is represented exactly by i128.
        ($after as i128) - ($before as i128)
    }};
}

pub fn manifest_atom_valid(atom: &str) -> bool {
    smros_posix_manifest_atom_valid_body!(atom)
}

pub fn staged_binary_path_valid(path: &str) -> bool {
    smros_posix_staged_binary_path_valid_body!(path)
}

pub fn filter_matches(
    kind: PosixFilterKind,
    value: &str,
    test_id: &str,
    group: &str,
    api: &str,
    runnable: bool,
    complete: bool,
) -> bool {
    smros_posix_filter_matches_body!(kind, value, test_id, group, api, runnable, complete)
}

pub fn pts_status(exit_code: i32) -> u8 {
    smros_posix_pts_status_body!(exit_code)
}

pub fn resource_delta(before: usize, after: usize) -> i128 {
    smros_posix_resource_delta_body!(before, after)
}

pub fn normalize_scheduler_threads(
    scheduler_threads: usize,
    harness_launcher_active: bool,
) -> usize {
    if harness_launcher_active {
        scheduler_threads.saturating_sub(1)
    } else {
        scheduler_threads
    }
}
