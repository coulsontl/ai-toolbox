//! On-demand, token-scoped static HTTP server for plugin pages.
//!
//! Nothing here runs until the first page is opened: [`PluginPageServer`]
//! binds `127.0.0.1:0` lazily, then serves only the folder of a page's entry
//! file, under a 32-character token minted for one `(plugin_id, page_id)`.
//! Disabling or uninstalling a plugin revokes its tokens; opening the page
//! again mints a new one.
//!
//! The server is intentionally tiny: one request per connection, GET/HEAD
//! only, no keep-alive, no directory listings, and a canonicalized path check
//! on every file so `..`, backslashes and symlinks cannot leave the page's
//! own folder.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::db::SqliteDbState;

use super::store;

/// Where a page loads the bridge from, relative to its own address.
pub const PAGE_BRIDGE_PATH: &str = "_magpie/bridge.js";
/// The bridge embedded into the binary and served beside every page.
pub const PAGE_BRIDGE_JS: &str = include_str!("page_bridge.js");

const PAGE_PREFIX: &str = "/_plugin/";
const TOKEN_LENGTH: usize = 32;
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);

struct PageRoute {
    plugin_id: String,
    page_id: String,
    /// Canonical directory the entry file lives in; every request resolves
    /// inside it.
    root: PathBuf,
    /// Entry file name relative to `root`.
    entry_file: String,
    service_port: Option<u16>,
}

#[derive(Default)]
struct PageServerState {
    address: Option<SocketAddr>,
    routes: HashMap<String, Arc<PageRoute>>,
}

/// One process-wide page server shared by every plugin.
pub struct PluginPageServer {
    state: Mutex<PageServerState>,
    secret: [u8; 32],
    random: SystemRandom,
}

impl Default for PluginPageServer {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginPageServer {
    pub fn new() -> Self {
        let random = SystemRandom::new();
        let mut secret = [0u8; 32];
        random
            .fill(&mut secret)
            .expect("the system random source must provide the page server secret");
        Self {
            state: Mutex::new(PageServerState::default()),
            secret,
            random,
        }
    }

