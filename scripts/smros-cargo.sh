# Shared kernel cargo job knob. Source this from build scripts.
# Host tests do not source it and do not set kernel codegen-units.

smros_build_jobs() {
    local jobs="${SMROS_BUILD_JOBS:-4}"
    case "$jobs" in
        ''|*[!0-9]*|0*)
            echo "SMROS_BUILD_JOBS must be a positive integer" >&2
            return 1
            ;;
    esac
    printf '%s\n' "$jobs"
}

smros_print_build_jobs() {
    local jobs
    jobs="$(smros_build_jobs)" || return 1
    echo "SMROS build jobs: ${jobs} (cargo -j ${jobs}, codegen-units ${jobs})"
}

smros_cargo() {
    local jobs
    jobs="$(smros_build_jobs)" || return 1
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$jobs" \
        CARGO_PROFILE_DEV_CODEGEN_UNITS="$jobs" \
        cargo "$@" -j "$jobs"
}
