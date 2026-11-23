// Pure campaign deadline predicate. Serial output and `system_reset` stay in
// `campaign_watch.rs`; they are the trusted reset path, not this predicate.

macro_rules! smros_campaign_deadline_exceeded_body {
    ($armed:expr, $deadline:expr, $now:expr) => {{
        $armed && $deadline != 0 && $now >= $deadline
    }};
}
