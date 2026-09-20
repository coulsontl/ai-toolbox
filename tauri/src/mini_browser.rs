//! Minimal embedded browser window for checking relay dashboards (API balance,
//! usage, backend data) without leaving the toolbox.
//!
//! Why a separate native webview instead of an `<iframe>` in the main window:
//! relay consoles almost always send `X-Frame-Options: DENY` (or a
//! `frame-ancestors` CSP), so an iframe renders a blank box. A top-level
//! navigation is not subject to those headers.
//!
//! Memory: the window is built on the platform webview the app already loads —
//! WebView2 on Windows, WebKitGTK on Linux, WKWebView on macOS — so nothing
//! ships a second browser engine. The window is created on first use and
//! destroyed when closed, so an unused toolbox pays nothing.
//!
//! Security: this browser loads third-party pages, so it must never be able to
//! reach the app's own commands. Two properties hold that line:
//! 1. It is created from Rust, and its labels (`mini-browser` and every
//!    `mini-browser-<profile>` derived from a saved account) are deliberately
//!    absent from `capabilities/default.json`, which grants access to the
//!    `main` *webview* only. The app's ACL therefore grants the browser no
//!    command access, standalone window and embedded child webview alike.
//! 2. Navigation is restricted to `http`/`https` by [`is_navigable_url`], so a
//!    page cannot walk the window into `file://` or `javascript:` territory.
//!
//! Two presentation modes share one implementation:
//! - a standalone window per account (`mini_browser_open`, the original mode);
//! - a child webview of the main window (`mini_browser_open_embedded`).
//! Both use the same label rule and the same data directory per account, so
//! switching modes keeps the login state.
//!
//! Multiple accounts: every saved account gets its own window/webview and its
//! own webview data directory, so two logins for the same relay never share
//! cookies. The profile id is part of both the label and the directory name, so
//! it is validated before either is built (see [`validate_profile`]).
//! That directory is handed to the platform webview as its own profile, and
//! WebView2 (Windows) and WKWebView (macOS) honour it differently: per-account
//! isolation is verified on Windows only, is unverified on macOS, and Windows
//! stays the acceptance target for this feature.

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

/// Window label used when a caller passes no profile id. Opening the browser
/// twice without a profile navigates that one window instead of stacking
/// duplicates, which keeps the legacy single-window behaviour.
pub const MINI_BROWSER_LABEL: &str = "mini-browser";

/// Prefix for profile-scoped labels: `mini-browser-<profile>`. One window and
/// one data directory per saved account.
pub const MINI_BROWSER_LABEL_PREFIX: &str = "mini-browser-";

/// Label of the window the app itself renders in. Embedded browsers are child
/// webviews of that window, which is why their label starts with the browser
/// prefix while their host window stays `main`.
const MAIN_WINDOW_LABEL: &str = "main";

/// Saved-page list is bounded so a runaway caller cannot grow it forever.
const MAX_URL_LEN: usize = 2048;

/// Profile ids become window labels *and* directory names, so they are limited
/// to the characters that are safe in both.
const MAX_PROFILE_LEN: usize = 64;

const INVALID_PROFILE_MESSAGE: &str = "Invalid browser profile id";

/// Bounded retry budget for deleting a profile directory. Windows keeps
/// WebView2's files locked briefly after its window is destroyed, so an
/// immediate `remove_dir_all` fails on any profile that has actually been used.
const PROFILE_CLEAR_ATTEMPTS: u32 = 20;
const PROFILE_CLEAR_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