    /// Bind the loopback listener on first use and start the accept loop.
    pub async fn ensure_started(self: &Arc<Self>) -> Result<SocketAddr, String> {
        if let Some(address) = self.state.lock().unwrap().address {
            return Ok(address);
        }

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| format!("Failed to start the plugin page server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("Failed to read the plugin page server address: {error}"))?;
        {
            let mut state = self.state.lock().unwrap();
            if let Some(existing) = state.address {
                // Another caller won the race; drop this listener.
                return Ok(existing);
            }
            state.address = Some(address);
        }

        let server = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let server = Arc::clone(&server);
                        tokio::spawn(async move {
                            server.serve_connection(stream).await;
                        });
                    }
                    Err(error) => {
                        log::warn!("plugin page server accept failed: {error}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Ok(address)
    }

    /// Mint a token for one page. The entry file must exist, be HTML, and its
    /// parent folder becomes the only directory the token can read.
    pub fn register_page(
        &self,
        plugin_id: &str,
        page_id: &str,
        entry_file: &Path,
        service_port: Option<u16>,
    ) -> Result<String, String> {
        let entry = std::fs::canonicalize(entry_file).map_err(|error| {
            format!(
                "Failed to resolve plugin page '{}': {error}",
                entry_file.display()
            )
        })?;
        let metadata = std::fs::metadata(&entry).map_err(|error| {
            format!("Failed to read plugin page '{}': {error}", entry.display())
        })?;
        if !metadata.is_file() {
            return Err(format!("'{}' is not a file", entry.display()));
        }
        if !is_html(&entry) {
            return Err(format!(
                "'{}' is not an .html or .htm page",
                entry.display()
            ));
        }
        let root = entry
            .parent()
            .ok_or_else(|| format!("'{}' has no parent folder", entry.display()))?
            .to_path_buf();
        let entry_file = entry
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| format!("'{}' has no file name", entry.display()))?;

        let token = self.new_token(plugin_id, page_id)?;
        let route = Arc::new(PageRoute {
            plugin_id: plugin_id.to_string(),
            page_id: page_id.to_string(),
            root,
            entry_file,
            service_port,
        });
        {
            let mut state = self.state.lock().unwrap();
            // One live token per page: opening it again replaces the old one
            // instead of growing the token table forever.
            state.routes.retain(|_, existing| {
                !(existing.plugin_id == plugin_id && existing.page_id == page_id)
            });
            state.routes.insert(token.clone(), route);
        }
        Ok(token)
    }

    /// Drop every token of a plugin. Its pages cannot be read until the
    /// frontend opens them again.
    pub fn revoke_plugin(&self, plugin_id: &str) {
        self.state
            .lock()
            .unwrap()
            .routes
            .retain(|_, route| route.plugin_id != plugin_id);
    }

    /// Drop the token of one page, leaving the plugin's other pages alone.
    pub fn revoke_plugin_page(&self, plugin_id: &str, page_id: &str) {
        self.state
            .lock()
            .unwrap()
            .routes
            .retain(|_, route| !(route.plugin_id == plugin_id && route.page_id == page_id));
    }

    /// Keep only tokens whose plugin is still installed and enabled and whose
    /// page still exists in its manifest.
    pub fn sync_enabled_plugins(&self, db: &SqliteDbState) -> Result<(), String> {
        let mut allowed = std::collections::HashSet::new();
        for record in store::list_installed_plugins(db)? {
            if !record.enabled {
                continue;
            }
            for page in record.pages {
                allowed.insert((record.id.clone(), page.id));
            }
        }
        self.state
            .lock()
            .unwrap()
            .routes
            .retain(|_, route| allowed.contains(&(route.plugin_id.clone(), route.page_id.clone())));
        Ok(())
    }

    fn new_token(&self, plugin_id: &str, page_id: &str) -> Result<String, String> {
        let mut nonce = [0u8; 32];
        self.random
            .fill(&mut nonce)
            .map_err(|_| "Failed to read the system random source".to_string())?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        let mut context = hmac::Context::with_key(&key);
        context.update(plugin_id.as_bytes());
        context.update(b"\0");
        context.update(page_id.as_bytes());
        context.update(b"\0");
        context.update(&nonce);
        let tag = context.sign();
        Ok(hex::encode(&tag.as_ref()[..16]))
    }

    fn route(&self, token: &str) -> Option<Arc<PageRoute>> {
        self.state.lock().unwrap().routes.get(token).cloned()
    }

    /// The exact origin a page is served from. WKWebView treats a sandboxed
    /// page as an opaque origin, where `'self'` matches nothing; the CSP has
    /// to name this prefix explicitly or the page's own scripts are blocked.
    fn origin_prefix(&self, token: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .address
            .map(|address| format!("http://{address}/_plugin/{token}/"))
    }

    async fn serve_connection(&self, mut stream: TcpStream) {
        let Some(request) = tokio::time::timeout(CONNECTION_TIMEOUT, read_request(&mut stream))
            .await
            .ok()
            .flatten()
        else {
            return;
        };
        let response = self.handle_request(&request).await;
        let bytes = response.to_bytes(request.method == "HEAD");
        let _ = tokio::time::timeout(CONNECTION_TIMEOUT, stream.write_all(&bytes)).await;
        let _ = stream.shutdown().await;
    }

    async fn handle_request(&self, request: &HttpRequest) -> HttpResponse {
        if request.method != "GET" && request.method != "HEAD" {
            return HttpResponse::text(405, "Method Not Allowed").with_allow_get();
        }

        let raw_path = request.target.split(['?', '#']).next().unwrap_or_default();
        let Some(rest) = raw_path.strip_prefix(PAGE_PREFIX) else {
            return HttpResponse::text(404, "Not Found");
        };
        let Some((token, relative)) = rest.split_once('/') else {
            return HttpResponse::text(404, "Not Found");
        };
        if token.len() != TOKEN_LENGTH || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return HttpResponse::text(404, "Not Found");
        }
        let Some(route) = self.route(token) else {
            return HttpResponse::text(404, "Not Found");
        };
        let Some(relative) = percent_decode(relative) else {
            return HttpResponse::text(400, "Bad Request");
        };
        if relative.contains('\0') || relative.contains('\\') {
            return HttpResponse::text(403, "Forbidden");
        }
        let segments: Vec<&str> = if relative.is_empty() {
            Vec::new()
        } else {
            relative.split('/').collect()
        };
        if segments.iter().any(|segment| *segment == "..") {
            return HttpResponse::text(403, "Forbidden");
        }
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == ".")
        {
            return HttpResponse::text(404, "Not Found");
        }

        if segments == ["_magpie", "bridge.js"] {
            return HttpResponse::file(
                200,
                "OK",
                "application/javascript; charset=utf-8",
                PAGE_BRIDGE_JS.as_bytes().to_vec(),
                None,
            );
        }

