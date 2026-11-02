mod app;
mod constants;
#[cfg(feature = "hotpath")]
pub mod demo;
mod events;
mod exit_hint;
mod http_worker;
mod input;
mod views;
mod widgets;

use std::time::Instant;

use app::App;
use clap::Parser;
use eyre::Result;

#[derive(Debug, Parser)]
pub struct ConsoleArgs {
    #[arg(
        long,
        default_value_t = default_metrics_port(),
        help = "Port where the metrics HTTP server is running (env: HOTPATH_METRICS_PORT)"
    )]
    pub metrics_port: u16,

    #[arg(
        long,
        default_value_t = default_metrics_host(),
        value_parser = validate_metrics_host,
        help = "Host URL where the metrics HTTP server is running (env: HOTPATH_METRICS_HOST)"
    )]
    pub metrics_host: String,

    #[arg(
        long,
        help = "Auth token for the metrics server, sent as-is in the Authorization header (env: HOTPATH_METRICS_AUTH_TOKEN)"
    )]
    pub metrics_auth_token: Option<String>,

    #[arg(long, default_value_t = default_refresh_interval(), help = "Refresh interval in milliseconds (env: HOTPATH_TUI_REFRESH_INTERVAL_MS)")]
    pub refresh_interval: u64,
}

#[hotpath::measure_all]
impl ConsoleArgs {
    pub fn run(&self) -> Result<()> {
        hotpath::dev_logging::init_logging();

        #[cfg(feature = "hotpath")]
        demo::init();

        let auth_token = self
            .metrics_auth_token
            .clone()
            .or_else(default_metrics_auth_token)
            .map(|token| validate_metrics_auth_token(&token))
            .transpose()
            .map_err(|e| eyre::eyre!(e))?;

        let mut app = App::new(
            &self.metrics_host,
            self.metrics_port,
            auth_token,
            self.refresh_interval,
        );

        let mut terminal = ratatui::init();
        enable_mouse_capture();

        let started = Instant::now();
        let app_result = app.run(&mut terminal);

        disable_mouse_capture();
        ratatui::restore();

        exit_hint::maybe_print(exit_hint::Session {
            duration: started.elapsed(),
            ended_cleanly: app_result.is_ok(),
            received_data: app.last_successful_fetch.is_some(),
        });

        app_result.map_err(|e| eyre::eyre!("TUI error: {}", e))
    }
}

/// Mouse capture is not part of `ratatui::init()`/`restore()`, so the panic
/// hook installed by `ratatui::init()` would leave it enabled on panic and the
/// terminal would keep printing mouse escape sequences. Chain a hook that
/// disables it before the ratatui restore hook runs.
fn enable_mouse_capture() {
    if crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture).is_err() {
        return;
    }
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_mouse_capture();
        prev_hook(info);
    }));
}

fn disable_mouse_capture() {
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
}

impl Default for ConsoleArgs {
    fn default() -> Self {
        Self {
            metrics_port: default_metrics_port(),
            metrics_host: default_metrics_host(),
            metrics_auth_token: default_metrics_auth_token(),
            refresh_interval: default_refresh_interval(),
        }
    }
}

fn default_metrics_port() -> u16 {
    std::env::var("HOTPATH_METRICS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6770)
}

fn default_refresh_interval() -> u64 {
    std::env::var("HOTPATH_TUI_REFRESH_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
}

fn default_metrics_host() -> String {
    std::env::var("HOTPATH_METRICS_HOST").unwrap_or_else(|_| "http://localhost".to_string())
}

fn default_metrics_auth_token() -> Option<String> {
    std::env::var("HOTPATH_METRICS_AUTH_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

fn validate_metrics_auth_token(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("metrics auth token cannot be empty".to_string());
    }
    reqwest::header::HeaderValue::from_str(s)
        .map_err(|_| "metrics auth token must be visible ASCII".to_string())?;
    Ok(s.to_string())
}

fn validate_metrics_host(s: &str) -> Result<String, String> {
    let s = s.trim();

    if s.is_empty() {
        return Err("metrics host cannot be empty".to_string());
    }

    let after_scheme = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .ok_or_else(|| {
            format!(
                "--metrics-host must start with 'http://' or 'https://', got: {}",
                s
            )
        })?;

    if after_scheme.is_empty() {
        return Err("metrics host must include a hostname after the scheme".to_string());
    }

    let host_part = after_scheme.split('/').next().unwrap_or("");

    if host_part.contains(':') {
        return Err(format!(
            "metrics host should not include a port (use --metrics-port instead), got: {}",
            s
        ));
    }

    if host_part.is_empty() {
        return Err("metrics host must include a valid hostname".to_string());
    }

    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_hosts() {
        let cases = [
            ("http://localhost", "http://localhost"),
            ("https://localhost", "https://localhost"),
            ("http://192.168.1.1", "http://192.168.1.1"),
            ("https://example.com", "https://example.com"),
            ("http://localhost/", "http://localhost/"),
            ("  http://localhost  ", "http://localhost"),
        ];

        for (input, expected) in cases {
            assert_eq!(
                validate_metrics_host(input),
                Ok(expected.to_string()),
                "failed for input: {input}"
            );
        }
    }

    #[test]
    fn test_invalid_hosts() {
        let cases = [
            "",
            "   ",
            "localhost",
            "ftp://localhost",
            "http://",
            "https://",
            "http://localhost:8080",
            "https://example.com:443",
        ];

        for input in cases {
            assert!(
                validate_metrics_host(input).is_err(),
                "expected error for input: {input}"
            );
        }
    }
}
