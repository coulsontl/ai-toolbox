use bytes::Bytes;
use futures_util::Stream;
use http_body_util::BodyExt;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_LENGTH, HOST};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Clone)]
pub(super) struct PreservedHeader {
    pub(super) name: String,
    pub(super) value: HeaderValue,
}

pub(super) const MAX_HEADER_PRESERVING_BODY_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

type HyperClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Full<Bytes>,
>;

/// Lazily-initialized hyper-util client with header-case preservation enabled.
///
/// Used as the fallback when the header-preserving raw-write path fails. Uses the
/// same root certificate store as `global_tls_connector()` (webpki + native
/// certs) so MITM proxy CAs installed in the system keychain are trusted.
pub(super) fn global_hyper_client() -> &'static HyperClient {
    static CLIENT: OnceLock<HyperClient> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        let (added, ignored) = root_store.add_parsable_certificates(native.certs);
        if ignored > 0 {
            log::debug!(
                "Skipped {} native TLS certificates while building hyper-util client root store",
                ignored
            );
        }
        log::debug!(
            "Gateway hyper-util fallback TLS root store loaded {added} native certificates"
        );
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = HttpsConnectorBuilder::new()
            .with_tls_config(config)
            .https_or_http()
            .enable_http1()
            .build();
        Client::builder(TokioExecutor::new())
            .http1_preserve_header_case(true)
            .http1_title_case_headers(true)
            .build(connector)
    })
}

pub(super) struct HeaderPreservingResponse {
    status: reqwest::StatusCode,
    headers: HeaderMap,
    body: hyper::body::Incoming,
    /// Remaining overall budget for non-stream body reads after headers arrived.
    body_timeout: Duration,
    /// Per-chunk idle timeout for SSE/stream reads. Must honor user settings and
    /// must NOT be capped by a hardcoded 60s ceiling.
    stream_idle_timeout: Duration,
}

impl HeaderPreservingResponse {
    pub(super) fn status(&self) -> reqwest::StatusCode {
        self.status
    }

    pub(super) fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub(super) async fn bytes(self) -> Result<Vec<u8>, String> {
        let timeout = if self.body_timeout.is_zero() {
            DEFAULT_BODY_IDLE_TIMEOUT
        } else {
            self.body_timeout
        };
        let collected = tokio::time::timeout(timeout, async {
            let mut body = self.body;
            let mut out = Vec::new();
            loop {
                match body.frame().await {
                    Some(Ok(frame)) => {
                        if let Ok(data) = frame.into_data() {
                            if out.len().saturating_add(data.len())
                                > MAX_HEADER_PRESERVING_BODY_BYTES
                            {
                                return Err(format!(
                                    "Upstream response body exceeded {} bytes",
                                    MAX_HEADER_PRESERVING_BODY_BYTES
                                ));
                            }
                            out.extend_from_slice(&data);
                        }
                    }
                    Some(Err(error)) => {
                        return Err(format!("Failed to read upstream response body: {error}"));
                    }
                    None => return Ok(out),
                }
            }
        })
        .await
        .map_err(|_| {
            format!(
                "Timed out reading upstream response body after {} seconds",
                timeout.as_secs()
            )
        })??;
        Ok(collected)
    }

    pub(super) fn bytes_stream(
        self,
    ) -> impl Stream<Item = Result<Vec<u8>, String>> + Send + 'static {
        let idle = if self.stream_idle_timeout.is_zero() {
            DEFAULT_BODY_IDLE_TIMEOUT
        } else {
            self.stream_idle_timeout
        };
        futures_util::stream::unfold((self.body, idle), |(mut body, idle)| async move {
            loop {
                match tokio::time::timeout(idle, body.frame()).await {
                    Ok(Some(Ok(frame))) => {
                        if let Ok(data) = frame.into_data() {
                            if data.is_empty() {
                                continue;
                            }
                            return Some((Ok(data.to_vec()), (body, idle)));
                        }
                    }
                    Ok(Some(Err(error))) => {
                        return Some((
                            Err(format!("Failed to read upstream response body: {error}")),
                            (body, idle),
                        ));
                    }
                    Ok(None) => return None,
                    Err(_) => {
                        return Some((
                            Err(format!(
                                "Timed out waiting for upstream stream chunk after {} seconds",
                                idle.as_secs()
                            )),
                            (body, idle),
                        ));
                    }
                }
            }
        })
    }
}

