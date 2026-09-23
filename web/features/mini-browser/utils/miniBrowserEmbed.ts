import type { MiniBrowserBounds } from '@/services/miniBrowserApi';

/**
 * Turn a DOM rectangle into the physical-pixel bounds the backend expects.
 *
 * `getBoundingClientRect()` is the source of truth so the placeholder cell and
 * the native page can never drift apart, but its numbers are CSS pixels while
 * the backend places the page with physical ones. The child webview applies its
 * own scale factor to whatever rectangle it is handed, so sending CSS pixels
 * shrinks the page by that factor instead of landing on the cell: on a 120 DPI
 * (125%) Windows display a 1899px-wide cell came out 1582px wide and shifted.
 *
 * The origin is rounded first and the size is derived from the rounded corners,
 * which keeps the rectangle seamless (a rounded size plus a rounded origin can
 * otherwise leave a one-pixel seam).
 *
 * Non-finite values, a scale factor that is not a positive number and empty
 * rectangles all return `null`, so callers skip the invoke instead of sending a
 * rectangle the backend would reject.
 */
export const toMiniBrowserBounds = (
  rect: { left: number; top: number; width: number; height: number },
  devicePixelRatio: number,
): MiniBrowserBounds | null => {
  if (!Number.isFinite(devicePixelRatio) || devicePixelRatio <= 0) return null;
  if (![rect.left, rect.top, rect.width, rect.height].every(Number.isFinite)) return null;

  const x = Math.round(rect.left * devicePixelRatio);
  const y = Math.round(rect.top * devicePixelRatio);
  const width = Math.round((rect.left + rect.width) * devicePixelRatio) - x;
  const height = Math.round((rect.top + rect.height) * devicePixelRatio) - y;
  if (width <= 0 || height <= 0) return null;

  return { x, y, width, height };
};

/**
 * Whether two bounds describe the same rectangle.
 *
 * Used to skip a `set_bounds` round-trip when a resize or scroll event did not
 * actually move the cell (the common case: the panel scrolls, the grid does
 * not), and to decide whether the backend already agrees with the page.
 */
export const areMiniBrowserBoundsEqual = (
  a: MiniBrowserBounds | null,
  b: MiniBrowserBounds | null,
): boolean => {
  if (!a || !b) return a === b;
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
};

/** One profile's placement as the backend has confirmed it. */
export interface MiniBrowserAppliedPlacement {
  bounds: MiniBrowserBounds;
  visible: boolean;
}

/**
 * One placement request, already ordered against the rest of the flush.
 *
 * Rectangle and visibility are separate operations because their order matters:
 * a page that is leaving has to be hidden *before* it is parked, and a page that
 * is arriving has to be placed *before* it is revealed. One combined call would
 * leave that order to the backend.
 */
export type MiniBrowserPlacementOp =
  | { kind: 'bounds'; profileId: string; bounds: MiniBrowserBounds }
  | { kind: 'visible'; profileId: string; visible: boolean };

/**
 * Work out which placement requests a flush still owes the backend.
 *
 * Wanted and confirmed placements are compared per profile, so a page that
 * already has the rectangle and visibility it asked for costs nothing: a tab
 * switch asks for the two tabs it touched, not for every open tab. A profile
 * with no confirmed placement is skipped — the open call is the only thing that
 * can create a page, and it records the placement it used, so anything else is
 * still opening or already gone.
 *
 * Leaving pages are grouped before arriving ones, so a single flush can never
 * have two pages on screen at once, and an arriving page is placed before it is
 * revealed: revealing it at the parked (or previous) rectangle is the flash this
 * ordering exists to prevent.
 */
export const planPlacementFlush = (
  desiredBounds: ReadonlyMap<string, MiniBrowserBounds>,
  desiredVisible: ReadonlyMap<string, boolean>,
  applied: ReadonlyMap<string, MiniBrowserAppliedPlacement>,
): MiniBrowserPlacementOp[] => {
  const leaving: MiniBrowserPlacementOp[] = [];
  const arriving: MiniBrowserPlacementOp[] = [];

  for (const [profileId, bounds] of desiredBounds) {
    const confirmed = applied.get(profileId);
    if (!confirmed) continue;

    const visible = desiredVisible.get(profileId) === true;
    const moved = !areMiniBrowserBoundsEqual(confirmed.bounds, bounds);
    const toggled = confirmed.visible !== visible;

    if (visible) {
      if (moved) arriving.push({ kind: 'bounds', profileId, bounds });
      if (toggled) arriving.push({ kind: 'visible', profileId, visible: true });
    } else {
      if (toggled) leaving.push({ kind: 'visible', profileId, visible: false });
      if (moved) leaving.push({ kind: 'bounds', profileId, bounds });
    }
  }

  return [...leaving, ...arriving];
};
