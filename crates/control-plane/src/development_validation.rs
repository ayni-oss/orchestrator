fn initial_phase(provider: &str) -> DevelopmentPhase {
    if provider == "openai" {
        DevelopmentPhase::Queued
    } else {
        DevelopmentPhase::Unsupported
    }
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn git_branch(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains("@{")
        && value.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && !std::path::Path::new(part)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("lock"))
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn model_text(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
}
