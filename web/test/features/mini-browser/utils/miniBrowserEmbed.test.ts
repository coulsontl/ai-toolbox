import assert from 'node:assert/strict';
import test from 'node:test';

import {
  areMiniBrowserBoundsEqual,
  planPlacementFlush,
  toMiniBrowserBounds,
  type MiniBrowserAppliedPlacement,
} from '../../../../features/mini-browser/utils/miniBrowserEmbed';

const CELL = { left: 320, top: 96, width: 800, height: 600 };

// ---- DOM rect -> physical bounds ------------------------------------------

test('mini browser scales a DOM rectangle to physical pixels', () => {
  // 125% Windows scaling: the cell is 800 CSS px wide but 1000 device px, and
  // sending the CSS number is what made the page 0.833x too narrow.
  assert.deepEqual(toMiniBrowserBounds(CELL, 1.25), {
    x: 400,
    y: 120,
    width: 1000,
    height: 750,
  });
});

test('mini browser passes a rectangle through at 100% scale', () => {
  assert.deepEqual(toMiniBrowserBounds(CELL, 1), { x: 320, y: 96, width: 800, height: 600 });
});

test('mini browser keeps a scaled rectangle seamless', () => {
  // Rounding the size and the origin independently can leave a one-pixel seam
  // between neighbouring cells; the size is derived from the rounded corners.
  assert.deepEqual(
    toMiniBrowserBounds({ left: 10.4, top: 20.6, width: 100.4, height: 50.5 }, 1.5),
    { x: 16, y: 31, width: 150, height: 76 },
  );
});

test('mini browser refuses a scale factor it cannot use', () => {
  for (const devicePixelRatio of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(toMiniBrowserBounds(CELL, devicePixelRatio), null);
  }
});

test('mini browser refuses rectangles the backend could not use', () => {
  // An empty or zero-sized cell means "not laid out yet" (hidden route or a
  // frame before the grid has a size), never a rectangle to send.
  assert.equal(toMiniBrowserBounds({ left: 0, top: 0, width: 0, height: 0 }, 1), null);
  assert.equal(toMiniBrowserBounds({ left: 0, top: 0, width: 0, height: 400 }, 1), null);
  assert.equal(toMiniBrowserBounds({ left: 0, top: 0, width: 400, height: 0 }, 1), null);
  assert.equal(
    toMiniBrowserBounds({ left: Number.NaN, top: 0, width: 400, height: 400 }, 1),
    null,
  );
  assert.equal(
    toMiniBrowserBounds({ left: 0, top: 0, width: Number.POSITIVE_INFINITY, height: 400 }, 1),
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

// ---- flush planning --------------------------------------------------------

const PLACED = { x: 400, y: 120, width: 1000, height: 750 };
const PARKED = { x: 0, y: 0, width: 1, height: 1 };

test('mini browser flush leaves a placement that already matches alone', () => {
  assert.deepEqual(
    planPlacementFlush(
      new Map([['alpha', PLACED]]),
      new Map([['alpha', true]]),
      new Map<string, MiniBrowserAppliedPlacement>([
        ['alpha', { bounds: PLACED, visible: true }],
      ]),
    ),
    [],
  );
});

test('mini browser flush orders a tab switch as hide, park, place, reveal', () => {
  const ops = planPlacementFlush(
    new Map([
      ['alpha', PARKED],
      ['beta', PLACED],
    ]),
    new Map([
      ['alpha', false],
      ['beta', true],
    ]),
    new Map<string, MiniBrowserAppliedPlacement>([
      ['alpha', { bounds: PLACED, visible: true }],
      ['beta', { bounds: PARKED, visible: false }],
    ]),
  );

  // The page that leaves is hidden *before* it is parked, and the page that
  // arrives is placed *before* it is revealed: either the other way round is
  // the stale rectangle the user sees as a flash.
  assert.deepEqual(ops, [
    { kind: 'visible', profileId: 'alpha', visible: false },
    { kind: 'bounds', profileId: 'alpha', bounds: PARKED },
    { kind: 'bounds', profileId: 'beta', bounds: PLACED },
    { kind: 'visible', profileId: 'beta', visible: true },
  ]);
});

test('mini browser flush ignores tabs the backend has not confirmed', () => {
  // A tab that has not been opened yet (or was closed while a flush was
  // queued) has no confirmed placement, and only the open call may create a
  // page: pushing a rectangle at it would be an error in the log, not a repair.
  assert.deepEqual(
    planPlacementFlush(
      new Map([['alpha', PLACED]]),
      new Map([['alpha', true]]),
      new Map<string, MiniBrowserAppliedPlacement>(),
    ),
    [],
  );
});

test('mini browser flush only re-sends the half that diverged', () => {
  const ops = planPlacementFlush(
    new Map([['alpha', PARKED]]),
    new Map([['alpha', true]]),
    new Map<string, MiniBrowserAppliedPlacement>([
      ['alpha', { bounds: PLACED, visible: true }],
    ]),
  );
  assert.deepEqual(ops, [{ kind: 'bounds', profileId: 'alpha', bounds: PARKED }]);
});
