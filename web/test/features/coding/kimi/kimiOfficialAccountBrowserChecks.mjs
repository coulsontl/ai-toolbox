import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the official-account card of the **real** Kimi page in a browser.
 *
 * What this guards, and why the real page rather than the component:
 *
 *  - **Placement.** The card is a member of the provider list, drawn at a slot
 *    derived from the merged ordering. A card-only fixture cannot see the slot
 *    arithmetic, and the defect this replaced was purely a placement mistake —
 *    the account area sat in the list footer while the user read it as a card.
 *  - **The merged drag index.** The drag indices come from `sortableItemIds`,
 *    not from `providers`. Using the provider list would silently write an
 *    index off by one — the card occupies a slot too — and nothing static
 *    would notice.
 *  - **The empty-list fallback.** The card is passed as both `children` and
 *    `alwaysVisible`; the two branches are exclusive. If only the first is
 *    wired, the sign-in entry disappears the moment the list is empty, and
 *    looking at a populated list would never show it.
 *
 * Labels are read off the rendered page rather than hardcoded, so re-wording
 * the card does not require editing this file.
 */
export async function verifyKimiOfficialAccountCard({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  // 50s, not 10s: the fixture rebuilds and boots the whole page, and in a full
  // `pnpm test` run the machine is still busy shutting down the previous
  // browser. A short budget turns that into a phantom failure.
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      if (await evaluate(expression)) return;
      await delay(125);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };
  const fixture = expression => evaluate('kimiOfficialAccountFixture.' + expression);
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  /** A real pointer drag: dnd-kit's PointerSensor only activates past its
   *  `distance` constraint, so the move must overshoot it. */
  const dragCard = async (fromName, toName) => {
    const from = await fixture(`handleCenter(${JSON.stringify(fromName)})`);
    const to = await fixture(`cardCenter(${JSON.stringify(toName)})`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: from.x, y: from.y, button: 'left', clickCount: 1 });
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

  const openFixture = async query => {
    // No `&` or `=` in the run id: it is appended as a query value, and an
    // unencoded separator would silently split the rest of the query off.
    const runId = `run-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${runId}` });
    await waitFor(
      `window.kimiOfficialAccountFixture?.state.runId === ${JSON.stringify(runId)}`,
      `fixture ${query}`,
    );
    await delay(300);
  };

  // --- populated list: the card is a member, at its stored slot -------------
  await openFixture('providers=3&accounts=2&accountIndex=0');
  await waitFor('kimiOfficialAccountFixture.renderedMembers().length === 4', 'four list members');
  const title = await fixture('OFFICIAL_ACCOUNT_TITLE');

  check(
    'the official-account card is drawn as the first member of the provider list',
    await fixture('renderedMembers()'),
    [title, 'Provider A', 'Provider B', 'Provider C'],
  );
  check('the card carries the same drag handle as the provider cards', await fixture(`hasHandle(${JSON.stringify(title)})`));
  await screenshot('populated-list');

  const cardButtons = await fixture(`cardButtonLabels(${JSON.stringify(title)})`);
  check('the sign-in entry lives on the card, not in the list toolbar', cardButtons[0], '登录');
  check(
    'the list toolbar no longer carries an official-account button',
    (await fixture('toolbarLabels()')).filter(label => label === title),
    [],
  );
  check(
    'each saved account offers a switch and a delete action',
    (await fixture(`cardButtonStates(${JSON.stringify(title)})`)).slice(1).map(button => button.label),
    ['切换', '删除', '切换', '删除'],
  );
  check(
    'the applied account is the one whose actions are withheld',
    (await fixture(`cardButtonStates(${JSON.stringify(title)})`)).slice(1).map(button => button.disabled),
    [true, true, false, false],
  );
  check(
    'the applied account is badged as the default rather than as an applied channel',
    await fixture(`cardText(${JSON.stringify(title)})`).then(text => text.includes('默认')),
  );
  check(
    'no sentence is rendered twice on the card',
    await fixture(`cardRepeatedSentences(${JSON.stringify(title)})`),
    [],
  );

  // --- dragging it past a provider card re-slots it ------------------------
  await dragCard(title, 'Provider C');
  check(
    'dragging the card past every provider moves it to the bottom',
    await fixture('waitForMembers(["Provider A","Provider B","Provider C",' + JSON.stringify(title) + '])'),
    ['Provider A', 'Provider B', 'Provider C', title],
  );
  await screenshot('dragged-to-bottom');
  check(
    'the new slot is persisted as a provider count, not as the card index',
    await fixture('savedIndices()'),
    [3],
  );
  check('the backend state matches the slot the user sees', await fixture('storedIndex()'), 3);

  // --- restored slot: index 2 puts it between B and C ----------------------
  await openFixture('providers=3&accounts=2&accountIndex=2');
  await waitFor('kimiOfficialAccountFixture.renderedMembers().length === 4', 'four list members');
  check(
    'a stored index of 2 draws the card below the second provider',
    await fixture('renderedMembers()'),
    ['Provider A', 'Provider B', title, 'Provider C'],
  );

  // --- empty list: the card is still the only way in ------------------------
  await openFixture('providers=0&accounts=0');
  await waitFor('kimiOfficialAccountFixture.renderedMembers().length === 1', 'the card alone');
  check(
    'an empty provider list still renders the card, so the sign-in entry is reachable',
    await fixture('renderedMembers()'),
    [title],
  );
  check(
    'with no account saved the card still offers sign-in',
    (await fixture(`cardButtonLabels(${JSON.stringify(title)})`))[0],
    '登录',
  );
  await screenshot('empty-list');

  return checks;
}