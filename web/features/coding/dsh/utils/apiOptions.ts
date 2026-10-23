/**
 * The `api` values dsh's `llm-pi-ai` route schema accepts.
 *
 * Kept in one module because the provider modal and the page both offer the
 * list; two copies is how one of them starts accepting a protocol the engine
 * rejects. dsh's `ApiSchema` only ships these three.
 */
export const DSH_API_OPTIONS = [
  'openai-completions',
  'openai-responses',
  'anthropic-messages',
].map((value) => ({ value, label: value }));
