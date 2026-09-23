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

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, OnceLock};

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

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

/// Where an embedded page sits, in physical pixels relative to the main
/// window's client area, plus whether it is shown.
///
/// This is the backend's *expected* placement: the target the last command
/// asked for, not a reading of the platform. Every placement command compares
/// against it first, so a repeated request costs a map lookup instead of a
/// `SetWindowPos` / `ShowWindow` on the main thread. The frontend runs a
/// verification loop against [`mini_browser_bounds`] for the other half of the
/// loop: a send that the platform refused stays visible as a mismatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Placement {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    visible: bool,
}

impl Placement {
    /// The four geometry fields, for comparing a `set_bounds` against the cache
    /// without the visibility flag (which only `set_visible` may change).
    fn same_geometry(&self, other: &Placement) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
    }
}

/// Whether a `set_bounds` can be dropped: the page already has this rectangle.
///
/// No entry at all means "unknown", never "already correct": a page the cache
/// has forgotten still has to be placed.
fn geometry_already_applied(current: Option<&Placement>, next: &Placement) -> bool {
    current.is_some_and(|current| current.same_geometry(next))
}

/// Whether a `set_visible` can be dropped: the page is already shown or hidden.
///
/// As above, no entry means unknown and the request goes through. Guessing
/// "already hidden" would leave the active tab invisible.
fn visibility_already_applied(current: Option<&Placement>, visible: bool) -> bool {
    current.is_some_and(|current| current.visible == visible)
}

/// Expected placement per webview label (both modes share the label rule).
///
/// A plain `Mutex` is enough: every critical section is a few map operations,
/// nothing is awaited while it is held, and the commands that read it already
/// run on a tokio worker (they are `async`).
static PLACEMENTS: OnceLock<Mutex<HashMap<String, Placement>>> = OnceLock::new();

/// Last address each webview was asked to load, so listing the open pages does
/// not have to call the blocking `Webview::url()` / `Window::title()` getters
/// (each of those is a synchronous round-trip to the main thread, and the list
/// is polled every couple of seconds).
static EMBEDDED_URLS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Labels whose embedded creation is in flight, and labels that were closed
/// while their creation was still running.
///
/// Both live under one lock because the interesting operation is atomic: "stop
/// marking this label as creating, and tell me whether it was closed meanwhile".
/// A webview cannot be closed before it exists, so without the second set a
/// "close" that arrives during creation would be lost and leave a page running
/// with nothing to control it.
#[derive(Default)]
struct CreationState {
    creating: HashSet<String>,
    close_pending: HashSet<String>,
}

static CREATION_STATE: OnceLock<Mutex<CreationState>> = OnceLock::new();

/// Serialises webview creation.
///
/// Creating a child webview blocks on the main thread (and pumps its message
/// loop while it does), so overlapping creations are exactly the reentrancy the
/// hang reports point at. One at a time, with the ID overlap handled by
/// [`CreationState`] instead.
///
/// It is also held by every other command that touches a webview's native
/// window. The lock is not really about creation: it is about the window's
/// message queue. While one webview is being created the main thread is inside
/// `wait_with_pump`, which dispatches whatever is already queued there, so a
/// placement or teardown that is in flight lands *inside* the creation. Taking
/// the lock before a command queues anything means nothing of ours is waiting in
/// that queue while a creation pumps it.
static CREATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How long a close waits for the main thread to run the teardown it queued.
///
/// Bounded on purpose: a wedged main thread must not turn closing a tab into a
/// second freeze — the caller (and the panel behind it) gives up and moves on
/// instead.
const MAIN_THREAD_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The [`CREATION_LOCK`] as seen from a command that is not creating a webview.
async fn native_guard() -> tokio::sync::MutexGuard<'static, ()> {
    CREATION_LOCK.lock().await
}

/// Wait until the main thread has run every task already queued for it.
///
/// Closing a webview only *queues* its native teardown: the manager forgets the
/// page immediately, so without this barrier a re-open of the same profile would
/// build a second WebView2 environment on a data directory whose previous owner
/// is still shutting down, and WebView2 blocks while that happens — on the main
/// thread, which is the freeze this exists to prevent.
///
/// The barrier is a task posted to the same queue, so once it has run, every
/// teardown queued before it has run too.
async fn settle_main_thread<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let (tx, rx) = std::sync::mpsc::channel();
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(());
        })
        .is_err()
    {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || rx.recv_timeout(MAIN_THREAD_SETTLE_TIMEOUT)).await;
}

