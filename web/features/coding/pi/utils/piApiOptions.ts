/**
 * The `api` values Pi's `models.json` accepts for a provider.
 *
 * Kept in one module because the provider modal, the imported-from-favorite
 * path and the model editor all offer the same list; three copies is how one
 * of them starts accepting a protocol the others do not.
 */
export const PI_API_OPTIONS = [
  'openai-completions',
  'openai-responses',
  'anthropic-messages',
  'google-generative-ai',
].map((value) => ({ value, label: value }));
