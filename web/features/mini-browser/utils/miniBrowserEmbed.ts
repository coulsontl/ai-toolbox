import type { MiniBrowserBounds } from '@/services/miniBrowserApi';

/**
 * Round a DOM rectangle into the logical-pixel bounds the backend expects.
 *
 * `getBoundingClientRect()` is the source of truth so the placeholder cell and
 * the native webview can never drift apart. Positions are floored and the size
 * is derived from the rounded corners, which keeps the rectangle seamless
 * (a rounded size plus a rounded origin can otherwise leave a one-pixel seam).
 * Non-finite or empty rectangles return `null` so callers skip the invoke
 * instead of sending a rectangle the backend would reject.
 *
 * The value is in logical pixels: `getBoundingClientRect()` is already in CSS
 * pixels, and the backend's `set_bounds` converts logical units to device pixels
 * with the window's scale factor, so no scaling may happen here as well.
 */
export const toMiniBrowserBounds = (
  rect: { left: number; top: number; width: number; height: number },
): MiniBrowserBounds | null => {
  if (![rect.left, rect.top, rect.width, rect.height].every(Number.isFinite)) return null;

  const x = Math.round(rect.left);
  const y = Math.round(rect.top);
  const width = Math.round(rect.left + rect.width) - x;
  const height = Math.round(rect.top + rect.height) - y;
  if (width <= 0 || height <= 0) return null;

  return { x, y, width, height };
};

/**
 * Whether two bounds describe the same rectangle.
 *
 * Used to skip a `set_bounds` round-trip when a resize/scroll event did not
 * actually move the cell (the common case: the panel scrolls, the grid does
 * not).
 */
export const areMiniBrowserBoundsEqual = (
  a: MiniBrowserBounds | null,
  b: MiniBrowserBounds | null,
): boolean => {
  if (!a || !b) return a === b;
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
};
