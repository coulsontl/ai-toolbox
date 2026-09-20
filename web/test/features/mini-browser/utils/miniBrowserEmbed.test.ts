import assert from 'node:assert/strict';
import test from 'node:test';

import {
  areMiniBrowserBoundsEqual,
  miniBrowserGridColumns,
  toMiniBrowserBounds,
} from '../../../../features/mini-browser/utils/miniBrowserEmbed';

// ---- grid shape ------------------------------------------------------------

test('mini browser embeds one account full width and keeps larger counts readable', () => {
  // A single account must not waste half of the area on an empty column.
  assert.equal(miniBrowserGridColumns(0), 1);
  assert.equal(miniBrowserGridColumns(1), 1);
  // Two to four accounts tile into a square-ish 2-column grid.
  assert.equal(miniBrowserGridColumns(2), 2);
  assert.equal(miniBrowserGridColumns(3), 2);
  assert.equal(miniBrowserGridColumns(4), 2);
  // From five on, three columns keep the cells wider than they are tall.
  assert.equal(miniBrowserGridColumns(5), 3);
  assert.equal(miniBrowserGridColumns(9), 3);
});

// ---- DOM rect -> logical bounds -------------------------------------------

test('mini browser passes a DOM rectangle through as logical bounds', () => {
  assert.deepEqual(
    toMiniBrowserBounds({ left: 320, top: 96, width: 800, height: 600 }),
    { x: 320, y: 96, width: 800, height: 600 },
  );
});

test('mini browser keeps a rounded rectangle seamless', () => {
  // Rounding the size and the origin independently can leave a one-pixel seam
  // between neighbouring cells; the size is derived from the rounded corners.
  assert.deepEqual(
    toMiniBrowserBounds({ left: 10.4, top: 20.6, width: 100.4, height: 50.5 }),
    { x: 10, y: 21, width: 101, height: 50 },
  );
});

test('mini browser refuses rectangles the backend could not use', () => {
  // An empty or zero-sized cell means "not laid out yet" (hidden route or a
  // frame before the grid has a size), never a rectangle to send.
  assert.equal(toMiniBrowserBounds({ left: 0, top: 0, width: 0, height: 0 }), null);
  assert.equal(toMiniBrowserBounds({ left: 0, top: 0, width: 0, height: 400 }), null);
  assert.equal(toMiniBrowserBounds({ left: 0, top: 0, width: 400, height: 0 }), null);
  assert.equal(
    toMiniBrowserBounds({ left: Number.NaN, top: 0, width: 400, height: 400 }),
    null,
  );
  assert.equal(
    toMiniBrowserBounds({ left: 0, top: 0, width: Number.POSITIVE_INFINITY, height: 400 }),
    null,
  );
});

// ---- change detection ------------------------------------------------------

test('mini browser only re-sends bounds whose rectangle changed', () => {
  const bounds = { x: 10, y: 20, width: 300, height: 400 };
  assert.equal(areMiniBrowserBoundsEqual(bounds, { ...bounds }), true);
  assert.equal(areMiniBrowserBoundsEqual(bounds, { ...bounds, x: 11 }), false);
  assert.equal(areMiniBrowserBoundsEqual(bounds, { ...bounds, height: 401 }), false);
  // "Not measured yet" must not compare equal to a real rectangle, and two
  // unmeasured cells stay equal so a scroll storm sends nothing.
  assert.equal(areMiniBrowserBoundsEqual(null, bounds), false);
  assert.equal(areMiniBrowserBoundsEqual(bounds, null), false);
  assert.equal(areMiniBrowserBoundsEqual(null, null), true);
});
