// Runtime locations are process-wide caches. Keep cross-tool import fixtures
// isolated from unrelated CLI tests which intentionally change those caches.
#[path = "coding/deeplink/provider_transfer.rs"]
mod provider_transfer;