pub(super) fn append_preserved_header(
    headers: &mut HeaderMap,
    preserved_headers: &mut Vec<PreservedHeader>,
    name: impl Into<String>,
    value: HeaderValue,
) -> Result<(), String> {
    let name = name.into();
    let header_name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|error| format!("Invalid request header name '{}': {error}", name))?;
    headers.insert(header_name, value.clone());
    preserved_headers.push(PreservedHeader { name, value });
    Ok(())
}

/// Like `append_preserved_header` but appends instead of replacing, so multiple
/// values for the same header survive (used by header `rename`/`copy` ops).
pub(super) fn append_preserved_header_value(
    headers: &mut HeaderMap,
    preserved_headers: &mut Vec<PreservedHeader>,
    name: &str,
    value: HeaderValue,
) -> Result<(), String> {
    let header_name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|error| format!("Invalid request header name '{name}': {error}"))?;
    headers.append(header_name, value.clone());
    preserved_headers.push(PreservedHeader {
        name: name.to_string(),
        value,
    });
    Ok(())
}

#[derive(Debug)]
pub(super) enum HeaderPreservingError {
    /// Connection/handshake failed before any request bytes were written.
    /// Safe to fall back to reqwest without risking a double-send.
    PreWrite { message: String },
    /// Request body was already written to the upstream socket.
    /// Must not fall back to reqwest (would re-execute the LLM request).
    PostWrite { message: String },
}

impl HeaderPreservingError {
    pub(super) fn message(&self) -> &str {
        match self {
            Self::PreWrite { message } | Self::PostWrite { message } => message,
        }
    }

    pub(super) fn request_body_written(&self) -> bool {
        matches!(self, Self::PostWrite { .. })
    }
}

pub(super) async fn send_header_preserving_request(
    upstream_url: &reqwest::Url,
    method: reqwest::Method,
    preserved_headers: &[PreservedHeader],
    body: Vec<u8>,
    timeout: Duration,
    stream_idle_timeout: Duration,
    proxy_url: Option<&str>,
) -> Result<HeaderPreservingResponse, HeaderPreservingError> {
    let started = std::time::Instant::now();
    let future = send_raw_request(upstream_url, method, preserved_headers, &body, proxy_url);
    let mut response = tokio::time::timeout(timeout, future).await.map_err(|_| {
        HeaderPreservingError::PreWrite {
            message: format!(
                "Timed out sending upstream request after {} seconds",
                timeout.as_secs()
            ),
        }
    })??;
    let elapsed = started.elapsed();
    response.body_timeout = timeout.saturating_sub(elapsed);
    if response.body_timeout.is_zero() {
        response.body_timeout = Duration::from_secs(1);
    }
    response.stream_idle_timeout = if stream_idle_timeout.is_zero() {
        DEFAULT_BODY_IDLE_TIMEOUT
    } else {
        stream_idle_timeout
    };
    Ok(response)
}

