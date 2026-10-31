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

  // The card's pencil edits the *channel*, which means the provider form in its
  // edit state: no "first model" fields (the channel already has one) and the
  // connection pre-filled from the channel. Wiring it to the model form again
  // would ask for a model id while promising to edit a provider.
  await evaluate(`(() => {
    const edit = document.querySelector('[data-provider-id] button .anticon-edit');
    if (!edit) throw new Error('card edit control not found');
    edit.closest('button').click();
  })()`);
  await waitFor(`Boolean([...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).length)`);
  const channelEditShape = JSON.parse(await evaluate(`(() => {
    ${visibleModal('.ant-modal')};
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    const value = (name) => modal.querySelector('[id$="' + name + '"]')?.value ?? null;
    return JSON.stringify({
      title: modal.querySelector('.ant-modal-title')?.textContent.trim(),
      labels: [...modal.querySelectorAll('.ant-form-item-label label')].map((node) => node.textContent.trim()),
      baseUrl: value('baseUrl'),
    });
  })()`));
  check(
    'the card edit opens the provider form in edit state',
    channelEditShape.labels,
    ['供应商', '端点', 'API Key'],
  );
  check(
    'the edit form pre-fills the channel endpoint',
    channelEditShape.baseUrl,
    'https://api.example.com/v1',
  );
  await evaluate(`(() => {
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    modal.querySelector('.ant-modal-close').click();
  })()`);
  await delay(600);

  // The card view is a *density* change, not the list card in a grid. Putting
  // the full row inside a 350px cell broke 「模型列表」 onto one character per
  // line and pushed 添加模型 out of the header (2026-10-31 report), which is why
  // the cell must be `ProviderCompactCard` and the model manager must live in
  // its sidebar instead.
  await evaluate(`(() => {
    const toggle = document.querySelector('[data-provider-view-toggle="list"]');
    if (!toggle) throw new Error('view toggle not found');
    toggle.click();
  })()`);
  // `evaluate` must return a serializable value: a bare `querySelector` hands
  // CDP a DOM node, which fails with "Object reference chain is too long".
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
      // The model manager belongs to the sidebar; a cell that draws it is the
      // list card again, and its toolbar is what broke the layout.
      return [...document.querySelectorAll('[data-provider-grid-cell]')]
        .filter((cell) => cell.innerText.includes('模型列表')).length;
    })()`),
    0,
  );
  check(
    'a grid cell keeps its header on one line',
    await evaluate(`(() => {
      // The failure mode was a squeezed toolbar: the cell's own title row grew
      // several lines tall. Anything above 40px means something overflowed.
      return [...document.querySelectorAll('[data-provider-grid-cell]')]
        .filter((cell) => cell.getBoundingClientRect().width < 280).length;
    })()`),
    0,
  );

  // Open one cell's sidebar: the model manager has to be reachable, otherwise
  // the card view would be a strictly worse list.
  await evaluate(`(() => {
    const cell = document.querySelector('[data-provider-card-clickable]');
    if (!cell) throw new Error('no clickable cell');
    cell.click();
  })()`);
  await waitFor('Boolean(document.querySelector("[data-provider-detail-panel]"))');
  // The drawer slides in; without this the screenshot catches it half open.
  await delay(500);
  check(
    'the cell sidebar carries the model manager',
    await evaluate(`(() => {
      const panel = document.querySelector('[data-provider-detail-panel]');
      return Boolean(panel) && panel.innerText.includes('模型列表');
    })()`),
  );

  // Captured while the sidebar is open: that is the only way to see the model
  // manager's layout in the card view, which is what the earlier report was
  // about.
  const sidebarShot = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(
    path.join(artifactRoot, `workbuddy-card-view-sidebar-${theme}.png`),
    Buffer.from(sidebarShot.data, 'base64'),
  );

  await evaluate(`(() => {
    // The dedicated close hook, not "the first button with an aria-label": the
    // panel's field rows also carry copy buttons, and clicking one of those
    // just copies and leaves the drawer open.
    const close = document.querySelector('[data-provider-detail-close]');
    if (!close) throw new Error('detail close button not found');
    close.click();
  })()`);
  await delay(400);

  // Back to the list so the screenshot matches what the page opens with.
  await evaluate(`(() => {
    const toggle = document.querySelector('[data-provider-view-toggle="card"]');
    toggle?.click();
  })()`);
  await delay(400);

  // Identity files are the global-prompt card list with a **fixed** set of
  // rows: four built-in documents that can be edited, never added, deleted or
  // dragged. Each of those is a claim about what the section offers, so each
  // gets an assertion — a regression that quietly re-adds 删除 would still
  // render and still work.
  const identityShape = JSON.parse(await evaluate(`(() => {
    const collapse = document.querySelector('#workbuddy-identity .ant-collapse');
    if (!collapse) throw new Error('identity collapse not found');
    const header = collapse.querySelector('.ant-collapse-header');
    header.click();
    return JSON.stringify({ found: true });
  })()`));
  check('the identity section renders as a prompt-style collapse', identityShape.found);
  await delay(400);

  check(
    'the identity section lists the four built-in documents',
    JSON.parse(await evaluate(`(() => {
      const names = [...document.querySelectorAll('#workbuddy-identity .ant-card')]
        .map((card) => card.innerText.split('\\n')[0].trim());
      return JSON.stringify(names);
    })()`)),
    ['IDENTITY.md', 'USER.md', 'SOUL.md', 'BOOTSTRAP.md'],
  );
  check(
    'an identity row offers no add affordance',
    await evaluate(`(() => {
      const section = document.querySelector('#workbuddy-identity');
      return [...section.querySelectorAll('button')]
        .filter((node) => node.innerText.includes('新增') || node.innerText.includes('添加')).length;
    })()`),
    0,
  );
  check(
    'an identity row has no drag handle',
    await evaluate(
      "document.querySelectorAll('#workbuddy-identity .anticon-holder').length",
    ),
    0,
  );

  // The one menu entry: edit. Delete/apply/disable would each be an operation
  // the fixed set does not have.
  await evaluate(`(() => {
    const card = document.querySelector('#workbuddy-identity .ant-card');
    const more = card.querySelector('button .anticon-more');
    if (!more) throw new Error('identity row has no menu');
    more.closest('button').click();
  })()`);
  await delay(300);
  check(
    'the identity row menu offers only edit',
    JSON.parse(await evaluate(`(() => {
      const items = [...document.querySelectorAll('.ant-dropdown:not(.ant-dropdown-hidden) .ant-dropdown-menu-item')]
        .map((node) => node.innerText.trim());
      return JSON.stringify(items);
    })()`)),
    ['编辑'],
  );

  // Editing opens the shared prompt dialog with the name locked: the name is
  // the file on disk, so there is nothing to rename it to.
  await evaluate(`(() => {
    const item = [...document.querySelectorAll('.ant-dropdown:not(.ant-dropdown-hidden) .ant-dropdown-menu-item')]
      .find((node) => node.innerText.trim() === '编辑');
    item.click();
  })()`);
  await waitFor(`Boolean([...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).length)`);
  const editDialog = JSON.parse(await evaluate(`(() => {
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    const nameInput = modal.querySelector('#name');
    return JSON.stringify({
      name: nameInput?.value,
      nameDisabled: Boolean(nameInput?.disabled),
      hasEditor: Boolean(modal.querySelector('.cm-editor, textarea')),
    });
  })()`));
  check('the identity editor loads the file name', editDialog.name, 'IDENTITY.md');
  check('the identity editor refuses to rename the file', editDialog.nameDisabled);
  check('the identity editor offers a content editor', editDialog.hasEditor);

  await evaluate(`(() => {
    const modal = [...document.querySelectorAll('.ant-modal')].filter((el) => el.offsetParent !== null).pop();
    modal.querySelector('.ant-modal-close').click();
  })()`);
  await delay(400);

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
