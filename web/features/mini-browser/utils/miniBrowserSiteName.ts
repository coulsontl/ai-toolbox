/**
 * Derive a default site name from a saved address.
 *
 * Saving a site used to demand a hand-typed name even though the host is
 * already implied by the URL. The extracted value is a convenience default
 * only: the form keeps the field editable, and a manually typed name always
 * wins (see `MiniBrowserPage.handleSaveSite`).
 *
 * The host (including its port, when given) is preferred over `URL.origin` so a
 * name stays readable and still distinguishes two dashboards on one machine
 * (`relay.example.com` / `127.0.0.1:3000`). A bare host is accepted too, so the
 * helper does not depend on the caller having normalised the address first. A
 * missing or non-web host falls back to `null` instead of inventing a label.
 */
export const deriveMiniBrowserSiteName = (rawUrl: string): string | null => {
  const trimmed = rawUrl.trim();
  if (!trimmed) return null;
  if (/[\u0000-\u001f\u007f]/.test(trimmed)) return null;

  const hasScheme = /^[a-zA-Z][a-zA-Z\d+\-.]*:\/\//.test(trimmed);
  try {
    const parsed = new URL(hasScheme ? trimmed : `https://${trimmed}`);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    if (!parsed.hostname) return null;
    return parsed.host;
  } catch {
    return null;
  }
};
