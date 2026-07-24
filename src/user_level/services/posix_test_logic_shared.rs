pub const POSIX_STATUS_PASS: u8 = 0;
pub const POSIX_STATUS_FAIL: u8 = 1;
pub const POSIX_STATUS_UNRESOLVED: u8 = 2;
pub const POSIX_STATUS_UNSUPPORTED: u8 = 3;
pub const POSIX_STATUS_UNTESTED: u8 = 4;
pub const POSIX_STATUS_INTERRUPTED: u8 = 5;

pub const POSIX_STAGE_BIN_PREFIX: &str = "/shared/posixtest/bin/";

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
            && atom
                .bytes()
                .all(|byte| (0x20..=0x7e).contains(&byte))
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
            _ => POSIX_STATUS_INTERRUPTED,
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