async fn send_raw_request(
    upstream_url: &reqwest::Url,
    method: reqwest::Method,
    preserved_headers: &[PreservedHeader],
    body: &[u8],
    proxy_url: Option<&str>,
) -> Result<HeaderPreservingResponse, HeaderPreservingError> {
    use tokio::io::AsyncWriteExt;

    let scheme = upstream_url.scheme();
    let host = upstream_url
        .host_str()
        .ok_or_else(|| HeaderPreservingError::PreWrite {
            message: "Upstream URL has no host".to_string(),
        })?;
    let port = upstream_url
        .port()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    let mut path_and_query = upstream_url.path().to_string();
    if path_and_query.is_empty() {
        path_and_query.push('/');
    }
    if let Some(query) = upstream_url.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }
    let authority = authority_for_host_header(upstream_url, host, port);
    let raw_request = build_raw_request(
        &method,
        &path_and_query,
        &authority,
        preserved_headers,
        body,
    );

    let stream =
        if let Some(proxy_url) = proxy_url {
            connect_via_proxy(proxy_url, host, port)
                .await
                .map_err(|message| HeaderPreservingError::PreWrite { message })?
        } else {
            ProxyStream::Tcp(
                connect_direct_happy_eyeballs(host, port)
                    .await
                    .map_err(|message| HeaderPreservingError::PreWrite { message })?,
            )
        };

    if scheme == "https" {
        let tls_connector = global_tls_connector();
        let tls_host = host.trim_start_matches('[').trim_end_matches(']');
        let server_name =
            rustls::pki_types::ServerName::try_from(tls_host.to_string()).map_err(|error| {
                HeaderPreservingError::PreWrite {
                    message: format!("Invalid upstream server name: {error}"),
                }
            })?;
        let mut tls_stream = tls_connector
            .connect(server_name, stream)
            .await
            .map_err(|error| HeaderPreservingError::PreWrite {
                message: format!("TLS handshake failed: {error}"),
            })?;
        // From here on, request bytes may reach the upstream.
        tls_stream.write_all(&raw_request).await.map_err(|error| {
            HeaderPreservingError::PostWrite {
                message: format!("Write failed: {error}"),
            }
        })?;
        tls_stream
            .flush()
            .await
            .map_err(|error| HeaderPreservingError::PostWrite {
                message: format!("Flush failed: {error}"),
            })?;
        parse_response_from_stream(WriteFilter::new(tls_stream), method)
            .await
            .map_err(|message| HeaderPreservingError::PostWrite { message })
    } else {
        let mut stream = stream;
        stream
            .write_all(&raw_request)
            .await
            .map_err(|error| HeaderPreservingError::PostWrite {
                message: format!("Write failed: {error}"),
            })?;
        stream
            .flush()
            .await
            .map_err(|error| HeaderPreservingError::PostWrite {
                message: format!("Flush failed: {error}"),
            })?;
        parse_response_from_stream(WriteFilter::new(stream), method)
            .await
            .map_err(|message| HeaderPreservingError::PostWrite { message })
    }
}

