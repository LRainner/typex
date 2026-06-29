pub const COMMON_TARGETS: &[&str] = &[
    "typex_core",
    "typex_pipeline",
    "typex_asr",
    "typex_llm",
    "typex_plugin",
    "typex_audio",
    "typex_injector",
];

#[macro_export]
macro_rules! log_target {
    ($level:expr, target: $target:literal, $fmt:literal $(, $arg:expr)* $(,)?) => {{
        match $level {
            tracing::Level::ERROR => tracing::error!(target: $target, $fmt $(, $arg)*),
            tracing::Level::WARN => tracing::warn!(target: $target, $fmt $(, $arg)*),
            tracing::Level::INFO => tracing::info!(target: $target, $fmt $(, $arg)*),
            tracing::Level::DEBUG => tracing::debug!(target: $target, $fmt $(, $arg)*),
            tracing::Level::TRACE => tracing::trace!(target: $target, $fmt $(, $arg)*),
        }
    }};
    ($level:expr, target: $target:literal, $message:expr $(,)?) => {{
        match $level {
            tracing::Level::ERROR => tracing::error!(target: $target, "{}", $message),
            tracing::Level::WARN => tracing::warn!(target: $target, "{}", $message),
            tracing::Level::INFO => tracing::info!(target: $target, "{}", $message),
            tracing::Level::DEBUG => tracing::debug!(target: $target, "{}", $message),
            tracing::Level::TRACE => tracing::trace!(target: $target, "{}", $message),
        }
    }};
}

#[macro_export]
macro_rules! log_text_target {
    ($level:expr, target: $target:literal, $message:expr, $text:expr, $record_text:expr $(,)?) => {{
        let text = $text;
        let logged_text = if $record_text { text } else { "<redacted>" };
        match $level {
            tracing::Level::ERROR => tracing::error!(
                target: $target,
                text = %logged_text,
                text_len = text.len(),
                text_chars = text.chars().count(),
                "{}",
                $message
            ),
            tracing::Level::WARN => tracing::warn!(
                target: $target,
                text = %logged_text,
                text_len = text.len(),
                text_chars = text.chars().count(),
                "{}",
                $message
            ),
            tracing::Level::INFO => tracing::info!(
                target: $target,
                text = %logged_text,
                text_len = text.len(),
                text_chars = text.chars().count(),
                "{}",
                $message
            ),
            tracing::Level::DEBUG => tracing::debug!(
                target: $target,
                text = %logged_text,
                text_len = text.len(),
                text_chars = text.chars().count(),
                "{}",
                $message
            ),
            tracing::Level::TRACE => tracing::trace!(
                target: $target,
                text = %logged_text,
                text_len = text.len(),
                text_chars = text.chars().count(),
                "{}",
                $message
            ),
        }
    }};
}

pub fn build_filter(level: &str, extra_targets: &[&str]) -> String {
    let mut parts = Vec::with_capacity(extra_targets.len() + COMMON_TARGETS.len());
    parts.extend(
        extra_targets
            .iter()
            .map(|target| format!("{target}={level}")),
    );
    parts.extend(
        COMMON_TARGETS
            .iter()
            .map(|target| format!("{target}={level}")),
    );
    parts.join(",")
}

pub fn text_preview(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    let mut preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

pub fn redact_url_for_log(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "<redacted>".into();
    }

    let (scheme, rest) = match trimmed.split_once("://") {
        Some((scheme, rest)) if !scheme.is_empty() && !rest.is_empty() => (scheme, rest),
        _ => return "<redacted>".into(),
    };

    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let rest = match rest.find('@') {
        Some(at) => rest.get(at + 1..).unwrap_or(""),
        None => rest,
    };

    if rest.is_empty() {
        return "<redacted>".into();
    }

    format!("{scheme}://{rest}")
}

#[cfg(test)]
mod tests {
    use super::{build_filter, redact_url_for_log, text_preview};

    #[test]
    fn text_preview_trims_input() {
        assert_eq!(text_preview("  hello  ", 20), "hello");
    }

    #[test]
    fn text_preview_truncates_by_chars() {
        assert_eq!(text_preview("你好世界", 2), "你好…");
    }

    #[test]
    fn text_preview_does_not_append_ellipsis_at_limit() {
        assert_eq!(text_preview("你好", 2), "你好");
    }

    #[test]
    fn build_filter_includes_common_targets() {
        let filter = build_filter("info", &["typex_cli"]);
        assert!(filter.contains("typex_cli=info"));
        assert!(filter.contains("typex_core=info"));
        assert!(filter.contains("typex_pipeline=info"));
    }

    #[test]
    fn redact_url_for_log_strips_credentials_and_query() {
        assert_eq!(
            redact_url_for_log("https://user:pass@example.com/v1?key=secret#frag"),
            "https://example.com/v1"
        );
    }

    #[test]
    fn redact_url_for_log_redacts_invalid_inputs() {
        assert_eq!(redact_url_for_log("not a url"), "<redacted>");
    }
}
