pub mod agent;
pub mod commands;
pub mod config;
pub mod extensions;
pub mod fox3d;
pub mod highlight;
pub mod llm;
pub mod mcp;
pub mod session;
pub mod theme;
pub mod tools;

pub use reqwest::Client as HttpClient;

/// Creates an HTTP client configured with sane connect and stream read timeouts.
pub fn build_http_client() -> HttpClient {
    HttpClient::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| HttpClient::new())
}
