//! One HTTP client shared by every adapter that speaks HTTP (FiveM, the 7 Days to Die web API,
//! VEIN's HTTP API), so connections and TLS setup are reused between checks.

use std::sync::OnceLock;
use std::time::Duration;

pub(crate) fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(4))
            .user_agent(concat!("Lighthouse/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("TLS backend is compiled in")
    })
}
