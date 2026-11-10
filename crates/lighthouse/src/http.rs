use std::time::Duration;

/// Shared HTTP client for Discord and GitHub calls made outside the bots.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Lighthouse/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .expect("TLS backend is compiled in")
}