fn authority_for_host_header(upstream_url: &reqwest::Url, host: &str, port: u16) -> String {
    match upstream_url.port() {
        Some(_) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

fn build_raw_request(
    method: &reqwest::Method,
    path_and_query: &str,
    authority: &str,
    preserved_headers: &[PreservedHeader],
    body: &[u8],
) -> Vec<u8> {
    let request_target = if path_and_query.is_empty() {
        "/"
    } else {
        path_and_query
    };
    let mut raw = Vec::with_capacity(4096 + body.len());
    raw.extend_from_slice(method.as_str().as_bytes());
    raw.extend_from_slice(b" ");
    raw.extend_from_slice(request_target.as_bytes());
    raw.extend_from_slice(b" HTTP/1.1\r\n");
    raw.extend_from_slice(b"Host: ");
    raw.extend_from_slice(authority.as_bytes());
    raw.extend_from_slice(b"\r\n");

    for header in preserved_headers {
        if header.name.eq_ignore_ascii_case(HOST.as_str())
            || header.name.eq_ignore_ascii_case(CONTENT_LENGTH.as_str())
        {
            continue;
        }
        raw.extend_from_slice(header.name.as_bytes());
        raw.extend_from_slice(b": ");
        raw.extend_from_slice(header.value.as_bytes());
        raw.extend_from_slice(b"\r\n");
    }

    raw.extend_from_slice(b"Content-Length: ");
    raw.extend_from_slice(body.len().to_string().as_bytes());
    raw.extend_from_slice(b"\r\n\r\n");
    raw.extend_from_slice(body);
    raw
}

async fn parse_response_from_stream<S>(
    stream: WriteFilter<S>,
    method: reqwest::Method,
) -> Result<HeaderPreservingResponse, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io = hyper_util::rt::TokioIo::new(stream);
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .handshake::<_, http_body_util::Full<Bytes>>(io)
        .await
        .map_err(|error| format!("HTTP response handshake failed: {error}"))?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            log::debug!("header-preserving upstream connection closed with error: {error}");
        }
    });

    let dummy_request = hyper::Request::builder()
        .method(method)
        .uri("/")
        .body(http_body_util::Full::new(Bytes::new()))
        .map_err(|error| format!("Failed to build response parser request: {error}"))?;
    let response = sender
        .send_request(dummy_request)
        .await
        .map_err(|error| format!("Failed to parse upstream response: {error}"))?;
    let (parts, body) = response.into_parts();
    Ok(HeaderPreservingResponse {
        status: parts.status,
        headers: parts.headers,
        body,
        body_timeout: DEFAULT_BODY_IDLE_TIMEOUT,
        stream_idle_timeout: DEFAULT_BODY_IDLE_TIMEOUT,
    })
}