/// The shared guard for a placement/URL cache, surviving a poisoned lock.
///
/// A poisoned mutex would otherwise turn a single panicking command into a
/// permanently broken browser panel, while the cache holds nothing that a
/// panic could leave inconsistent (it is a plain map of plain numbers).
fn locked<T: Default>(cell: &OnceLock<Mutex<T>>) -> MutexGuard<'_, T> {
    cell.get_or_init(|| Mutex::new(T::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Forget every cache entry for one label: it is no longer loaded, so its
/// rectangle and address must not be reused by a later incarnation.
fn forget_caches(label: &str) {
    locked(&PLACEMENTS).remove(label);
    locked(&EMBEDDED_URLS).remove(label);
}

/// Round a JS-supplied coordinate to a physical pixel.
///
/// `as` saturates and maps `NaN` to 0, so a nonsense value becomes the origin
/// instead of a panic or a wildly off-screen window.
fn round_coordinate(value: f64) -> i32 {
    value.round() as i32
}

/// Round a JS-supplied size to a physical pixel, never collapsing it to zero: a
/// zero-sized webview cannot be shown, and the frontend hides a tab by
/// visibility instead of by shrinking it.
fn round_dimension(value: f64) -> u32 {
    (value.round() as u32).max(1)
}

/// Register a creation in flight, reporting whether this label is already being
/// created (in which case the caller short-circuits as a duplicate request).
fn begin_creation(label: &str) -> bool {
    locked(&CREATION_STATE).creating.insert(label.to_string())
}

/// Mark a creation as finished, reporting whether the label was closed while it
/// was running. Also clears the pending flag: one cancellation is enough.
fn end_creation(label: &str) -> bool {
    let mut state = locked(&CREATION_STATE);
    state.creating.remove(label);
    state.close_pending.remove(label)
}

/// Record that a label waiting to be closed was still being created.
///
/// Returns whether the label was creating: when it was not, there is a live
/// webview to close and the caller proceeds as usual.
fn mark_close_pending_if_creating(label: &str) -> bool {
    let mut state = locked(&CREATION_STATE);
    if state.creating.contains(label) {
        state.close_pending.insert(label.to_string());
        true
    } else {
        false
    }
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
            let window = webview.window();
            // A standalone browser owns its window, so the window title follows
            // the page (with the address as a fallback, so the tab strip never
            // renders an empty row). An embedded browser shares the main window,
            // which has no per-browser title, so the address is the label.
            let (title, url) = if window.label() == label {
                // A standalone window is one page the user opened on purpose, so
                // its live address and title are worth the getters.
                let url = webview
                    .url()
                    .map(|url| url.to_string())
                    .unwrap_or_default();
                let title = window.title().unwrap_or_else(|_| url.clone());
                (title, url)
            } else {
                // Embedded: read the remembered address. `webview.url()` is a
                // blocking getter on the main thread and this list is polled, so
                // asking every open page would queue one round-trip per tab every
                // couple of seconds.
                let url = locked(&EMBEDDED_URLS)
                    .get(&label)
                    .cloned()
                    .unwrap_or_default();
                (url.clone(), url)
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
///
/// A label that is still being created has no webview to close yet: the close is
/// recorded instead, and the creation tears itself down when it finishes. The
/// caller still gets `Ok`, because from its point of view the page is closed.
fn close_browser<R: tauri::Runtime>(app: &tauri::AppHandle<R>, label: &str) -> Result<(), String> {
    if mark_close_pending_if_creating(label) {
        return Ok(());
    }
    let Some(webview) = app.get_webview(label) else {
        // Already gone: drop whatever the caches still remember about it, so a
        // later page reusing this label starts from a clean slate.
        forget_caches(label);
        return Ok(());
    };
    let window = webview.window();
    let result = if window.label() == label {
        window
            .close()
            .map_err(|error| format!("Failed to close the browser window: {error}"))
    } else {
        webview
            .close()
            .map_err(|error| format!("Failed to close the embedded browser: {error}"))
    };
    if result.is_ok() {
        forget_caches(label);
    }
    result
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
    // A standalone window is built on the main thread too (and building pumps
    // its message loop, exactly like a child webview), so it takes the same
    // turn: opening an account by hand while the panel is loading the rest must
    // not put two creations on that thread at once.
    let _guard = native_guard().await;
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
pub async fn mini_browser_current_url<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: Option<String>,
) -> Result<Option<String>, String> {
    // `webview.url()` is a blocking getter that has to be answered by the main
    // thread, so it is queued behind any creation instead of inside it.
    let _guard = native_guard().await;
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
/// Async so the close never runs inline on the main thread: creating a webview pumps that thread's message loop, and running a native teardown inside that pump is what hangs the window.
#[tauri::command]
pub async fn mini_browser_close<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    // Labels still being created have no webview to close yet, and a page being
    // created right now would otherwise survive this call and show up later as a
    // page the user already closed.
    let pending = {
        let state = locked(&CREATION_STATE);
        state.creating.iter().cloned().collect::<Vec<_>>()
    };
    if !pending.is_empty() {
        let mut state = locked(&CREATION_STATE);
        for label in pending {
            state.close_pending.insert(label);
        }
    }
    let labels = app
        // webviews(), not webview_windows(): the latter misses embedded child
        // webviews, which "close the browser" must close too.
        .webviews()
        .into_keys()
        .filter(|label| label.starts_with(MINI_BROWSER_LABEL))
        .collect::<Vec<_>>();
    // Taken *after* the labels were read on purpose: a creation that was already
    // in flight has to finish first (its label is one of these), but taking the
    // lock after the read means this call does not queue any teardown ahead of a
    // creation that is about to start for a label not in this list.
    let _guard = native_guard().await;
    for label in labels {
        close_browser(&app, &label)?;
    }
    // Held across the wait, not released before it: every queued teardown has
    // really run by the time the lock is free, so a re-open cannot meet a
    // half-dead WebView2 on the same data directory.
    settle_main_thread(&app).await;
    Ok(())
}

/// Close the window belonging to one account.
/// Async so the close never runs inline on the main thread: creating a webview pumps that thread's message loop, and running a native teardown inside that pump is what hangs the window.
#[tauri::command]
pub async fn mini_browser_close_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
) -> Result<(), String> {
    // Serialised against creation, and only reported as done once the main
    // thread has actually destroyed the page. The panel closes a tab and the
    // user can immediately press "open" again, and that second open has to wait
    // rather than build a WebView2 environment on a data directory whose
    // previous owner is still shutting down — WebView2 blocks while that
    // happens, and it blocks the main thread.
    let _guard = native_guard().await;
    let result = close_profile_window(&app, Some(&profile_id));
    settle_main_thread(&app).await;
    result
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

/// Bounds of an embedded browser in **physical** pixels, relative to the main
/// window's client area.
///
/// Physical is the only unit that survives the trip: the platform webview
/// multiplies whatever rectangle it is given by the child window's own scale
/// factor (`wry` `Webview::set_bounds` -> `set_bounds_inner`), so a logical
/// rectangle would arrive scaled again — on a 125% display a 1899px cell was
/// painted 1582px wide and offset.
fn embedded_bounds(placement: Placement) -> tauri::Rect {
    tauri::Rect {
        position: PhysicalPosition::new(placement.x, placement.y).into(),
        size: PhysicalSize::new(placement.width, placement.height).into(),
    }
}

/// Reposition the embedded browser for one account. Errors when that account
/// has no embedded browser (never opened, or closed).
///
/// `x` / `y` / `width` / `height` are physical pixels (see [`embedded_bounds`]).
/// The call is idempotent and async: an unchanged rectangle is dropped without
/// touching the main thread, and a changed one is queued there instead of being
/// executed inline by the IPC handler that received it.
#[tauri::command]
pub async fn mini_browser_set_bounds<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // Queued before the webview is looked up: the point is the order in which
    // work reaches the main thread, and a lookup is cheap either way.
    let _guard = native_guard().await;
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);
    let Some(webview) = embedded_webview(&app, &label) else {
        return Err(format!("No embedded browser is open for '{profile_id}'"));
    };

    let geometry = Placement {
        x: round_coordinate(x),
        y: round_coordinate(y),
        width: round_dimension(width),
        height: round_dimension(height),
        visible: false,
    };
    let placement = {
        let mut placements = locked(&PLACEMENTS);
        let current = placements.get(&label).copied();
        // Same rectangle as the one already asked for: the page is either there
        // or on its way there, so going to the platform again would only add
        // work to the queue that causes the hang.
        if geometry_already_applied(current.as_ref(), &geometry) {
            return Ok(());
        }
        // Visibility is `set_visible`'s business: keep what it last asked for
        // and only replace the geometry. With no entry the caller's rectangle is
        // recorded as shown, because a page this build is placing was just
        // created visible.
        let desired = match current {
            Some(current) => Placement {
                visible: current.visible,
                ..geometry
            },
            None => Placement {
                visible: true,
                ..geometry
            },
        };
        placements.insert(label.clone(), desired);
        desired
    };
    webview
        .set_bounds(embedded_bounds(placement))
        .map_err(|error| format!("Failed to resize the embedded browser: {error}"))
}

/// Show or hide the embedded browser for one account. Hiding keeps the webview
/// (and its login state) alive, which is what the tab strip needs when it
/// switches accounts. Errors when that account has no embedded browser.
///
/// Async and idempotent for the same reason as [`mini_browser_set_bounds`]: a
/// tab switch must not spend two main-thread round-trips on pages that are
/// already in the state being asked for.
#[tauri::command]
pub async fn mini_browser_set_visible<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
    visible: bool,
) -> Result<(), String> {
    let _guard = native_guard().await;
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);
    let Some(webview) = embedded_webview(&app, &label) else {
        return Err(format!("No embedded browser is open for '{profile_id}'"));
    };
    {
        let mut placements = locked(&PLACEMENTS);
        let current = placements.get(&label).copied();
        // Already in the state being asked for: nothing to do and, more to the
        // point, nothing to queue on the main thread.
        if visibility_already_applied(current.as_ref(), visible) {
            return Ok(());
        }
        // A page with no entry at all (created by an older build, or a cache
        // that was cleared) is *not* assumed to be hidden: it still gets shown
        // or hidden for real, because guessing "already hidden" would leave the
        // active tab invisible — the blank page this change exists to remove.
        let current = current.unwrap_or_default();
        placements.insert(
            label.clone(),
            Placement {
                visible,
                ..current
            },
        );
    }
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
    let geometry = Placement {
        x: round_coordinate(x),
        y: round_coordinate(y),
        width: round_dimension(width),
        height: round_dimension(height),
        visible: false,
    };

    /*
     * Serialised with every other native call, including for the two re-use
     * paths below: `navigate`, `set_bounds` and `show` all queue work on the
     * main thread, and if a *different* page is being created right now, that
     * work would be dispatched from inside the creation's message pump. That is
     * the freeze this lock exists to stop — it is about the queue, not about the
     * label being taken.
     */
    // Held from here to the end of the function, creation included: creating a
    // child webview pumps the main thread's message loop, which dispatches
    // whatever is already queued there, so the whole call has to be exclusive
    // against every other command that queues native work. The lock is *not*
    // re-taken further down — `tokio::sync::Mutex` is not reentrant, and taking
    // it twice on one task would deadlock the command.
    let _guard = CREATION_LOCK.lock().await;

    if let Some(existing) = embedded_webview(&app, &label) {
        existing
            .navigate(parsed)
            .map_err(|error| format!("Failed to navigate the embedded browser: {error}"))?;
        existing
            .set_bounds(embedded_bounds(geometry))
            .map_err(|error| format!("Failed to resize the embedded browser: {error}"))?;
        let _ = existing.show();
        // The open call places and shows the page itself, so both halves of the
        // expected placement are recorded here: the next flush then only sends
        // what it really wants to differ.
        locked(&EMBEDDED_URLS).insert(label.clone(), normalised);
        locked(&PLACEMENTS).insert(
            label.clone(),
            Placement {
                x: geometry.x,
                y: geometry.y,
                width: geometry.width,
                height: geometry.height,
                visible: true,
            },
        );
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

    // A second request for a page that is still being created is a no-op: the
    // in-flight creation already loads its address, and two overlapping
    // creations on one label would fail on the label being taken anyway.
    if !begin_creation(&label) {
        return Ok(());
    }

    let Some(main) = crate::main_window(&app) else {
        end_creation(&label);
        return Err("The main window is not available".to_string());
    };

    let builder = tauri::WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        // Same directory rule as the standalone window, so a profile keeps its
        // cookies when the presentation mode changes.
        .data_directory(profile_data_dir(&profile_id))
        // Reject non-web schemes here too: this also covers redirects and
        // `target=_blank`, so the page cannot walk itself into `file://`.
        .on_navigation(is_navigable_url);

    let created = main.add_child(
        builder,
        PhysicalPosition::new(geometry.x, geometry.y),
        PhysicalSize::new(geometry.width, geometry.height),
    );

    // The create may have been cancelled while it was running: a "close this
    // tab" arrives before the webview exists, so it has nothing to close. Tear
    // the new page down here instead, or it would be a visible page with no tab
    // controlling it.
    let cancelled = end_creation(&label);

    let webview = match created {
        Ok(webview) => webview,
        Err(error) => {
            return Err(format!("Failed to open the embedded browser: {error}"));
        }
    };

    if cancelled {
        let _ = webview.close();
        // The label never became a page this build placed, so any leftovers from
        // an earlier incarnation of it must not survive into the next one.
        forget_caches(&label);
        return Ok(());
    }

    // The child webview reports itself visible as soon as it is created (that
    // is why a page loading behind the active tab is created at the parked
    // rectangle), so the expected placement starts shown.
    locked(&EMBEDDED_URLS).insert(label.clone(), normalised);
    locked(&PLACEMENTS).insert(
        label,
        Placement {
            visible: true,
            ..geometry
        },
    );
    Ok(())
}

/// Show and focus the window belonging to one account.
///
/// Async for the same reason as the placement commands: `show`, `unminimize` and
/// `set_focus` are main-thread calls, and this one can be clicked while another
/// page is still being created.
#[tauri::command]
pub async fn mini_browser_focus_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
) -> Result<(), String> {
    let _guard = native_guard().await;
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
///
/// Async, and serialised with creation, because a standalone window's address
/// and title are blocking getters answered by the main thread. This command is
/// polled every couple of seconds, so a poll that landed inside a webview
/// creation's message pump would run those getters against a half-built page.
#[tauri::command]
pub async fn mini_browser_list_windows<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Vec<MiniBrowserWindowInfo> {
    let _guard = native_guard().await;
    open_window_infos(&app)
}

/// Where the backend really has one embedded page, for the panel's check loop.
///
/// The three readings are deliberately reported together: the rectangle the
/// platform reports, the scale factor the window is running at, and the client
/// area in physical pixels. A caller can then tell "the page is one window
/// behind" apart from "the page was placed in the wrong unit" without guessing —
/// which is exactly the confusion that made the 125% display look like a fixed
/// 0.833x zoom.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniBrowserBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub client_width: u32,
    pub client_height: u32,
    pub visible: bool,
}

