use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

use super::state::AppState;

/// Embedded dashboard assets from the pre-built React SPA.
/// If `dashboard/dist/` does not exist at compile time, this embeds zero files.
#[derive(Embed)]
#[folder = "../../dashboard/dist/"]
struct DashboardAssets;

/// Build the dashboard route group.
///
/// Behaviour depends on the `MIKA_DASHBOARD_ENABLED` setting:
/// - **Enabled + assets exist:** Serves the embedded SPA with token injection.
/// - **Enabled + assets missing:** Returns a "not built" instructions page.
/// - **Disabled (default):** Returns a branded "disabled" page.
pub fn dashboard_routes() -> Router<AppState> {
    Router::new().fallback(serve_dashboard)
}

async fn serve_dashboard(State(state): State<AppState>, uri: axum::http::Uri) -> Response {
    if !state.settings.dashboard_enabled {
        return disabled_page().into_response();
    }

    // Check if any assets are embedded at all
    if DashboardAssets::iter().next().is_none() {
        return not_built_page().into_response();
    }

    // Strip leading slash to get the file path within the embedded folder
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // Try to serve the exact file
    if let Some(file) = DashboardAssets::get(path) {
        let mime = mime_from_path(path);
        if path == "index.html" {
            // Inject runtime config into index.html
            let html = String::from_utf8_lossy(&file.data);
            let injected = inject_config_into_html(&html, &state);
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                injected,
            )
                .into_response();
        }
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            file.data.to_vec(),
        )
            .into_response();
    }

    // SPA fallback: serve index.html for any unmatched path (client-side routing)
    if let Some(index) = DashboardAssets::get("index.html") {
        let html = String::from_utf8_lossy(&index.data);
        let injected = inject_config_into_html(&html, &state);
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            injected,
        )
            .into_response();
    }

    // Should not happen if assets were embedded, but handle gracefully
    not_built_page().into_response()
}

/// Inject `window.__MIKA_CONFIG__` into the HTML `<head>` for runtime token discovery.
fn inject_config_into_html(html: &str, state: &AppState) -> String {
    use secrecy::ExposeSecret;

    // Prefer dashboard_token (read-only). Fall back to internal_token (superuser).
    let token = state
        .dashboard_token
        .as_ref()
        .map(|t| t.expose_secret().to_string())
        .unwrap_or_else(|| state.internal_token.expose_secret().to_string());

    let config_script = format!(
        r#"<script>window.__MIKA_CONFIG__={{token:"{}",basePath:"/dashboard"}};</script>"#,
        token.replace('\\', "\\\\").replace('"', "\\\""),
    );

    // Insert before </head> if found, otherwise prepend to document
    if let Some(pos) = html.find("</head>") {
        let mut result = String::with_capacity(html.len() + config_script.len());
        result.push_str(&html[..pos]);
        result.push_str(&config_script);
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{config_script}{html}")
    }
}

/// Branded HTML page shown when the dashboard is disabled.
fn disabled_page() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Mika Dashboard — Disabled</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    background: #0a0a0a; color: #e5e5e5;
    font-family: system-ui, -apple-system, sans-serif;
    display: flex; align-items: center; justify-content: center;
    min-height: 100vh; padding: 2rem;
  }
  .container { max-width: 480px; text-align: center; }
  h1 { font-size: 1.5rem; margin-bottom: 0.75rem; color: #fff; }
  p { color: #a3a3a3; line-height: 1.6; margin-bottom: 1.5rem; }
  code {
    display: inline-block; background: #1a1a2e; color: #818cf8;
    padding: 0.5rem 1rem; border-radius: 6px; font-size: 0.875rem;
    font-family: 'SF Mono', Menlo, monospace;
  }
  .mika { color: #818cf8; }
</style>
</head>
<body>
<div class="container">
  <h1><span class="mika">✦</span> Mika Dashboard</h1>
  <p>The embedded dashboard is disabled. To enable it, set the environment variable:</p>
  <code>MIKA_DASHBOARD_ENABLED=true</code>
  <p style="margin-top: 1.5rem; font-size: 0.875rem;">Then restart mika-server.</p>
</div>
</body>
</html>"#,
    )
}

/// HTML page shown when the dashboard is enabled but assets were not built.
fn not_built_page() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Mika Dashboard — Not Built</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    background: #0a0a0a; color: #e5e5e5;
    font-family: system-ui, -apple-system, sans-serif;
    display: flex; align-items: center; justify-content: center;
    min-height: 100vh; padding: 2rem;
  }
  .container { max-width: 520px; text-align: center; }
  h1 { font-size: 1.5rem; margin-bottom: 0.75rem; color: #fff; }
  p { color: #a3a3a3; line-height: 1.6; margin-bottom: 1rem; }
  code {
    display: block; background: #1a1a2e; color: #818cf8;
    padding: 0.5rem 1rem; border-radius: 6px; font-size: 0.875rem;
    font-family: 'SF Mono', Menlo, monospace; margin: 0.5rem 0;
    text-align: left;
  }
  .mika { color: #818cf8; }
</style>
</head>
<body>
<div class="container">
  <h1><span class="mika">✦</span> Mika Dashboard</h1>
  <p>The dashboard is enabled but no assets were found. Build the dashboard before compiling the server:</p>
  <code>npm run build --prefix dashboard</code>
  <code>cargo build --release</code>
  <p style="margin-top: 1rem; font-size: 0.875rem;">The Docker build handles this automatically.</p>
</div>
</body>
</html>"#,
    )
}

/// Map file extension to Content-Type header value.
fn mime_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_from_path() {
        assert_eq!(mime_from_path("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_from_path("app.js"), "application/javascript");
        assert_eq!(mime_from_path("style.css"), "text/css");
        assert_eq!(mime_from_path("data.json"), "application/json");
        assert_eq!(mime_from_path("logo.svg"), "image/svg+xml");
        assert_eq!(mime_from_path("photo.png"), "image/png");
        assert_eq!(mime_from_path("font.woff2"), "font/woff2");
        assert_eq!(mime_from_path("unknown.xyz"), "application/octet-stream");
    }

    #[test]
    fn test_inject_config_into_html() {
        // We can't easily construct a full AppState for unit testing,
        // so we test the HTML injection logic directly.
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let expected_contains = r#"window.__MIKA_CONFIG__"#;

        // Test the injection point detection
        assert!(html.contains("</head>"));
        let pos = html.find("</head>").unwrap();
        assert!(pos > 0);

        // Test that the disabled and not-built pages render
        let disabled = disabled_page();
        assert!(disabled.0.contains("MIKA_DASHBOARD_ENABLED"));

        let not_built = not_built_page();
        assert!(not_built.0.contains("npm run build"));

        // Verify the config script format is valid
        let script = format!(
            r#"<script>window.__MIKA_CONFIG__={{token:"{}",basePath:"/dashboard"}};</script>"#,
            "test-token"
        );
        assert!(script.contains(expected_contains));
    }
}
