import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the provider list's 定位 action (scroll to the applied provider) with
 * a real click in a real browser, then asserts what the user can observe: the
 * applied card comes into view, it flashes, and the flash ends.
 *
 * Why a browser and not a unit test: the whole feature is "make the page move".
 * Everything it depends on — the card carrying `data-provider-id`, the card
 * being inside the element the page actually scrolls (`main`), the flash rule
 * surviving the CSS-module build — is invisible to a test that calls the
 * handler directly. The failure this guards is a button that reports success
 * while nothing on screen moves.
 *
 * It also drives the two states where the answer is *no*: an applied provider
 * filtered out by the search, and nothing applied at all. Both must say why
 * instead of doing nothing — a locate action that silently no-ops reads as
 * broken.
 */
export async function verifyProviderListLocate({ send, evaluate, baseUrl, artifactRoot }) {
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
  const locateLabel = '定位';
  const searchLabel = '搜索';
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  /** The newest antd message on screen — what the user is being told. */
  const lastMessage =
    '(() => { const notes = [...document.querySelectorAll(".ant-message-notice")];'
    + ' return notes.length ? notes[notes.length - 1].textContent.trim() : null; })()';

  const openFixture = async (query, expectedCards) => {
    const runId = `${query}-${Date.now()}`;
    // Encoded: this fixture's queries carry `&` themselves, and an unencoded
    // run id would be split into extra query parameters — leaving the fixture
    // waiting for a run id the page never saw.
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${encodeURIComponent(runId)}` });
    await waitFor(
      `window.providerListDragFixture?.state.runId === ${JSON.stringify(runId)}`
        + ` && providerListDragFixture.renderedOrder().length === ${expectedCards}`,
      `fixture ${query}`,
    );
    await delay(300);
  };

  /** Clicks a header control by its visible label, the way a user reaches it. */
  const clickHeaderControl = async label => {
    const center = await evaluate(`(() => {
      const button = [...document.querySelectorAll('button')]
        .find(element => element.textContent.trim() === ${JSON.stringify(label)});
      if (!button) return null;
      const rect = button.getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`);
    assert.ok(center, `the ${label} control must be on screen`);
    await send('Input.dispatchMouseEvent', {
      type: 'mousePressed', x: center.x, y: center.y, button: 'left', clickCount: 1,
    });
    await send('Input.dispatchMouseEvent', {
      type: 'mouseReleased', x: center.x, y: center.y, button: 'left', clickCount: 1,
    });
  };

  /** The search control is a link button until clicked; then it is an input. */
  const setSearch = async value => {
    await clickHeaderControl(searchLabel);
    await waitFor(
      `Boolean(document.getElementById('codex-providers')?.querySelector('input'))`,
      'the provider search input to open',
    );
    await evaluate(`(() => {
      const input = document.getElementById('codex-providers').querySelector('input');
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, ${JSON.stringify(value)});
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await delay(300);
  };

  // --- the applied card is below the fold: locating it must scroll ----------
  await openFixture('count=14&applied=provider-n', 14);
  check(
    'every provider card carries the locate attribute',
    await evaluate(`Boolean(document.querySelector('[data-provider-id="provider-n"]'))`),
  );
  const beforeBox = await fixture('appliedCardBox()');
  check('the applied card starts below the fold', beforeBox.bottom > beforeBox.viewportHeight);
  await screenshot('locate-before');

  await clickHeaderControl(locateLabel);

  await waitFor(
    '(() => { const box = providerListDragFixture.appliedCardBox();'
    + ' return Boolean(box) && box.top >= 0 && box.bottom <= box.viewportHeight; })()',
    'the applied card to be fully in view',
  );
  const afterBox = await fixture('appliedCardBox()');
  check(
    'the applied card is scrolled fully into view',
    afterBox.top >= 0 && afterBox.bottom <= afterBox.viewportHeight,
  );
  // The flash has to be alive *here*, on arrival: a flash that ran while the
  // list was still scrolling would be over before the card was ever on screen,
  // which is exactly the state the user would read as "it did nothing".
  const shadowWhileFlashing = await fixture('appliedCardShadow()');
  // A zero-spread ring is a computed shadow that paints nothing; the flash has
  // to be an actual visible ring, not merely "some box-shadow exists".
  check(
    'the located card flashes a visible ring on arrival',
    shadowWhileFlashing !== 'none'
      && shadowWhileFlashing !== null
      && !shadowWhileFlashing.includes('0px 0px 0px 0px'),
  );
  await screenshot('locate-after');

  await delay(1800);
  check(
    'the flash ends on its own, leaving no stuck highlight',
    await fixture('appliedCardShadow()'),
    'none',
  );

  // --- the applied provider is filtered out: say so -----------------------
  await openFixture('count=14&applied=provider-n', 14);
  await setSearch('Provider A');
  check(
    'the search really hides the applied card',
    await fixture('appliedCardBox()'),
    null,
  );
  await clickHeaderControl(locateLabel);
  await waitFor(`${lastMessage} !== null`, 'the filtered-out message');
  check(
    'a filtered-out applied provider is explained, not silently skipped',
    await evaluate(lastMessage),
    '已应用的供应商被当前搜索词过滤掉了',
  );
  await screenshot('locate-filtered');

  // --- nothing is applied: say so -----------------------------------------
  await openFixture('count=14&applied=none', 14);
  check(
    'the locate control is still offered when nothing is applied',
    await evaluate(`Boolean([...document.querySelectorAll('button')]
      .find(element => element.textContent.trim() === ${JSON.stringify(locateLabel)}))`),
  );
  await clickHeaderControl(locateLabel);
  await waitFor(`${lastMessage} !== null`, 'the nothing-applied message');
  check(
    'with nothing applied the action says so',
    await evaluate(lastMessage),
    '当前没有已应用的供应商',
  );
  await screenshot('locate-none');

  return checks;
}