enum ProxyStream {
    Tcp(tokio::net::TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>),
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ProxyStream::Tcp(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
            ProxyStream::Tls(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ProxyStream::Tcp(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
            ProxyStream::Tls(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ProxyStream::Tcp(stream) => std::pin::Pin::new(stream).poll_flush(cx),
            ProxyStream::Tls(stream) => std::pin::Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            ProxyStream::Tcp(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
            ProxyStream::Tls(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
        }
    }
}

async fn connect_via_proxy(
    proxy_url: &str,
    target_host: &str,
    target_port: u16,
) -> Result<ProxyStream, String> {
    use base64::Engine;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let parsed =
        reqwest::Url::parse(proxy_url).map_err(|error| format!("Invalid proxy URL: {error}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!(
            "Header-preserving proxy path does not support {} proxies",
            parsed.scheme()
        ));
    }

    let proxy_host = parsed
        .host_str()
        .ok_or_else(|| "Proxy URL has no host".to_string())?;
    let proxy_port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let tcp_stream = connect_direct_happy_eyeballs(proxy_host, proxy_port)
        .await
        .map_err(|error| format!("Proxy TCP connect failed: {error}"))?;
    let mut stream = if parsed.scheme() == "https" {
        let tls_connector = global_tls_connector();
        let tls_proxy_host = proxy_host.trim_start_matches('[').trim_end_matches(']');
        let server_name = rustls::pki_types::ServerName::try_from(tls_proxy_host.to_string())
            .map_err(|error| format!("Invalid proxy server name: {error}"))?;
        let tls_stream = tls_connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|error| format!("Proxy TLS handshake failed: {error}"))?;
        ProxyStream::Tls(Box::new(tls_stream))
    } else {
        ProxyStream::Tcp(tcp_stream)
    };

    let mut connect_request = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n"
    );
    if !parsed.username().is_empty() {
        let password = parsed.password().unwrap_or("");
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{password}", parsed.username()));
        connect_request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    connect_request.push_str("\r\n");
    stream
        .write_all(connect_request.as_bytes())
        .await
        .map_err(|error| format!("CONNECT write failed: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("CONNECT flush failed: {error}"))?;

    let mut reader = BufReader::new(&mut stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .map_err(|error| format!("CONNECT read failed: {error}"))?;
    if !status_line.contains(" 200 ") {
        return Err(format!("Proxy CONNECT rejected: {}", status_line.trim()));
    }
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("CONNECT header read failed: {error}"))?;
        if line.trim().is_empty() {
            break;
        }
    }
    drop(reader);
    Ok(stream)
}

fn global_tls_connector() -> &'static tokio_rustls::TlsConnector {
    static CONNECTOR: OnceLock<tokio_rustls::TlsConnector> = OnceLock::new();
    CONNECTOR.get_or_init(|| {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        let (added, ignored) = root_store.add_parsable_certificates(native.certs);
        if ignored > 0 {
            log::debug!(
                "Skipped {} native TLS certificates while building gateway root store",
                ignored
            );
        }
        log::debug!("Gateway header-preserving TLS root store loaded {added} native certificates");
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
    })
}

struct WriteFilter<S> {
    inner: S,
}

impl<S> WriteFilter<S> {
    fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for WriteFilter<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: Unpin> AsyncWrite for WriteFilter<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_request_preserves_forwarded_header_case() {
        let headers = vec![
            PreservedHeader {
                name: "X-Custom-Token".to_string(),
                value: HeaderValue::from_static("abc"),
            },
            PreservedHeader {
                name: "anthropic-version".to_string(),
                value: HeaderValue::from_static("2023-06-01"),
            },
        ];
        let raw = build_raw_request(
            &reqwest::Method::POST,
            "/v1/messages",
            "api.example.com",
            &headers,
            b"{}",
        );
        let raw_text = String::from_utf8(raw).expect("request bytes should be UTF-8 in test");
        assert!(raw_text.contains("X-Custom-Token: abc\r\n"));
        assert!(raw_text.contains("anthropic-version: 2023-06-01\r\n"));
        assert!(raw_text.contains("Content-Length: 2\r\n"));
    }

    #[tokio::test]
    async fn connect_direct_happy_eyeballs_reaches_ipv4_and_parses_bracketed_ipv6() {
        let v4 = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ipv4");
        let v4_port = v4.local_addr().expect("v4 addr").port();
        let stream = connect_direct_happy_eyeballs("127.0.0.1", v4_port)
            .await
            .expect("connect ipv4");
        assert_eq!(stream.peer_addr().expect("peer").port(), v4_port);

        // Verify bracketed IPv6 literal normalization without depending on host IPv6 firewall rules.
        let parsed_v6 = parse_literal_socket_addr("[::1]", 8443).expect("parse bracketed ipv6");
        assert!(parsed_v6.is_ipv6());
        assert_eq!(parsed_v6.port(), 8443);

        // Verify Happy Eyeballs falls through an immediate refusal to a working IPv4 listener.
        let closed = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind temp");
        let closed_addr = closed.local_addr().expect("closed addr");
        drop(closed);
        let live = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind live");
        let live_addr = live.local_addr().expect("live addr");
        let raced = connect_ordered_addresses(vec![closed_addr, live_addr])
            .await
            .expect("race should succeed on live address");
        assert_eq!(raced.peer_addr().expect("peer"), live_addr);
    }

    #[test]
    fn interleave_addresses_alternates_address_families() {
        let addrs: Vec<std::net::SocketAddr> = vec![
            "[::1]:443".parse().unwrap(),
            "[::2]:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap(),
            "127.0.0.2:443".parse().unwrap(),
        ];
        let ordered = interleave_address_families(addrs);
        assert!(ordered[0].is_ipv6());
        assert!(ordered[1].is_ipv4());
        assert!(ordered[2].is_ipv6());
        assert!(ordered[3].is_ipv4());
    }
}

fn parse_literal_socket_addr(host: &str, port: u16) -> Option<std::net::SocketAddr> {
    let normalized = host.trim_start_matches('[').trim_end_matches(']');
    normalized
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| std::net::SocketAddr::new(ip, port))
}

/// Open a direct TCP connection to `host:port` using RFC 8305 Happy Eyeballs
/// (300ms stagger between interleaved address families) so dead AAAA records
/// in front of working IPv4 addresses do not stall requests for 43s-150s.
async fn connect_direct_happy_eyeballs(
    host: &str,
    port: u16,
) -> Result<tokio::net::TcpStream, String> {
    if let Some(addr) = parse_literal_socket_addr(host, port) {
        return tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|error| format!("TCP connect failed: {error}"));
    }
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    let resolved: Vec<std::net::SocketAddr> = tokio::net::lookup_host((normalized_host, port))
        .await
        .map_err(|error| format!("DNS resolution failed for {host}: {error}"))?
        .collect();
    if resolved.is_empty() {
        return Err(format!("DNS resolution returned no addresses for {host}"));
    }
    connect_ordered_addresses(interleave_address_families(resolved)).await
}