/// Normalise user input into an absolute `http`/`https` URL.
///
/// A bare `relay.example.com/console` gets an `https://` prefix (the common
/// case when typing a host). Anything with a non-web scheme — `javascript:`,
/// `file:`, `data:` — is rejected rather than silently rewritten, because those
/// are the vectors that would let a pasted string escape the browser sandbox.
pub fn normalise_browser_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter a URL to open".to_string());
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err("That URL is too long".to_string());
    }
    // Reject control characters before parsing: they can be used to smuggle a
    // second value past a naive consumer.
    if trimmed.chars().any(char::is_control) {
        return Err("That URL contains control characters".to_string());
    }

    let has_scheme = trimmed.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    });
    let candidate = if has_scheme {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let parsed = tauri::Url::parse(&candidate).map_err(|error| format!("Invalid URL: {error}"))?;
    if !is_navigable_url(&parsed) {
        return Err("Only http and https addresses can be opened".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("That URL is missing a host".to_string());
    }
    Ok(parsed.to_string())
}

/// Whether the mini browser may navigate to `url`.
///
/// Used both for user input and as the window's `on_navigation` guard, so a
/// link clicked inside the loaded page cannot escape to `file://`.
fn is_navigable_url(url: &tauri::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Base profile directory so relay cookies survive restarts without mixing with
/// the toolbox's own webview storage. Per-account directories are children of
/// this folder.
fn browser_data_dir() -> std::path::PathBuf {
    crate::app_paths::resolved_data_dir().join("mini-browser")
}

/// Reject anything that is not a plain lowercase id.
///
/// The profile id is concatenated into a window label and a directory name, so
/// this is the guard that keeps `../` (and Windows separators, reserved device
/// names' punctuation, and absolute paths) out of both.
fn validate_profile(profile: &str) -> Result<(), String> {
    let valid_length = !profile.is_empty() && profile.len() <= MAX_PROFILE_LEN;
    let valid_chars = profile
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid_length && valid_chars {
        Ok(())
    } else {
        Err(INVALID_PROFILE_MESSAGE.to_string())
    }
}

/// `mini-browser-<profile>`, or the legacy `mini-browser` when no profile is
/// given. Callers must validate `profile` first.
fn window_label(profile: Option<&str>) -> String {
    match profile {
        Some(profile) => format!("{MINI_BROWSER_LABEL_PREFIX}{profile}"),
        None => MINI_BROWSER_LABEL.to_string(),
    }
}

/// Validate an optional profile coming from the frontend.
fn checked_profile(profile: Option<&str>) -> Result<Option<&str>, String> {
    if let Some(profile) = profile {
        validate_profile(profile)?;
    }
    Ok(profile)
}

/// Per-account webview data directory (cookies, storage, cache).
fn profile_data_dir(profile: &str) -> std::path::PathBuf {
    browser_data_dir().join(profile)
}

/// What the frontend needs to render one tab in its tab strip.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MiniBrowserWindowInfo {
    pub profile_id: Option<String>,
    pub label: String,
    pub title: String,
    pub url: String,
}

/// Every open mini browser window, profile-scoped and legacy alike.
fn open_window_infos<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Vec<MiniBrowserWindowInfo> {
    let mut windows = app
        // Manager::webviews() rather than webview_windows(): an embedded
        // browser is a child webview of `main`, which webview_windows() cannot
        // see. Both modes are just webviews whose label carries the prefix.
        .webviews()
        .into_iter()
        .filter(|(label, _)| label.starts_with(MINI_BROWSER_LABEL))
        .map(|(label, webview)| {
            let url = webview.url().map(|url| url.to_string()).unwrap_or_default();
            let window = webview.window();
            // A standalone browser owns its window, so the window title follows
            // the page (with the address as a fallback, so the tab strip never
            // renders an empty row). An embedded browser shares the main window,
            // which has no per-browser title, so the address is the label.
            let title = if window.label() == label {
                window.title().unwrap_or_else(|_| url.clone())
            } else {
                url.clone()
            };
            let profile_id = label
                .strip_prefix(MINI_BROWSER_LABEL_PREFIX)
                .map(|profile| profile.to_string());
            MiniBrowserWindowInfo {
                profile_id,
                label,
                title,
                url,
            }
        })
        .collect::<Vec<_>>();
    windows.sort_by(|left, right| left.label.cmp(&right.label));
    windows
}

/// Close one mini browser for its validated profile label, whichever mode it is
/// in.
///
/// A standalone browser is its own window, so the window is closed (which tears
/// down both). An embedded browser is a child webview of the main window: only
/// that webview is closed, which is what "close this tab" means there. In both
/// cases the webview leaves the manager, so the profile stops being reported as
/// open.
fn close_browser<R: tauri::Runtime>(app: &tauri::AppHandle<R>, label: &str) -> Result<(), String> {
    let Some(webview) = app.get_webview(label) else {
        return Ok(());
    };
    let window = webview.window();
    if window.label() == label {
        window
            .close()
            .map_err(|error| format!("Failed to close the browser window: {error}"))
    } else {
        webview
            .close()
            .map_err(|error| format!("Failed to close the embedded browser: {error}"))
    }
}

/// Bring an existing window forward, or create it with its own data directory.
fn open_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    url: tauri::Url,
    profile: Option<&str>,
) -> Result<(), String> {
    let profile = checked_profile(profile)?;
    let label = window_label(profile);

    // Reuse the existing browser when present: one browser per account, and the
    // legacy no-profile window keeps its old single-window semantics. This also
    // covers the embedded mode: a browser already embedded in the main window is
    // navigated rather than duplicated (a second webview with the same label
    // would be rejected).
    if let Some(existing) = app.get_webview(&label) {
        existing
            .navigate(url)
            .map_err(|error| format!("Failed to navigate the browser: {error}"))?;
        let window = existing.window();
        if window.label() == label {
            // Standalone: the webview fills its own window, so bring the window
            // forward.
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        } else {
            // Embedded: the page shares the main window, so leave its bounds and
            // visibility to whoever positioned it (the embedded commands).
            let _ = window.set_focus();
        }
        return Ok(());
    }

    let data_directory = match profile {
        Some(profile) => profile_data_dir(profile),
        None => browser_data_dir(),
    };

    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title("AI Toolbox Browser")
        .inner_size(1100.0, 800.0)
        .min_inner_size(480.0, 360.0)
        .center()
        .data_directory(data_directory)
        // Reject non-web schemes here too: this also covers redirects and
        // `target=_blank` handled by the webview itself, so a page cannot walk
        // the window into `file://` or `javascript:`.
        .on_navigation(is_navigable_url);

    let window = builder
        .build()
        .map_err(|error| format!("Failed to open the browser window: {error}"))?;
    let _ = window.set_focus();
    Ok(())
}