        let candidate = if segments.is_empty() {
            route.root.join(&route.entry_file)
        } else {
            let mut path = route.root.clone();
            for segment in &segments {
                path.push(segment);
            }
            path
        };
        let canonical = match tokio::fs::canonicalize(&candidate).await {
            Ok(path) => path,
            Err(_) => return HttpResponse::text(404, "Not Found"),
        };
        if !canonical.starts_with(&route.root) {
            return HttpResponse::text(403, "Forbidden");
        }
        match tokio::fs::metadata(&canonical).await {
            Ok(metadata) if metadata.is_file() => {}
            _ => return HttpResponse::text(404, "Not Found"),
        }
        let bytes = match tokio::fs::read(&canonical).await {
            Ok(bytes) => bytes,
            Err(_) => return HttpResponse::text(404, "Not Found"),
        };
        let csp = if is_html(&canonical) {
            let origin = self.origin_prefix(token).unwrap_or_default();
            Some(html_csp(route.service_port, &origin))
        } else if is_svg(&canonical) {
            Some(SVG_CSP.to_string())
        } else {
            None
        };
        HttpResponse::file(200, "OK", content_type_for(&canonical), bytes, csp)
    }
}

struct HttpRequest {
    method: String,
    target: String,
}

struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    csp: Option<String>,
    allow: bool,
}

impl HttpResponse {
    fn text(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: reason.as_bytes().to_vec(),
            csp: None,
            allow: false,
        }
    }

    fn file(
        status: u16,
        reason: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
        csp: Option<String>,
    ) -> Self {
        Self {
            status,
            reason,
            content_type,
            body,
            csp,
            allow: false,
        }
    }

    fn with_allow_get(mut self) -> Self {
        self.allow = true;
        self
    }

    fn to_bytes(&self, head_only: bool) -> Vec<u8> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\nConnection: close\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        );
        if let Some(csp) = &self.csp {
            head.push_str(&format!("Content-Security-Policy: {csp}\r\n"));
        }
        if self.allow {
            head.push_str("Allow: GET, HEAD\r\n");
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        if !head_only {
            bytes.extend_from_slice(&self.body);
        }
        bytes
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            return None;
        }
    }

    let head = String::from_utf8_lossy(&buffer);
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    Some(HttpRequest { method, target })
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = hex_value(*bytes.get(index + 1)?)?;
                let low = hex_value(*bytes.get(index + 2)?)?;
                decoded.push(high << 4 | low);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_html(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("html") | Some("htm")
    )
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

/// SVG can carry script; never let one reach the network or the host.
const SVG_CSP: &str =
    "sandbox; default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; connect-src 'none'";

/// The page policy: scripts and inline styles of its own files, pictures as
/// data or blob, no network at all unless the page declares a local service.
///
/// `origin` is the exact `http://<address>/_plugin/<token>/` prefix. It is
/// listed beside `'self'` because a WKWebView sandbox gives the page an opaque
/// origin, and then `'self'` matches none of its own resources.
fn html_csp(service_port: Option<u16>, origin: &str) -> String {
    let connect_src = match service_port {
        Some(port) => format!("connect-src http://127.0.0.1:{port} ws://127.0.0.1:{port}"),
        None => "connect-src 'none'".to_string(),
    };
    let origin = if origin.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", origin.trim())
    };
    format!(
        "sandbox allow-scripts; default-src 'self'{origin}; script-src 'self' 'unsafe-inline'{origin}; style-src 'self' 'unsafe-inline'{origin}; img-src 'self' data: blob:{origin}; font-src 'self' data:{origin}; {connect_src}"
    )
}

