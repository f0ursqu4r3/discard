//! Tested CLI versions and the fail-closed version gate.
use discard_core::provider::ProviderError;

pub(crate) const CLAUDE_TESTED: &str = "2.1.318";
pub(crate) const CODEX_TESTED: &str = "0.166.0";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VersionCheck {
    Tested,
    Newer { running: String },
    Older { running: String },
    Unknown,
}

fn parse(output: &str) -> Option<([u64; 3], String)> {
    output.split_whitespace().find_map(|token| {
        let parts: Vec<_> = token.split('.').collect();
        if parts.len() != 3 || parts.iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit())) { return None; }
        Some(([parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?], token.into()))
    })
}

/// The first `x.y.z` token of CLI version output.
pub(crate) fn number(output: &str) -> Option<String> { parse(output).map(|(_, text)| text) }

pub(crate) fn check(output: &str, tested: &str) -> VersionCheck {
    let Some((running, text)) = parse(output) else { return VersionCheck::Unknown; };
    let Some((tested, _)) = parse(tested) else { return VersionCheck::Unknown; };
    match running.cmp(&tested) {
        std::cmp::Ordering::Equal => VersionCheck::Tested,
        std::cmp::Ordering::Greater => VersionCheck::Newer { running: text },
        std::cmp::Ordering::Less => VersionCheck::Older { running: text },
    }
}

pub(crate) fn require(output: &str, cli: &str, tested: &str) -> Result<String, ProviderError> {
    match check(output, tested) {
        VersionCheck::Tested => Ok(tested.into()),
        VersionCheck::Newer { running } => Ok(running),
        VersionCheck::Older { running } => Err(ProviderError::Setup(format!("{cli} {running} is older than the tested version {tested}. Update {cli}, then retry."))),
        VersionCheck::Unknown => Err(ProviderError::Setup(format!("The {cli} version could not be read. Check the CLI installation, then retry."))),
    }
}

pub(crate) fn notice(cli: &str, running: &str, tested: &str) -> Option<String> {
    (running != tested).then(|| format!("{cli} {running} is newer than the tested version {tested}. Discard verified the isolation controls before this response."))
}

/// Preserve the error type, including Workspace's read-only fallback.
pub(crate) fn verification_error(error: ProviderError, cli: &str, running: &str, tested: &str) -> ProviderError {
    if running == tested { return error; }
    let context = |message| format!("{cli} {running} (tested version {tested}): {message}");
    match error {
        ProviderError::Setup(m) => ProviderError::Setup(context(m)),
        ProviderError::SandboxUnavailable(m) => ProviderError::SandboxUnavailable(context(m)),
        ProviderError::Protocol(m) => ProviderError::Protocol(context(m)),
        ProviderError::Spawn(m) => ProviderError::Spawn(context(m)),
        ProviderError::Io(m) => ProviderError::Io(context(m)),
        ProviderError::ResumeRejected(m) => ProviderError::ResumeRejected(context(m)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_numeric_versions_and_ignores_surrounding_text() {
        assert_eq!(check("2.1.318 (Claude Code)", CLAUDE_TESTED), VersionCheck::Tested);
        assert_eq!(check("codex-cli 0.166.0", CODEX_TESTED), VersionCheck::Tested);
        for running in ["2.1.321", "2.2.0", "3.0.0"] {
            assert_eq!(check(&format!("CLI {running} extra text"), CLAUDE_TESTED), VersionCheck::Newer { running: running.into() });
        }
        assert_eq!(check("2.1.99", CLAUDE_TESTED), VersionCheck::Older { running: "2.1.99".into() });
        for output in ["garbage", "2.1", "2.1.318.1", "2.1.318-beta", "2.1.-1", "999999999999999999999.1.0"] {
            assert_eq!(check(output, CLAUDE_TESTED), VersionCheck::Unknown);
        }
    }
}