/// Close one window by its validated profile label.
fn close_profile_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    profile: Option<&str>,
) -> Result<(), String> {
    let profile = checked_profile(profile)?;
    close_browser(app, &window_label(profile))
}

/// Open a page in the embedded browser, creating the window if needed.
///
/// `profile_id` selects the account: each profile gets its own window and its
/// own login state. Omitting it keeps the legacy single-window behaviour.
#[tauri::command]
pub async fn mini_browser_open<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    url: String,
    profile_id: Option<String>,
) -> Result<(), String> {
    let normalised = normalise_browser_url(&url)?;
    let parsed = tauri::Url::parse(&normalised).map_err(|error| format!("Invalid URL: {error}"))?;
    open_window(&app, parsed, profile_id.as_deref())
}

/// Navigate the already-open browser window.
#[tauri::command]
pub async fn mini_browser_navigate<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    url: String,
    profile_id: Option<String>,
) -> Result<(), String> {
    mini_browser_open(app, url, profile_id).await
}

/// Address currently shown, or `None` when the window is not open.
#[tauri::command]
pub fn mini_browser_current_url<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: Option<String>,
) -> Result<Option<String>, String> {
    let profile = checked_profile(profile_id.as_deref())?;
    // get_webview, not get_webview_window: an embedded browser is a child
    // webview of `main`, and the URL is a webview property either way.
    let Some(webview) = app.get_webview(&window_label(profile)) else {
        return Ok(None);
    };
    webview
        .url()
        .map(|url| Some(url.to_string()))
        .map_err(|error| format!("Failed to read the browser address: {error}"))
}

/// Whether the browser window for this profile currently exists.
#[tauri::command]
pub fn mini_browser_is_open<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: Option<String>,
) -> bool {
    checked_profile(profile_id.as_deref())
        .map(|profile| app.get_webview(&window_label(profile)).is_some())
        .unwrap_or(false)
}

/// Close every mini browser window (legacy semantics: "close the browser").
#[tauri::command]
pub fn mini_browser_close<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    let labels = app
        // webviews(), not webview_windows(): the latter misses embedded child
        // webviews, which "close the browser" must close too.
        .webviews()
        .into_keys()
        .filter(|label| label.starts_with(MINI_BROWSER_LABEL))
        .collect::<Vec<_>>();
    for label in labels {
        close_browser(&app, &label)?;
    }
    Ok(())
}

/// Close the window belonging to one account.
#[tauri::command]
pub fn mini_browser_close_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
) -> Result<(), String> {
    close_profile_window(&app, Some(&profile_id))
}

/// Embedded mode: the browser is a child webview of the main window instead of
/// its own top-level window. Everything else — profile validation, the label
/// rule, the per-account data directory, the http/https navigation guard — is
/// shared with the standalone mode above, so switching modes keeps the login
/// state and the frontend contract.
///
/// Look up the webview for a validated profile *only* if it is hosted by the
/// main window. A same-labelled standalone browser window is a different thing:
/// it cannot be repositioned inside another window.
fn embedded_webview<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    label: &str,
) -> Option<tauri::Webview<R>> {
    let webview = app.get_webview(label)?;
    (webview.window().label() == MAIN_WINDOW_LABEL).then_some(webview)
}

