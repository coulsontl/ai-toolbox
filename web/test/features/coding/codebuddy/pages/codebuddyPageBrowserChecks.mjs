import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Visual + layout checks for the real CodeBuddy page.
 *
 * Same invariants as the WorkBuddy page, plus the two CodeBuddy-only sections:
 * `CODEBUDDY.md` prompt presets and the read-only `settings.json` preview.
 */
export async function verifyCodebuddyPage({ send, evaluate, baseUrl, artifactRoot }) {
  const theme = process.env.CODEBUDDY_PAGE_THEME === 'dark' ? 'dark' : 'light';
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
    url: `${baseUrl}/?theme=${theme}&runId=codebuddy-page-${Date.now()}`,
  });
  await waitFor('window.__codebuddyFixture && document.querySelectorAll("[data-provider-id]").length > 0');

  const channelCount = await evaluate('window.__codebuddyFixture.providerCardCount()');
  check('both channels render as cards', channelCount, 2);

  const sidebarTitles = JSON.parse(
    await evaluate('JSON.stringify(window.__codebuddyFixture.sidebarSectionTitles())'),
  );
  check('all four sections are marked for the sidebar', sidebarTitles.length, 4);
  check(
    'every sidebar marker carries a title',
    sidebarTitles.every((title) => typeof title === 'string' && title.trim().length > 0),
  );

  // `availableModels` is non-empty here, so the hint must be the restricted
  // variant — the two states are easy to wire to the wrong branch.
  const restrictedHint = await evaluate(
    'document.body.innerText.includes("availableModels")',
  );
  check('the restricted hint names availableModels', restrictedHint);

  const overflowingCards = await evaluate(`(() => {
    return [...document.querySelectorAll('[data-provider-id]')]
      .filter((card) => card.getBoundingClientRect().right > innerWidth + 1).length;
  })()`);
  check('no card overflows the viewport horizontally', overflowingCards, 0);

  const documentScrollsHorizontally = await evaluate(
    'document.documentElement.scrollWidth > document.documentElement.clientWidth + 1',
  );
  check('the page does not scroll horizontally', documentScrollsHorizontally, false);

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

  const providerFormLabels = JSON.parse(await evaluate(`(() => {
    ${visibleModal('.ant-modal')};
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    return JSON.stringify([...modal.querySelectorAll('.ant-form-item-label label')]
      .map((node) => node.textContent.trim()));
  })()`));
  check(
    'add-provider opens the provider form, not the model form',
    providerFormLabels,
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

  await evaluate(`(() => {
    ${visibleModal('.ant-modal')};
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    const close = modal.querySelector('.ant-modal-close');
    if (!close) throw new Error('modal close button not found');
    close.click();
  })()`);
  await delay(600);

  // SOP §4.2.4: the model dialog must offer the preset picker (ZCode shipped
  // without it and nothing else noticed).
  await evaluate(`(() => {
    const add = [...document.querySelectorAll('button')]
      .find((node) => node.textContent.trim() === '添加模型' && node.offsetParent !== null);
    if (!add) throw new Error('add-model button not found');
    add.click();
  })()`);
  await waitFor(`Boolean([...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).length)`);
  check(
    'the model dialog offers the preset-model picker',
    await evaluate(`(() => {
      const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
      return [...modal.querySelectorAll('a, button')]
        .some((node) => node.textContent.includes('选择预设模型'));
    })()`),
  );
  await evaluate(`(() => {
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    modal.querySelector('.ant-modal-close').click();
  })()`);
  await delay(400);

  // The card view is a density change, not the list card poured into a grid
  // cell: the full row's toolbar (five text buttons) broke 「模型列表」 onto one
  // character per line inside a 350px cell. Same rule as the WorkBuddy page, so
  // both pages guard it.
  await evaluate(`(() => {
    const toggle = document.querySelector('[data-provider-view-toggle="list"]');
    if (!toggle) throw new Error('view toggle not found');
    toggle.click();
  })()`);
  await waitFor('Boolean(document.querySelector("[data-provider-grid]"))');
  await delay(400);

  check(
    'the card view renders the compact grid, not the list inside a grid',
    await evaluate(`(() => {
      const cells = [...document.querySelectorAll('[data-provider-grid-cell]')];
      return cells.length > 0
        && cells.every((cell) => cell.querySelectorAll('[data-provider-card-footer]').length <= 1);
    })()`),
  );
  check(
    'a grid cell does not render the model list',
    await evaluate(`(() => {
      return [...document.querySelectorAll('[data-provider-grid-cell]')]
        .filter((cell) => cell.innerText.includes('模型列表')).length;
    })()`),
    0,
  );

  await evaluate(`(() => {
    const cell = document.querySelector('[data-provider-card-clickable]');
    if (!cell) throw new Error('no clickable cell');
    cell.click();
  })()`);
  await waitFor('Boolean(document.querySelector("[data-provider-detail-panel]"))');
  await delay(500);
  check(
    'the cell sidebar carries the model manager',
    await evaluate(`(() => {
      const panel = document.querySelector('[data-provider-detail-panel]');
      return Boolean(panel) && panel.innerText.includes('模型列表');
    })()`),
  );

  const sidebarShot = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(
    path.join(artifactRoot, `codebuddy-card-view-sidebar-${theme}.png`),
    Buffer.from(sidebarShot.data, 'base64'),
  );

  await evaluate(`(() => {
    const close = document.querySelector('[data-provider-detail-close]');
    if (!close) throw new Error('detail close button not found');
    close.click();
  })()`);
  await delay(400);
  await evaluate(`(() => {
    const toggle = document.querySelector('[data-provider-view-toggle="card"]');
    toggle?.click();
  })()`);
  await delay(400);

  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(
    path.join(artifactRoot, `codebuddy-page-${theme}.png`),
    Buffer.from(data, 'base64'),
  );

  return checks;
}
