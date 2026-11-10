//! Reachability of an explicit Claude API address. No request data is sent.
use discard_core::provider::ProviderError;
use tokio::process::Command;
use std::time::Duration;

fn api_url(command: &Command) -> Option<url::Url> {
    let key = std::ffi::OsStr::new("ANTHROPIC_BASE_URL");
    let value = match command.as_std().get_envs().find(|(name, _)| *name == key) {
        Some((_, value)) => value?.to_os_string(),
        None => std::env::var_os(key)?,
    };
    url::Url::parse(value.to_str()?).ok()

}

/// The scheme, host, and port of a custom API address. Never the path or user information.
pub(super) fn api_address(command: &Command) -> Option<String> {
    origin(&api_url(command)?)
}

fn origin(url: &url::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

pub(super) async fn check_api_address(command: &Command) -> Result<(), ProviderError> {
    let Some(url) = api_url(command) else { return Ok(()); };
    let Some(host) = url.host() else { return Ok(()); };
    let Some(port) = url.port().or_else(|| match url.scheme() { "https" => Some(443), "http" => Some(80), _ => None }) else { return Ok(()); };
    let host = match host {
        url::Host::Domain(host) => host.to_owned(),
        url::Host::Ipv4(host) => host.to_string(),
        url::Host::Ipv6(host) => host.to_string(),
    };
    if matches!(tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect((host.as_str(), port))).await, Ok(Ok(_))) { return Ok(()); }
    let origin = origin(&url).expect("URL has a host");
    Err(ProviderError::Setup(format!("Claude cannot reach the API address {origin} (ANTHROPIC_BASE_URL). Start that service or remove the variable, then retry.")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unreachable_address_fails_fast_without_credentials_or_path() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut command = Command::new("unused");
        command.env("ANTHROPIC_BASE_URL", format!("http://user:secret@127.0.0.1:{port}/private?token=hidden"));
        let start = std::time::Instant::now();
        let error = check_api_address(&command).await.unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(matches!(error, ProviderError::Setup(_)));
        let message = error.to_string();
        assert!(message.contains(&format!("Claude cannot reach the API address http://127.0.0.1:{port} (ANTHROPIC_BASE_URL)")), "{message}");
        for secret in ["user", "secret", "private", "hidden"] { assert!(!message.contains(secret)); }
    }
    #[tokio::test]
    async fn reachable_address_passes_without_sending_data() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut command = Command::new("unused");
        command.env("ANTHROPIC_BASE_URL", format!("http://{}", listener.local_addr().unwrap()));
        check_api_address(&command).await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1];
        use tokio::io::AsyncReadExt;
        assert_eq!(socket.read(&mut bytes).await.unwrap(), 0);
    }
    #[tokio::test]
    async fn removed_or_unparseable_address_skips_the_check() {
        let mut command = Command::new("unused");
        command.env_remove("ANTHROPIC_BASE_URL");
        assert_eq!(api_address(&command), None);
        check_api_address(&command).await.unwrap();
        command.env("ANTHROPIC_BASE_URL", "not a URL");
        check_api_address(&command).await.unwrap();
        command.env("ANTHROPIC_BASE_URL", "file:///private");
        check_api_address(&command).await.unwrap();
    }
    #[test]
    fn ipv6_origin_has_no_user_information_or_path() {
        let mut command = Command::new("unused");
        command.env("ANTHROPIC_BASE_URL", "http://user:secret@[::1]:8787/private");
        assert_eq!(api_address(&command).as_deref(), Some("http://[::1]:8787"));
    }
}