/// Bounds of an embedded browser, in logical units relative to the main
/// window's client area.
fn embedded_bounds(x: f64, y: f64, width: f64, height: f64) -> tauri::Rect {
    tauri::Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width, height).into(),
    }
}

/// Reposition the embedded browser for one account. Errors when that account
/// has no embedded browser (never opened, or closed).
#[tauri::command]
pub fn mini_browser_set_bounds<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);
    let Some(webview) = embedded_webview(&app, &label) else {
        return Err(format!("No embedded browser is open for '{profile_id}'"));
    };
    webview
        .set_bounds(embedded_bounds(x, y, width, height))
        .map_err(|error| format!("Failed to resize the embedded browser: {error}"))
}

/// Show or hide the embedded browser for one account. Hiding keeps the webview
/// (and its login state) alive, which is what the tab strip needs when it
/// switches accounts. Errors when that account has no embedded browser.
#[tauri::command]
pub fn mini_browser_set_visible<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
    visible: bool,
) -> Result<(), String> {
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);
    let Some(webview) = embedded_webview(&app, &label) else {
        return Err(format!("No embedded browser is open for '{profile_id}'"));
    };
    if visible {
        webview
            .show()
            .map_err(|error| format!("Failed to show the embedded browser: {error}"))
    } else {
        webview
            .hide()
            .map_err(|error| format!("Failed to hide the embedded browser: {error}"))
    }
}

/// Open a page in a child webview of the main window, creating it if needed.
///
/// Async for the same reason as [`mini_browser_open`]: creating a webview from
/// a synchronous command can deadlock WebView2 on Windows.
#[tauri::command]
pub async fn mini_browser_open_embedded<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let normalised = normalise_browser_url(&url)?;
    let parsed = tauri::Url::parse(&normalised).map_err(|error| format!("Invalid URL: {error}"))?;
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);

    if let Some(existing) = embedded_webview(&app, &label) {
        existing
            .navigate(parsed)
            .map_err(|error| format!("Failed to navigate the embedded browser: {error}"))?;
        existing
            .set_bounds(embedded_bounds(x, y, width, height))
            .map_err(|error| format!("Failed to resize the embedded browser: {error}"))?;
        let _ = existing.show();
        return Ok(());
    }

    // A standalone browser window may already own this profile. Reuse it rather
    // than building a second webview on a label that is already taken (that
    // would fail with `WebviewLabelAlreadyExists`).
    if let Some(standalone) = app.get_webview(&label) {
        standalone
            .navigate(parsed)
            .map_err(|error| format!("Failed to navigate the browser window: {error}"))?;
        let _ = standalone.show();
        let _ = standalone.window().set_focus();
        return Ok(());
    }

    let Some(main) = crate::main_window(&app) else {
        return Err("The main window is not available".to_string());
    };

    let builder = tauri::WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        // Same directory rule as the standalone window, so a profile keeps its
        // cookies when the presentation mode changes.
        .data_directory(profile_data_dir(&profile_id))
        // Reject non-web schemes here too: this also covers redirects and
        // `target=_blank`, so the page cannot walk itself into `file://`.
        .on_navigation(is_navigable_url);

    main.add_child(
        builder,
        LogicalPosition::new(x, y),
        LogicalSize::new(width, height),
    )
    .map_err(|error| format!("Failed to open the embedded browser: {error}"))?;
    Ok(())
}