fn content_type_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::types::{InstalledPluginRecord, PluginPageManifest};

    struct RawResponse {
        status: u16,
        headers: String,
        body: String,
    }

    async fn request(address: SocketAddr, method: &str, path: &str) -> RawResponse {
        let mut stream = TcpStream::connect(address).await.unwrap();
        let request =
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let (headers, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        RawResponse {
            status,
            headers: headers.to_string(),
            body: body.to_string(),
        }
    }

    fn page_fixture() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let page_dir = root.path().join("page");
        std::fs::create_dir_all(&page_dir).unwrap();
        std::fs::write(
            page_dir.join("index.html"),
            "<!doctype html><h1>page</h1><script src=\"app.js\"></script>",
        )
        .unwrap();
        std::fs::write(page_dir.join("app.js"), "console.log('page')").unwrap();
        std::fs::write(page_dir.join("icon.svg"), "<svg></svg>").unwrap();
        std::fs::write(
            page_dir.join("index.html.extra"),
            "<!doctype html><h1>extra</h1>",
        )
        .unwrap();
        std::fs::write(root.path().join("outside.txt"), "outside").unwrap();
        (root, page_dir.join("index.html"))
    }

    #[tokio::test]
    async fn serves_html_assets_and_the_bridge_with_security_headers() {
        let (_root, entry) = page_fixture();
        let server = Arc::new(PluginPageServer::new());
        let token = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        let address = server.ensure_started().await.unwrap();

        let html = request(address, "GET", &format!("/_plugin/{token}/index.html")).await;
        assert_eq!(html.status, 200);
        assert!(html
            .headers
            .contains("Content-Type: text/html; charset=utf-8"));
        assert!(html
            .headers
            .contains("Content-Security-Policy: sandbox allow-scripts;"));
        assert!(html.headers.contains("connect-src 'none'"));
        // WKWebView gives the sandbox an opaque origin; the page's own
        // address has to be named beside 'self'.
        assert!(
            html.headers
                .contains(&format!("http://{address}/_plugin/{token}/")),
            "{}",
            html.headers
        );
        assert!(html.headers.contains("X-Content-Type-Options: nosniff"));
        assert!(html.headers.contains("Cache-Control: no-store"));
        assert!(html.body.contains("<h1>page</h1>"));

        let bridge = request(
            address,
            "GET",
            &format!("/_plugin/{token}/{PAGE_BRIDGE_PATH}"),
        )
        .await;
        assert_eq!(bridge.status, 200);
        assert!(bridge
            .headers
            .contains("Content-Type: application/javascript; charset=utf-8"));
        assert!(bridge.body.contains("window.magpie = api"));
        assert!(bridge.body.contains("window.aiToolboxPlugin = api"));

        let asset = request(address, "GET", &format!("/_plugin/{token}/app.js")).await;
        assert_eq!(asset.status, 200);
        assert!(asset
            .headers
            .contains("Content-Type: application/javascript; charset=utf-8"));
        assert_eq!(asset.body, "console.log('page')");

        let root_page = request(address, "GET", &format!("/_plugin/{token}/")).await;
        assert_eq!(root_page.status, 200);
        assert!(root_page.body.contains("<h1>page</h1>"));

        let head = request(address, "HEAD", &format!("/_plugin/{token}/index.html")).await;
        assert_eq!(head.status, 200);
        assert!(head.body.is_empty());
        assert!(head.headers.contains("Content-Length: 58"));
    }

    #[tokio::test]
    async fn rejects_path_traversal_and_unknown_tokens() {
        let (_root, entry) = page_fixture();
        let server = Arc::new(PluginPageServer::new());
        let token = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        let address = server.ensure_started().await.unwrap();

        for path in [
            format!("/_plugin/{token}/../outside.txt"),
            format!("/_plugin/{token}/%2e%2e/outside.txt"),
            format!("/_plugin/{token}/%2E%2E%2Foutside.txt"),
            format!("/_plugin/{token}/..%5coutside.txt"),
        ] {
            let response = request(address, "GET", &path).await;
            assert!(
                response.status == 403 || response.status == 404,
                "{path} returned {}",
                response.status
            );
        }

        let unknown = request(
            address,
            "GET",
            "/_plugin/00000000000000000000000000000000/index.html",
        )
        .await;
        assert_eq!(unknown.status, 404);

        let missing = request(address, "GET", &format!("/_plugin/{token}/missing.js")).await;
        assert_eq!(missing.status, 404);

        let post = request(address, "POST", &format!("/_plugin/{token}/index.html")).await;
        assert_eq!(post.status, 405);
        assert!(post.headers.contains("Allow: GET, HEAD"));
    }

    #[tokio::test]
    async fn allows_only_the_declared_service_in_the_page_policy() {
        let (_root, entry) = page_fixture();
        let server = Arc::new(PluginPageServer::new());
        let token = server
            .register_page("plugin-a", "default", &entry, Some(3477))
            .unwrap();
        let address = server.ensure_started().await.unwrap();

        let html = request(address, "GET", &format!("/_plugin/{token}/index.html")).await;
        assert_eq!(html.status, 200);
        assert!(html
            .headers
            .contains("connect-src http://127.0.0.1:3477 ws://127.0.0.1:3477"));

        let asset = request(address, "GET", &format!("/_plugin/{token}/icon.svg")).await;
        assert_eq!(asset.status, 200);
        assert!(asset.headers.contains("Content-Type: image/svg+xml"));
        // SVG can carry script: it gets a locked-down policy of its own.
        assert!(asset.headers.contains("Content-Security-Policy: sandbox;"));
        assert!(asset.headers.contains("connect-src 'none'"));
    }

    #[tokio::test]
    async fn reopening_a_page_replaces_its_token_and_revoking_one_page_keeps_the_other() {
        let (_root, entry) = page_fixture();
        let server = Arc::new(PluginPageServer::new());
        let first = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        let other = server
            .register_page("plugin-a", "other", &entry, None)
            .unwrap();
        let address = server.ensure_started().await.unwrap();

        // A second open of the same page mints a new token and drops the old.
        let second = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        assert_ne!(second, first);
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{first}/index.html"))
                .await
                .status,
            404
        );
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{second}/index.html"))
                .await
                .status,
            200
        );
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{other}/index.html"))
                .await
                .status,
            200
        );

        server.revoke_plugin_page("plugin-a", "default");
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{second}/index.html"))
                .await
                .status,
            404
        );
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{other}/index.html"))
                .await
                .status,
            200
        );
    }

    #[tokio::test]
    async fn revoking_a_plugin_makes_its_tokens_404() {
        let (_root, entry) = page_fixture();
        let server = Arc::new(PluginPageServer::new());
        let token = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        let address = server.ensure_started().await.unwrap();

        assert_eq!(
            request(address, "GET", &format!("/_plugin/{token}/index.html"))
                .await
                .status,
            200
        );
        server.revoke_plugin("plugin-a");
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{token}/index.html"))
                .await
                .status,
            404
        );

        // Reopening mints a new token.
        let second = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        assert_ne!(second, token);
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{second}/index.html"))
                .await
                .status,
            200
        );
    }

    #[tokio::test]
    async fn sync_revokes_disabled_plugins_and_unknown_pages() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let (_root, entry) = page_fixture();
        let server = Arc::new(PluginPageServer::new());
        let token = server
            .register_page("plugin-a", "default", &entry, None)
            .unwrap();
        let address = server.ensure_started().await.unwrap();

        let record = InstalledPluginRecord {
            id: "plugin-a".to_string(),
            package_name: "plugin-a".to_string(),
            enabled: true,
            pages: vec![PluginPageManifest {
                id: "default".to_string(),
                entry_file: "index.html".to_string(),
                ..PluginPageManifest::default()
            }],
            ..InstalledPluginRecord::default()
        };
        store::upsert_installed_plugin(&db, &record).unwrap();
        server.sync_enabled_plugins(&db).unwrap();
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{token}/index.html"))
                .await
                .status,
            200
        );

        store::set_plugin_enabled(&db, "plugin-a", false).unwrap();
        server.sync_enabled_plugins(&db).unwrap();
        assert_eq!(
            request(address, "GET", &format!("/_plugin/{token}/index.html"))
                .await
                .status,
            404
        );
    }

    #[test]
    fn percent_decoding_rejects_bad_input() {
        assert_eq!(percent_decode("a%20b"), Some("a b".to_string()));
        assert_eq!(percent_decode("%E4%B8%AD"), Some("中".to_string()));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("100%"), None);
        assert_eq!(percent_decode("%FF"), None);
    }

    #[test]
    fn mime_types_cover_the_static_formats_pages_use() {
        for (name, expected) in [
            ("index.html", "text/html; charset=utf-8"),
            ("app.js", "application/javascript; charset=utf-8"),
            ("app.mjs", "application/javascript; charset=utf-8"),
            ("app.css", "text/css; charset=utf-8"),
            ("icon.svg", "image/svg+xml"),
            ("icon.png", "image/png"),
            ("photo.JPG", "image/jpeg"),
            ("photo.webp", "image/webp"),
            ("anim.gif", "image/gif"),
            ("favicon.ico", "image/x-icon"),
            ("data.json", "application/json"),
            ("font.woff", "font/woff"),
            ("font.woff2", "font/woff2"),
            ("font.ttf", "font/ttf"),
            ("module.wasm", "application/wasm"),
            ("README", "application/octet-stream"),
        ] {
            assert_eq!(content_type_for(Path::new(name)), expected, "{name}");
        }
    }
}
