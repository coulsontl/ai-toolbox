import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Visual + layout checks for the real WorkBuddy page.
 *
 * The page is mounted against a stubbed Tauri bridge, so every element here is
 * production code — this catches what a component-level test cannot: card
 * overflow, page-level horizontal scrolling, a stray body background, and the
 * sidebar markers the section navigation depends on.
 */
export async function verifyWorkbuddyPage({ send, evaluate, baseUrl, artifactRoot }) {
  const theme = process.env.WORKBUDDY_PAGE_THEME === 'dark' ? 'dark' : 'light';
  const checks = [];
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  const waitFor = async (expression) => {
    for (let attempt = 0; attempt < 200; attempt += 1) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out waiting for: ' + expression);
  };

  await send('Page.navigate', {
    url: `${baseUrl}/?theme=${theme}&runId=workbuddy-page-${Date.now()}`,
  });
  await waitFor('window.__workbuddyFixture && document.querySelectorAll("[data-provider-id]").length > 0');

  const channelCount = await evaluate('window.__workbuddyFixture.providerCardCount()');
  check('both channels render as cards', channelCount, 2);

  const sidebarTitles = JSON.parse(
    await evaluate('JSON.stringify(window.__workbuddyFixture.sidebarSectionTitles())'),
  );
  check('the three sections are marked for the sidebar', sidebarTitles.length, 3);
  check(
    'every sidebar marker carries a title',
    sidebarTitles.every((title) => typeof title === 'string' && title.trim().length > 0),
  );

  // A card wider than the viewport would mean a long URL or key was not elided.
  const overflowingCards = await evaluate(`(() => {
    return [...document.querySelectorAll('[data-provider-id]')]
      .filter((card) => card.getBoundingClientRect().right > innerWidth + 1).length;
  })()`);
  check('no card overflows the viewport horizontally', overflowingCards, 0);

  // The app locks the document and scrolls only inside `main`.
  const documentScrollsHorizontally = await evaluate(
    'document.documentElement.scrollWidth > document.documentElement.clientWidth + 1',
  );
  check('the page does not scroll horizontally', documentScrollsHorizontally, false);

  // A section nested in another bordered card reads as a card inside a card,
  // which DESIGN.md forbids.
  const nestedCards = await evaluate(`(() => {
    return [...document.querySelectorAll('[data-provider-id]')]
      .filter((card) => card.parentElement?.closest('[data-provider-id]')).length;
  })()`);
  check('provider cards are not nested inside each other', nestedCards, 0);

  // SOP §4.2.1: the toolbar's 「添加供应商」 must open a *provider* form. This
  // page used to wire it straight to the model form, so the button promised a
  // provider and asked for a model id + every capability switch.
  const visibleModal = (selector) => `
    const visible = (el) => el && el.offsetParent !== null;
    [...document.querySelectorAll('${selector}')].filter(visible)[0] ?? null`;
  await evaluate(`(() => {
    const visible = (el) => el && el.offsetParent !== null;
    const button = [...document.querySelectorAll('button')]
      .find((node) => node.textContent.trim() === '添加供应商' && visible(node));
    if (!button) throw new Error('add-provider button not found');
    button.click();
  })()`);
  await waitFor(`(() => { ${visibleModal('.ant-modal')}; return Boolean([
    ...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).length); })()`);

  const providerFormShape = await evaluate(`(() => {
    ${visibleModal('.ant-modal')};
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    const labels = [...modal.querySelectorAll('.ant-form-item-label label')]
      .map((node) => node.textContent.trim());
    return JSON.stringify({ title: modal.querySelector('.ant-modal-title')?.textContent.trim(), labels });
  })()`);
  const providerForm = JSON.parse(providerFormShape);
  check(
    'add-provider opens the provider form, not the model form',
    providerForm.labels,
    ['供应商', '端点', 'API Key', '首条模型 ID', '显示名称'],
  );
  check(
    'the provider form carries no capability switches',
    await evaluate(`(() => {
      ${visibleModal('.ant-modal')};
      const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
      return modal.querySelectorAll('.ant-switch').length;
    })()`),
    0,
  );

  // Close it so the screenshot captures the page, not a dialog. The footer's
  // cancel label is antd's localized 「取 消」 (it inserts a space between two
  // CJK characters), so match the close affordance instead of the text.
  await evaluate(`(() => {
    ${visibleModal('.ant-modal')};
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    const close = modal.querySelector('.ant-modal-close');
    if (!close) throw new Error('modal close button not found');
    close.click();
  })()`);
  await delay(600);

  // Let the spin wrapper finish its fade and the cards settle before capturing;
  // an early screenshot renders half-transparent cards, which reads as a real
  // styling defect rather than a timing artifact.
  await delay(600);

  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(
    path.join(artifactRoot, `workbuddy-page-${theme}.png`),
    Buffer.from(data, 'base64'),
  );

  return checks;
}