/// Show and focus the window belonging to one account.
#[tauri::command]
pub fn mini_browser_focus_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
) -> Result<(), String> {
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);
    let Some(webview) = app.get_webview(&label) else {
        return Ok(());
    };
    let window = webview.window();
    if window.label() != label {
        // Embedded: the page shares the main window, so "focus" can only reveal
        // it and bring that window forward.
        let _ = webview.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

/// Every open mini browser window, so the panel can render its tab strip.
#[tauri::command]
pub fn mini_browser_list_windows<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Vec<MiniBrowserWindowInfo> {
    open_window_infos(&app)
}

/// Forget an account's login state: close its window, then delete its profile
/// directory. A missing directory counts as success.
///
/// Async so the retries below never block the UI thread: after the window is
/// destroyed Windows still holds WebView2's files for a moment, and a single
/// immediate delete fails on every profile that has actually been used.
#[tauri::command]
pub async fn mini_browser_clear_profile<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
) -> Result<(), String> {
    let profile = checked_profile(Some(&profile_id))?;
    close_profile_window(&app, profile)?;
    let directory = profile_data_dir(&profile_id);
    let mut last_error = None;
    for attempt in 0..PROFILE_CLEAR_ATTEMPTS {
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => return Ok(()),
            // Already gone (never opened, or cleared before): success.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < PROFILE_CLEAR_ATTEMPTS {
                    tokio::time::sleep(PROFILE_CLEAR_RETRY_DELAY).await;
                }
            }
        }
    }
    Err(format!(
        "Failed to clear the browser profile: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "the directory is still in use".to_string())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_adds_https_to_a_bare_host() {
        assert_eq!(
            normalise_browser_url("relay.example.com").unwrap(),
            "https://relay.example.com/"
        );
        assert_eq!(
            normalise_browser_url("  relay.example.com/console  ").unwrap(),
            "https://relay.example.com/console"
        );
        assert_eq!(
            normalise_browser_url("http://127.0.0.1:3000/balance").unwrap(),
            "http://127.0.0.1:3000/balance"
        );
    }

    #[test]
    fn normalise_keeps_an_explicit_scheme() {
        assert_eq!(
            normalise_browser_url("https://api.example.com/usage?tab=credits").unwrap(),
            "https://api.example.com/usage?tab=credits"
        );
    }

    #[test]
    fn normalise_rejects_non_web_schemes() {
        // These are the strings that would otherwise escape the sandbox.
        assert!(normalise_browser_url("javascript:alert(1)").is_err());
        assert!(normalise_browser_url("file:///C:/Windows/win.ini").is_err());
        assert!(normalise_browser_url("data:text/html,<h1>x</h1>").is_err());
        assert!(normalise_browser_url("ms-settings:privacy").is_err());
        assert!(normalise_browser_url("").is_err());
        assert!(normalise_browser_url("   ").is_err());
    }

    #[test]
    fn normalise_rejects_control_characters_and_overlong_input() {
        assert!(normalise_browser_url("https://ok.example.com/\nheader").is_err());
        assert!(
            normalise_browser_url(&format!("https://example.com/{}", "a".repeat(3000))).is_err()
        );
    }

    #[test]
    fn navigable_url_guard_allows_only_web_schemes() {
        assert!(is_navigable_url(
            &tauri::Url::parse("https://a.example.com").unwrap()
        ));
        assert!(is_navigable_url(
            &tauri::Url::parse("http://a.example.com").unwrap()
        ));
        assert!(!is_navigable_url(
            &tauri::Url::parse("file:///etc/passwd").unwrap()
        ));
        assert!(!is_navigable_url(
            &tauri::Url::parse("javascript:alert(1)").unwrap()
        ));
    }

    #[test]
    fn validate_profile_accepts_lowercase_ids() {
        assert!(validate_profile("a").is_ok());
        assert!(validate_profile("site-1").is_ok());
        assert!(validate_profile("1234567890").is_ok());
        assert!(validate_profile("relay-account-2").is_ok());
        // Exactly at the boundary.
        assert!(validate_profile(&"a".repeat(MAX_PROFILE_LEN)).is_ok());
    }

    #[test]
    fn validate_profile_rejects_traversal_and_other_unsafe_ids() {
        // The reason this guard exists: the id is concatenated into a path.
        assert!(validate_profile("../escape").is_err());
        assert!(validate_profile("..").is_err());
        assert!(validate_profile("a/../b").is_err());
        assert!(validate_profile("a/b").is_err());
        assert!(validate_profile("a\\b").is_err());
        assert!(validate_profile("C:\\Windows").is_err());
        assert!(validate_profile("").is_err());
        assert!(validate_profile("UPPER").is_err());
        assert!(validate_profile("with space").is_err());
        assert!(validate_profile("under_score").is_err());
        assert!(validate_profile("has.dot").is_err());
        assert!(validate_profile("null\0byte").is_err());
        assert!(validate_profile("站点").is_err());
        // One past the boundary.
        assert!(validate_profile(&"a".repeat(MAX_PROFILE_LEN + 1)).is_err());
    }

    #[test]
    fn window_label_keeps_the_legacy_name_without_a_profile() {
        assert_eq!(window_label(None), MINI_BROWSER_LABEL);
        assert_eq!(window_label(Some("site-1")), "mini-browser-site-1");
        assert_eq!(
            window_label(Some("site-1")),
            format!("{MINI_BROWSER_LABEL_PREFIX}site-1")
        );
    }

    #[test]
    fn checked_profile_rejects_invalid_ids_before_use() {
        assert_eq!(checked_profile(None).unwrap(), None);
        assert_eq!(checked_profile(Some("site-1")).unwrap(), Some("site-1"));
        assert_eq!(
            checked_profile(Some("../escape")).unwrap_err(),
            INVALID_PROFILE_MESSAGE
        );
    }
}
