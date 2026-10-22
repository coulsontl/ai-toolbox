import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the **real** provider-list drag interaction in a browser: pointer down
 * on the grip, move past dnd-kit's activation distance, move onto another card,
 * release. Then asserts what the user can observe — the rendered order changed,
 * and the page wrote that order back to the backend.
 *
 * This exists because reading the drag chain ("the handle renders, `useSortable`
 * registers, the page has a `DndContext`") proves nothing about whether a drag
 * actually moves a card. The failure this repo has hit twice is a handle that
 * looks and behaves like a handle while the drag is disabled underneath it, and
 * every static check passes in that state.
 */
export async function verifyProviderListDrag({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 200; attempt++) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };
  const fixture = expression => evaluate('providerListDragFixture.' + expression);
  /** The fixture renders in zh-CN; labels are read from the page, not hardcoded. */
  const sortLabel = '排序';
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  const openFixture = async query => {
    const runId = `${query}-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${runId}` });
    await waitFor(
      `window.providerListDragFixture?.state.runId === ${JSON.stringify(runId)}`
        + ' && providerListDragFixture.renderedOrder().length === 3',
      `fixture ${query}`,
    );
    await delay(300);
  };

  /** A real pointer drag: dnd-kit's PointerSensor only activates past its
   *  `distance` constraint, so the move must overshoot it. */
  const dragCard = async (fromName, toName) => {
    const from = await fixture(`handleCenter(${JSON.stringify(fromName)})`);
    const to = await fixture(`cardCenter(${JSON.stringify(toName)})`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: from.x, y: from.y, button: 'left', clickCount: 1 });
    // Intermediate moves: a single jump can be swallowed as a teleport and the
    // sortable never sees an over-target.
    const steps = 6;
    for (let step = 1; step <= steps; step++) {
      await send('Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: from.x + ((to.x - from.x) * step) / steps,
        y: from.y + ((to.y - from.y) * step) / steps,
        button: 'left',
      });
      await delay(60);
    }
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: to.x, y: to.y, button: 'left', clickCount: 1 });
    await delay(400);
  };

  // --- custom mode: the handle exists and a drag reorders ------------------
  await openFixture('sortMode=custom');
  check('the grip renders on every card in custom order', await fixture('renderedOrder().length === 3 && ["Provider A","Provider B","Provider C"].every(name => providerListDragFixture.hasHandle(name))'));
  await screenshot('custom-before');
  await dragCard('Provider A', 'Provider C');
  const afterDrag = await fixture('renderedOrder()');
  check('dragging the first card onto the last moves it there', afterDrag, ['Provider B', 'Provider C', 'Provider A']);
  await screenshot('custom-after');
  check('the new order is written back to the backend', await fixture('lastReorder()'), ['provider-b', 'provider-c', 'provider-a']);
  check('the backend state matches what the user sees', await fixture('storedOrder()'), ['Provider B', 'Provider C', 'Provider A']);

  // --- non-custom mode: the handle is gone, and the sort control says why ---
  await openFixture('sortMode=created');
  check(
    'no grip renders in a non-custom sort mode, so the list cannot look draggable',
    await fixture('renderedOrder().length === 3 && ["Provider A","Provider B","Provider C"].every(name => !providerListDragFixture.hasHandle(name))'),
  );
  // Newest first: the fixture's `createdAt` ascends with the stored order, so
  // this order is the reverse of it. The previous assertion expected A, B, C —
  // the stored order — and therefore passed whether or not the sort ran.
  check('non-custom mode renders in its own order, not the stored one', await fixture('renderedOrder()'), ['Provider C', 'Provider B', 'Provider A']);
  const sortButtonCenter = await evaluate(`(() => {
    const button = [...document.querySelectorAll('button')]
      .find(element => element.textContent.includes(${JSON.stringify(sortLabel)}));
    if (!button) return null;
    const rect = button.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  })()`);
  assert.ok(sortButtonCenter, 'the sort control must be on screen');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: sortButtonCenter.x, y: sortButtonCenter.y });
  await delay(600);
  check(
    'hovering the sort control explains why dragging is unavailable',
    await evaluate(`document.querySelector('.ant-tooltip')?.textContent ?? null`),
    '拖拽排序仅在选择「默认」排序时可用',
  );
  await screenshot('created-mode');

  // --- the control actually sorts: open the menu, pick, observe -----------
  //
  // Everything above asserts the *consequence* of a sort mode loaded from the
  // backend. None of it drives the control, so a dropdown whose click never
  // reaches the page — the reported "排序不生效" — would pass all of them.
  const clickSortMode = async label => {
    const center = await evaluate(`(() => {
      const button = [...document.querySelectorAll('button')]
        .find(element => element.textContent.includes(${JSON.stringify(sortLabel)}));
      if (!button) return null;
      const rect = button.getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`);
    assert.ok(center, 'the sort control must be on screen');
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: center.x, y: center.y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: center.x, y: center.y, button: 'left', clickCount: 1 });
    await delay(300);
    const items = await fixture('sortMenuItems()');
    assert.ok(items.length > 0, 'the sort menu must open: ' + JSON.stringify(items));
    const itemCenter = await evaluate(`(() => {
      const item = [...document.querySelectorAll('.ant-dropdown-menu-item')]
        .find(element => element.textContent.trim() === ${JSON.stringify(label)});
      if (!item) return null;
      const rect = item.getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`);
    assert.ok(itemCenter, `the "${label}" sort item must be in the menu`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: itemCenter.x, y: itemCenter.y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: itemCenter.x, y: itemCenter.y, button: 'left', clickCount: 1 });
    await delay(400);
  };

  await openFixture('sortMode=custom');
  await clickSortMode('按名称');
  check(
    'picking a sort mode from the menu reorders the list',
    await fixture('renderedOrder()'),
    ['Provider A', 'Provider B', 'Provider C'],
  );
  check(
    'picking a sort mode persists the choice',
    await fixture('lastSavedSortMode()'),
    { module: 'codex', mode: 'name' },
  );
  // The dropdown lives inside the Collapse header: a click that bubbles would
  // also toggle the whole section, which looks like "the menu did nothing".
  // Read `aria-expanded`, not a content class: antd 6 keeps the active state on
  // the collapse header, and there is no `.ant-collapse-content-active` to find.
  check(
    'picking a sort mode leaves the provider section expanded',
    await evaluate(`document.querySelector('#codex-providers .ant-collapse-header')?.getAttribute('aria-expanded')`),
    'true',
  );

  return checks;
}