/// Read the live rectangle of one embedded page.
#[tauri::command]
pub async fn mini_browser_bounds<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    profile_id: String,
) -> Result<MiniBrowserBounds, String> {
    // A read-only probe that still talks to the main thread twice (rectangle and
    // window size), and it is polled: it waits for a creation rather than being
    // answered from inside one.
    let _guard = native_guard().await;
    let profile = checked_profile(Some(&profile_id))?;
    let label = window_label(profile);
    let Some(webview) = embedded_webview(&app, &label) else {
        return Err(format!("No embedded browser is open for '{profile_id}'"));
    };

    // The rectangle the runtime hands back is already in physical pixels, so it
    // is read as-is and can be compared with the wanted rectangle directly. The
    // conversion is spelled out here rather than via `to_physical`, which
    // asserts a sane scale factor — a panic in a release build aborts the whole
    // app, and a scale reading is not worth that.
    let window = webview.window();
    let scale_factor = window
        .scale_factor()
        .map_err(|error| format!("Failed to read the window scale: {error}"))?;
    let bounds = webview
        .bounds()
        .map_err(|error| format!("Failed to read the embedded browser bounds: {error}"))?;
    let (x, y) = match bounds.position {
        tauri::Position::Physical(position) => (position.x, position.y),
        tauri::Position::Logical(position) => (
            (position.x * scale_factor).round() as i32,
            (position.y * scale_factor).round() as i32,
        ),
    };
    let (width, height) = match bounds.size {
        tauri::Size::Physical(size) => (size.width, size.height),
        tauri::Size::Logical(size) => (
            (size.width * scale_factor).round() as u32,
            (size.height * scale_factor).round() as u32,
        ),
    };
    let client = window
        .inner_size()
        .map_err(|error| format!("Failed to read the main window size: {error}"))?;

    Ok(MiniBrowserBounds {
        x,
        y,
        width,
        height,
        scale_factor,
        client_width: client.width,
        client_height: client.height,
        visible: locked(&PLACEMENTS)
            .get(&label)
            .map(|placement| placement.visible)
            .unwrap_or(false),
    })
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
    {
        let _guard = native_guard().await;
        close_profile_window(&app, profile)?;
    }
    // The directory is deleted below, and Windows keeps WebView2's files locked
    // until the page is really gone — so wait for the teardown instead of
    // spending the whole retry budget waiting for it.
    settle_main_thread(&app).await;
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

    /// The placement every dedupe test starts from.
    fn placement() -> Placement {
        Placement {
            x: 12,
            y: 34,
            width: 1000,
            height: 750,
            visible: true,
        }
    }

    #[test]
    fn placement_dedupe_same_geometry_is_noop() {
        // The whole point of the cache: a repeated request must be answered
        // without a `SetWindowPos` on the main thread, at any visibility.
        let current = placement();
        assert!(current.same_geometry(&placement()));
        assert!(current.same_geometry(&Placement {
            visible: false,
            ..placement()
        }));
    }

    #[test]
    fn placement_dedupe_any_geometry_field_changes_needs_send() {
        let current = placement();
        for moved in [
            Placement { x: 13, ..placement() },
            Placement { y: 35, ..placement() },
            Placement { width: 1001, ..placement() },
            Placement { height: 751, ..placement() },
        ] {
            assert!(
                !current.same_geometry(&moved),
                "{moved:?} must be a different rectangle"
            );
        }
    }

    #[test]
    fn placement_dedupe_visibility_alone_is_not_a_geometry_change() {
        // `set_bounds` must not turn a show/hide into a resize (and vice versa):
        // the two commands own two different halves of the placement.
        let shown = placement();
        let hidden = Placement {
            visible: false,
            ..placement()
        };
        assert!(shown.same_geometry(&hidden));
        assert_ne!(shown, hidden);
    }

    #[test]
    fn placement_dedupe_missing_entry_is_never_a_noop() {
        // A page the cache has forgotten still has to be placed and shown: a
        // dropped `set_visible(false)` would leave a page painted over the UI,
        // and a dropped `set_visible(true)` would leave the active tab blank.
        let known = placement();
        assert!(geometry_already_applied(Some(&known), &known));
        assert!(visibility_already_applied(Some(&known), true));
        assert!(!geometry_already_applied(None, &known));
        assert!(!visibility_already_applied(None, true));
        assert!(!visibility_already_applied(None, false));
    }

    #[test]
    fn physical_pixels_round_and_sizes_never_collapse() {
        // A 125% display reports fractional physical pixels; half a pixel is
        // rounded, never truncated, or the page drifts a pixel per resize.
        assert_eq!(round_coordinate(1899.6), 1900);
        // `f64::round` rounds half away from zero, in both directions.
        assert_eq!(round_coordinate(-4.5), -5);
        assert_eq!(round_coordinate(4.5), 5);
        assert_eq!(round_dimension(753.5), 754);
        // A rectangle the platform cannot show must not become 0-wide: a
        // zero-sized webview cannot be revealed at all.
        assert_eq!(round_dimension(0.0), 1);
        assert_eq!(round_dimension(-3.0), 1);
        assert_eq!(round_dimension(f64::NAN), 1);
        assert_eq!(round_coordinate(f64::NAN), 0);
        // Nonsense input is clamped to the integer range instead of wrapping.
        assert_eq!(round_coordinate(f64::INFINITY), i32::MAX);
        assert_eq!(round_dimension(f64::INFINITY), u32::MAX);
        assert_eq!(round_coordinate(f64::NEG_INFINITY), i32::MIN);
    }

    #[test]
    fn creation_state_reports_and_consumes_a_pending_close() {
        // The close/creation handshake: a close that lands during creation is
        // remembered exactly once, so the finished page tears itself down.
        assert!(begin_creation("mini-browser-handshake"));
        // A second request for the same label is a duplicate, not a second
        // creation.
        assert!(!begin_creation("mini-browser-handshake"));
        assert!(mark_close_pending_if_creating("mini-browser-handshake"));
        assert!(end_creation("mini-browser-handshake"));
        // Consumed: a later page reusing the label must not be closed by it.
        assert!(!end_creation("mini-browser-handshake"));
        // And a close for a page that is not being created is a normal close.
        assert!(!mark_close_pending_if_creating("mini-browser-handshake"));
    }
}