async fn connect_ordered_addresses(
    ordered: Vec<std::net::SocketAddr>,
) -> Result<tokio::net::TcpStream, String> {
    const HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(300);

    if ordered.len() == 1 {
        return tokio::net::TcpStream::connect(ordered[0])
            .await
            .map_err(|error| format!("TCP connect failed: {error}"));
    }

    let mut attempts = tokio::task::JoinSet::new();
    let mut iter = ordered.into_iter();
    let mut last_error: Option<String> = None;

    if let Some(first) = iter.next() {
        attempts.spawn(async move { tokio::net::TcpStream::connect(first).await });
    }

    loop {
        if let Some(next_addr) = iter.next() {
            let sleep = tokio::time::sleep(HAPPY_EYEBALLS_DELAY);
            tokio::pin!(sleep);
            tokio::select! {
                joined = attempts.join_next(), if !attempts.is_empty() => {
                    match joined {
                        Some(Ok(Ok(stream))) => {
                            attempts.abort_all();
                            return Ok(stream);
                        }
                        Some(Ok(Err(error))) => {
                            last_error = Some(error.to_string());
                            // Failed immediately — start the next address right away.
                            attempts.spawn(async move { tokio::net::TcpStream::connect(next_addr).await });
                        }
                        Some(Err(join_error)) => {
                            last_error = Some(join_error.to_string());
                            attempts.spawn(async move { tokio::net::TcpStream::connect(next_addr).await });
                        }
                        None => {
                            attempts.spawn(async move { tokio::net::TcpStream::connect(next_addr).await });
                        }
                    }
                }
                () = &mut sleep => {
                    attempts.spawn(async move { tokio::net::TcpStream::connect(next_addr).await });
                }
            }
        } else {
            match attempts.join_next().await {
                Some(Ok(Ok(stream))) => {
                    attempts.abort_all();
                    return Ok(stream);
                }
                Some(Ok(Err(error))) => {
                    last_error = Some(error.to_string());
                }
                Some(Err(join_error)) => {
                    last_error = Some(join_error.to_string());
                }
                None => break,
            }
        }
    }

    Err(format!(
        "TCP connect failed: {}",
        last_error.unwrap_or_else(|| "no reachable address".to_string())
    ))
}

fn interleave_address_families(addrs: Vec<std::net::SocketAddr>) -> Vec<std::net::SocketAddr> {
    if addrs.len() <= 1 {
        return addrs;
    }
    let first_is_v6 = addrs[0].is_ipv6();
    let mut primary = Vec::new();
    let mut secondary = Vec::new();
    for addr in addrs {
        if addr.is_ipv6() == first_is_v6 {
            primary.push(addr);
        } else {
            secondary.push(addr);
        }
    }
    let mut out = Vec::with_capacity(primary.len() + secondary.len());
    let mut p_iter = primary.into_iter();
    let mut s_iter = secondary.into_iter();
    loop {
        let p = p_iter.next();
        let s = s_iter.next();
        if p.is_none() && s.is_none() {
            break;
        }
        if let Some(addr) = p {
            out.push(addr);
        }
        if let Some(addr) = s {
            out.push(addr);
        }
    }
    out
}
