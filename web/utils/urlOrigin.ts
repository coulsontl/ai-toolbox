/**
 * Extract the origin (scheme + host + port) from a base URL.
 * Path and query are stripped. Invalid or unsupported values return null.
 */
export function getUrlOrigin(baseUrl?: string | null): string | null {
  const raw = baseUrl?.trim();
  if (!raw) {
    return null;
  }

  const hasScheme = /^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(raw);
  const candidates = hasScheme ? [raw] : [raw, `https://${raw}`];

  for (const candidate of candidates) {
    try {
      const parsed = new URL(candidate);
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        continue;
      }
      return parsed.origin;
    } catch {
      // try next candidate
    }
  }

  return null;
}
