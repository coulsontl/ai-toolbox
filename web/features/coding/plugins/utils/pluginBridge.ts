/**
 * Small shared helpers for the host side of the Magpie Bridge v1. Both the
 * full-page frame and the inline panel frame answer the same page requests, so
 * payload validation and image decoding live here instead of twice.
 */

export type PluginBridgeResponse =
  | { ok: true; result?: unknown }
  | { ok: false; error: string };

export const MAX_PLUGIN_IMAGE_BYTES = 20 * 1024 * 1024;

export const isPluginBridgeRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

export const requirePluginBridgeString = (value: unknown): string | null =>
  typeof value === 'string' && value.trim() ? value : null;

/**
 * Accept the documented base64/data-URL string, and a `Blob` as a defensive
 * second shape because structured cloning lets pages push binary data through
 * the bridge without an encode step.
 */
export const readPluginBridgeImagePayload = (value: unknown): Promise<string | null> => {
  if (typeof value === 'string') {
    return Promise.resolve(value || null);
  }
  if (!(value instanceof Blob)) {
    return Promise.resolve(null);
  }
  if (value.size > MAX_PLUGIN_IMAGE_BYTES) {
    return Promise.reject(new Error('image payload exceeds 20 MiB'));
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : '';
      resolve(result || null);
    };
    reader.onerror = () => reject(reader.error ?? new Error('failed to read image blob'));
    reader.readAsDataURL(value);
  });
};
