import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Real-browser assertions for the four mapping layers migrated on 2026-10-20
 * (Claude Desktop, Gemini CLI, Grok, OmO Native's card view).
 *
 * Why a browser: every claim here is about what is rendered — which layout a
 * mapping layer picked, which facts a cell states, whether a footer spans the
 * card, whether a protected delete is greyed out. Source guards
 * (`verify-provider-card-layout`) only prove the mapping layer is thin; they
 * cannot see that the footer geometry regressed or that the row lost its sixth
 * action.
 */
export async function verifyMigratedProviderCards({ send, evaluate, baseUrl, artifactRoot }) {
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
  const fixture = expression => evaluate('migratedProviderCardsFixture.' + expression);
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  const openFixture = async (query) => {
    const runId = `${query}-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${encodeURIComponent(runId)}` });
    await waitFor(
      "Boolean(window.migratedProviderCardsFixture)"
        + " && Boolean(document.querySelector('[data-fixture=\"grok-custom-list\"]'))",
      `fixture ${query}`,
    );
    await delay(300);
  };

  /* ---------------------------------------------------------------- Claude Desktop */

  await openFixture('theme=light');

  // Role bindings keep their `Role: model` shape, with the value monospaced.
  const desktopText = await fixture('text("claude-desktop-list")');
  check(
    'the Claude Desktop card states each configured role as `Role: model`',
    /Sonnet:\s*claude-sonnet-4-5/.test(desktopText) && /Opus:\s*claude-opus-4-8/.test(desktopText),
  );

  // `tierAlias` is a real tag, not secondary text.
  const tierTag = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-list"]');
      const tags = [...root.querySelectorAll('.ant-tag')];
      return tags.some(tag => tag.textContent.includes('sonnet'));
    })()
  `);
  check('a role tier alias renders as a tag', tierTag);

  // Notes live in the footer, below the name row and spanning the card's body.
  const notesBelowHeader = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-list"]');
      const note = [...root.querySelectorAll('*')].find(node =>
        node.children.length === 0 && node.textContent.trim() === '团队共享额度');
      if (!note) return false;
      const name = [...root.querySelectorAll('*')].find(node =>
        node.children.length === 0 && node.textContent.includes('Desktop Relay'));
      return note.getBoundingClientRect().top > name.getBoundingClientRect().bottom;
    })()
  `);
  check('notes sit below the name row, not beside it', notesBelowHeader);

  // A disabled drag leaves no handle at all (the grip used to stay and do nothing).
  check('a drag-disabled card renders no handle', await fixture('hasHandle("claude-desktop-list")'), false);

  // With no role bindings the notes take that line's place (the bespoke card
  // drew them there too); the shared card must not drop the only text left.
  const desktopOfficialText = await fixture('text("claude-desktop-official-list")');
  check(
    'a binding-less Claude Desktop card still states its notes',
    desktopOfficialText.includes('官方渠道备注'),
  );

  // The official channel is titled by the app, not by its row: `🔗 官方账号`.
  // The glyph is antd's `LinkOutlined` (the shared `OfficialAccountHeadingIcon`),
  // so it is asserted as the icon element, not as literal text. The old
  // "官方订阅" tag said the same thing a second time and is gone.
  const officialHeading = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-official-list"]');
      return {
        text: root.textContent.replace(/\\s+/g, ' ').trim(),
        glyph: Boolean(root.querySelector('.anticon-link')),
      };
    })()
  `);
  check(
    'the official channel is headed 官方账号 with the shared glyph',
    officialHeading.glyph
      && officialHeading.text.startsWith('官方账号')
      && !officialHeading.text.includes('官方订阅')
      && !officialHeading.text.includes('Claude Desktop Official'),
  );

  // ...and it cannot be deleted: the "more" menu (where the list card keeps
  // delete) has no 删除 entry at all, rather than a disabled one.
  const officialMenuHasDelete = await evaluate(`
    (async () => {
      const root = document.querySelector('[data-fixture="claude-desktop-official-list"]');
      const more = root.querySelector('button .anticon-more')?.closest('button');
      if (!more) return 'no-menu-button';
      const mountedBefore = new Set(document.querySelectorAll('.ant-dropdown'));
      more.click();
      await new Promise(resolve => setTimeout(resolve, 300));
      const menu = [...document.querySelectorAll('.ant-dropdown')]
        .find(node => !mountedBefore.has(node));
      const items = [...(menu?.querySelectorAll('[role="menuitem"]') ?? [])]
        .map(item => item.textContent.replace(/\\s+/g, ' ').trim());
      document.body.click();
      await new Promise(resolve => setTimeout(resolve, 150));
      return items.some(item => item.includes('删除'));
    })()
  `);
  check('the official channel has no delete entry in its menu', officialMenuHasDelete, false);

  // The meta line never wraps: the facts compress and the trailing
  // 「模型测试」 keeps its place on the same line (user-reported 2026-10-21 —
  // a long endpoint plus all four role bindings pushed the action onto a
  // second row). The fixture reproduces exactly that shape, so a passing
  // check means compression actually happened, not that the row was short.
  const metaLineGeometry = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-long-list"]');
      const code = [...root.querySelectorAll('code')]
        .find(node => node.textContent.includes('a-very-long-endpoint-name'));
      const test = [...root.querySelectorAll('button')]
        .find(node => node.textContent.replace(/\\s+/g, ' ').trim() === '模型测试');
      if (!code || !test) return null;
      // antd renders \`<span class="ant-typography">…<code/></span>\`; the
      // ellipsis class and the title live on the spans around it.
      const typography = code.closest('.ant-typography');
      const entry = typography?.parentElement;
      const row = entry?.parentElement;
      const codeRect = code.getBoundingClientRect();
      const testRect = test.getBoundingClientRect();
      const rowRect = row?.getBoundingClientRect();
      const cardRect = root.querySelector('.ant-card').getBoundingClientRect();
      return {
        rowHeight: rowRect ? Math.round(rowRect.height) : -1,
        centerDelta: Math.abs(
          (codeRect.top + codeRect.bottom) / 2 - (testRect.top + testRect.bottom) / 2,
        ),
        insideCard: testRect.right <= cardRect.right + 1,
        entryTitle: entry?.getAttribute('title') ?? '',
      };
    })()
  `);
  // A single line is ~20px; a wrap would roughly double the row height. The
  // fixture reproduces the reported shape (a long endpoint plus all four role
  // bindings), so this only passes when the facts actually compressed.
  check(
    'the meta line keeps 模型测试 on the same row as the facts',
    Boolean(metaLineGeometry)
      && metaLineGeometry.rowHeight > 0
      && metaLineGeometry.rowHeight <= 26
      && metaLineGeometry.centerDelta <= 6
      && metaLineGeometry.insideCard,
  );
  check(
    'a compressed entry still carries its full value on hover',
    Boolean(metaLineGeometry)
      && metaLineGeometry.entryTitle.includes('a-very-long-endpoint-name'),
  );

  // A claude-safe provider that maps routes onto other claude ids needs the
  // gateway (Direct 3P mode forbids model mapping). It must offer
  // 「应用并代理」 and no bare 应用 — the AnyRouter case, 2026-10-21.
  const mappedActionLabels = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-mapped-list"]');
      return [...root.querySelectorAll('button')]
        .map(button => button.textContent.replace(/\\s+/g, ' ').trim())
        .filter(Boolean);
    })()
  `);
  check(
    'a mapped Claude Desktop provider applies through the gateway',
    mappedActionLabels.includes('应用并代理') && !mappedActionLabels.includes('应用'),
  );

  // The grid cell states the role bindings on its second line.
  const desktopSubtitle = await fixture('subtitle("claude-desktop-card")');
  check(
    'the Claude Desktop cell states its role bindings',
    Boolean(desktopSubtitle && /Sonnet:\s*claude-sonnet-4-5/.test(desktopSubtitle)),
  );

  // The official channel has no endpoint/key/binding to state, and no account
  // list of its own: its cell states the sign-in owner instead of leaving the
  // reserved line blank, and offers no connectivity probe it cannot run.
  const desktopOfficialSubtitle = await fixture('subtitle("claude-desktop-official-card")');
  check(
    'the official Claude Desktop cell states who owns the sign-in',
    Boolean(desktopOfficialSubtitle
      && desktopOfficialSubtitle.includes('官方登录')
      && desktopOfficialSubtitle.includes('由 Claude Desktop 管理')),
  );
  check(
    'the official Claude Desktop card offers no connectivity probe',
    !(await fixture('text("claude-desktop-official-list")')).includes('模型测试'),
  );

  /* ---------------------------------------------------------------- Gemini CLI */

  const geminiText = await fixture('text("gemini-list")');
  check(
    'the Gemini official card is titled with the shared heading and its count',
    /官方账号\s*\(2\)/.test(geminiText),
  );
  check(
    'the Gemini official card explains switching on its second line',
    geminiText.includes('替换当前登录'),
  );

  // The account block's rows must reach the card's content edge; the footer
  // used to sit in the header's left column and stopped ~160px short.
  //
  // The list card's fold starts closed (it always has), so open it first.
  check('the Gemini account list can be opened from its title', await fixture('expandAccounts("gemini-list")'));
  await delay(200);
  const rowEdges = await fixture('edges("gemini-list", "[style*=\\"border-bottom\\"]")');
  const cardContentEdges = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="gemini-list"]');
      const card = root.querySelector('.ant-card');
      const body = card.querySelector('.ant-card-body');
      const style = getComputedStyle(body);
      const rect = body.getBoundingClientRect();
      return {
        right: Math.round(rect.right - parseFloat(style.paddingRight)),
        left: Math.round(rect.left + parseFloat(style.paddingLeft)),
      };
    })()
  `);
  check(
    'the account rows span the card content width',
    Boolean(rowEdges) && Math.abs(rowEdges.right - cardContentEdges.right) <= 2,
  );

  // The official channel's second line is the explanation of what applying it
  // does, with the user's notes after it (the official branch used to replace
  // the whole line, and Claude Desktop's line was empty).
  check(
    'the official Claude Desktop card explains what applying it does',
    desktopOfficialText.includes('官方登录模式')
      && desktopOfficialText.includes('官方渠道备注'),
  );

  // A virtual (disk-only) login can only be saved.
  const virtualRowText = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="gemini-list"]');
      const row = [...root.querySelectorAll('[style*="border-bottom"]')]
        .find(node => node.textContent.includes('local@example.test'));
      return row ? row.textContent.replace(/\\s+/g, ' ').trim() : null;
    })()
  `);
  check(
    'the virtual login row offers 「保存当前登录」 and no switch',
    Boolean(virtualRowText
      && virtualRowText.includes('保存当前登录')
      && !virtualRowText.includes('切换')),
  );

  // The applied stored account offers no switch either (it is the current one).
  const appliedRowText = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="gemini-list"]');
      const row = [...root.querySelectorAll('[style*="border-bottom"]')]
        .find(node => node.textContent.includes('work@example.test'));
      return row ? row.textContent.replace(/\\s+/g, ' ').trim() : null;
    })()
  `);
  check(
    'the applied account is marked default instead of offering a switch',
    Boolean(appliedRowText && appliedRowText.includes('默认') && !appliedRowText.includes('切换')),
  );

  // The cell states the login in use.
  const geminiSubtitle = await fixture('subtitle("gemini-card")');
  check(
    'the Gemini official cell states the current login',
    Boolean(geminiSubtitle && geminiSubtitle.includes('当前登录') && geminiSubtitle.includes('work@example.test')),
  );

  /* ---------------------------------------------------------------- Grok */

  const grokOfficialText = await fixture('text("grok-official-list")');
  check(
    'the Grok official card still states its notes beside the switching hint',
    grokOfficialText.includes('官方渠道备注'),
  );
  check(
    'the Grok official card is titled with the shared heading',
    /官方账号\s*\(1\)/.test(grokOfficialText),
  );
  // The fold starts closed on the list card; the sixth action lives on a row.
  check('the Grok account list can be opened from its title', await fixture('expandAccounts("grok-official-list")'));
  await delay(200);
  const grokExpandedText = await fixture('text("grok-official-list")');
  check(
    'Grok keeps its sixth account action through the shared extra-action slot',
    grokExpandedText.includes('刷新 Token'),
  );

  // The refresh-token action is absent from the custom provider (no accounts).
  check(
    'a custom Grok provider renders no account block',
    !(await fixture('text("grok-custom-list")')).includes('刷新 Token'),
  );

  // The model toolbar keeps the documented order:
  // 批量删除 → 模型测试 → 获取模型 → 添加模型.
  //
  // Searched from the catalog's own title, not from the top of the card: the
  // header's inline connectivity action carries the same words (模型测试), and
  // an unscoped `indexOf` would score that one as the first toolbar item.
  const toolbarOrder = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="grok-custom-list"]');
      const text = root.textContent.replace(/\s+/g, ' ');
      const from = text.indexOf('模型列表');
      if (from < 0) return [-1, -1, -1, -1];
      return ['批量删除', '模型测试', '获取模型', '添加模型']
        .map(label => text.indexOf(label, from));
    })()
  `);
  check(
    'the Grok model toolbar keeps its documented order',
    toolbarOrder.every((index, position) => index >= 0 && (position === 0 || index > toolbarOrder[position - 1])),
  );

  // The toolbar is this CLI's only connectivity entry: the bespoke card never
  // had a second one in the header, so the migrated card must not add it.
  const grokTestEntries = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="grok-custom-list"]');
      const text = root.textContent.replace(/\\s+/g, ' ');
      return text.split('模型测试').length - 1;
    })()
  `);
  check('the custom Grok card offers connectivity in one place only', grokTestEntries, 1);

  // The custom card's second line states the endpoint, model and masked key.
  //
  // No protocol tag here: `apiFormatLabel` comes from `openAiApiFormatFromBaseUrl`
  // / the config's own `api` key, and this fixture's bare endpoint resolves to
  // none. The tag's presence is covered by the list card in the app; what this
  // check guards is that the free-form entries survived the migration.
  const grokCustomText = await fixture('text("grok-custom-list")');
  check(
    'the custom Grok card states endpoint, model and masked key',
    grokCustomText.includes('https://grok.example.test/v1')
      && grokCustomText.includes('grok-4 (high)')
      && grokCustomText.includes('API Key:'),
  );

  // An official channel with no saved login cannot be applied: the action stays
  // visible (a missing button reads as a missing feature) but locked, and it
  // explains itself on hover — the shared `primaryAction.disabledReason`.
  const grokEmptyApply = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="grok-official-empty-list"]');
      const button = [...root.querySelectorAll('button')]
        .find(node => node.textContent.replace(/\\s+/g, ' ').trim() === '应用');
      if (!button) return 'no-apply-button';
      return { disabled: button.disabled === true };
    })()
  `);
  check(
    'the unsigned-in Grok official card keeps a locked 应用 button',
    Boolean(grokEmptyApply && grokEmptyApply.disabled === true),
  );

  /* ---------------------------------------------------------------- OmO Native */

  const omoDefaultSubtitle = await fixture('subtitle("omo-default-card")');
  check(
    'the OmO cell states the npm package',
    Boolean(omoDefaultSubtitle && omoDefaultSubtitle.startsWith('@ai-sdk/')),
  );
  check(
    'the provider that owns the default states it',
    Boolean(omoDefaultSubtitle && omoDefaultSubtitle.includes('当前主模型')),
  );

  const omoOtherSubtitle = await fixture('subtitle("omo-other-card")');
  check(
    'a non-default OmO cell states only the package',
    Boolean(omoOtherSubtitle && omoOtherSubtitle.startsWith('@ai-sdk/') && !omoOtherSubtitle.includes('当前主模型')),
  );

  const defaultDelete = await fixture('deleteMenuItem("omo-default-card")');
  check(
    'the default provider’s menu greys out delete and explains why',
    Boolean(defaultDelete && defaultDelete.found && defaultDelete.disabled),
  );

  const otherDelete = await fixture('deleteMenuItem("omo-other-card")');
  check(
    'an ordinary provider’s menu keeps delete live',
    Boolean(otherDelete && otherDelete.found && !otherDelete.disabled),
  );

  /* ---------------------------------------------------------------- OpenClaw */

  const openClawText = await fixture('text("openclaw-list")');
  check(
    'the OpenClaw card states its id, npm package and endpoint',
    openClawText.includes('relay-main')
      && openClawText.includes('@ai-sdk/openai-compatible')
      && openClawText.includes('https://relay.openclaw.example.test/v1'),
  );
  check(
    'the OpenClaw card offers no provider-level 应用',
    !openClawText.includes('应用'),
  );

  // The catalog toolbar is the only place the probes live now: the bespoke card
  // had them in a header `extraActions` slot the shared styles do not have.
  const openClawToolbar = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="openclaw-list"]');
      const buttons = [...root.querySelectorAll('button')].map(node => node.textContent.replace(/\\s+/g, ' ').trim());
      return {
        batch: buttons.some(text => text.includes('批量删除')),
        test: buttons.some(text => text.includes('模型测试')),
        fetch: buttons.some(text => text.includes('获取模型')),
        add: buttons.some(text => text.includes('添加模型')),
      };
    })()
  `);
  check(
    'the OpenClaw model toolbar carries batch delete, test, fetch and add',
    openClawToolbar.batch && openClawToolbar.test && openClawToolbar.fetch && openClawToolbar.add,
  );

  check('the OpenClaw cell states the npm package', await fixture('subtitle("openclaw-card")') === '@ai-sdk/openai-compatible');

  // The OpenCode-style list card has no "more" menu — delete is an icon button
  // in the header, which the default provider's guard must grey out.
  const openClawDefaultDelete = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="openclaw-default-list"]');
      const deleteButton = [...root.querySelectorAll('button')]
        .find(node => node.querySelector('.anticon-delete'));
      if (!deleteButton) return { found: false };
      // A disabled antd button drops pointer events, so the tooltip has to hang
      // off a wrapper element — the button's direct parent must be that span.
      return {
        found: true,
        disabled: deleteButton.disabled === true,
        hasTooltipHost: deleteButton.parentElement?.tagName === 'SPAN',
      };
    })()
  `);
  check(
    'the default OpenClaw provider greys out delete',
    Boolean(openClawDefaultDelete.found && openClawDefaultDelete.disabled),
  );
  check(
    'the disabled OpenClaw delete keeps a hover target for its reason',
    Boolean(openClawDefaultDelete.hasTooltipHost),
  );

  /* ---------------------------------------------------------------- Hermes */

  const hermesText = await fixture('text("hermes-list")');
  check(
    'the Hermes card states its endpoint and API mode',
    hermesText.includes('https://api.anthropic.com') && hermesText.includes('anthropic'),
  );
  check(
    'the Hermes card offers no provider-level 应用',
    !hermesText.includes('应用'),
  );
  // No new "default" badge on the card: the fact was already stated on the
  // catalog row (`当前主模型`), and a migration does not add state furniture.
  // Scoped to the header line, which is where a badge would have gone.
  const hermesNameLine = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="hermes-list"]');
      const name = [...root.querySelectorAll('*')]
        .find(node => node.children.length === 0 && node.textContent.includes('Anthropic Direct'));
      return name?.parentElement?.textContent.replace(/\\s+/g, ' ').trim() ?? '';
    })()
  `);
  check(
    'the Hermes card does not re-state the default as a name-line badge',
    !hermesNameLine.includes('当前主模型'),
  );

  // The catalog rows live behind the card's fold; read them after opening it.
  check('the Hermes card opens its model catalog', await fixture('expandModels("hermes-list")'));
  const hermesRows = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="hermes-list"]');
      const rows = [...root.querySelectorAll('[data-model-row]')].map(node => ({
        text: node.textContent.replace(/\\s+/g, ' ').trim(),
        grip: Boolean(node.querySelector('[class*="grip"]')),
        setPrimary: [...node.querySelectorAll('button')]
          .some(button => button.textContent.includes('设为主模型')),
      }));
      return rows;
    })()
  `);
  check(
    'the Hermes model row that owns the default says so',
    Boolean(hermesRows.find(row => row.text.includes('claude-opus-4-8'))?.text.includes('当前主模型')),
  );
  check(
    'another Hermes model row offers 设为主模型',
    Boolean(hermesRows.find(row => row.text.includes('claude-sonnet-4-5'))?.setPrimary),
  );
  check(
    'Hermes model rows carry no drag grip (the CLI has no model reorder)',
    hermesRows.every(row => !row.grip),
  );
  // The provider card itself does drag: `custom_providers` is a YAML sequence,
  // and `reorder_hermes_providers` rewrites it. A model grip would drag nothing;
  // the card grip is the one that persists.
  check(
    'the Hermes provider card carries a drag grip',
    await evaluate(`Boolean(document.querySelector('[data-fixture="hermes-list"] [data-provider-card-handle]'))`),
  );

  const hermesReadOnly = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="hermes-readonly-list"]');
      const text = root.textContent.replace(/\\s+/g, ' ').trim();
      // The catalog toolbar always carries 批量删除 / 添加模型; what the read-only
      // state must remove is the *provider* actions (header icon buttons).
      const headerIcons = [...root.querySelectorAll('button')]
        .map(node => node.querySelector('.anticon')?.className ?? '')
        .filter(name => name);
      return {
        tagged: text.includes('只读'),
        edit: headerIcons.some(name => name.includes('anticon-edit')),
        copy: headerIcons.some(name => name.includes('anticon-copy')),
        del: headerIcons.some(name => name.includes('anticon-delete')),
      };
    })()
  `);
  check(
    'a read-only Hermes provider is tagged 只读',
    hermesReadOnly.tagged,
  );
  check(
    'a read-only Hermes provider offers no edit, copy or delete',
    !hermesReadOnly.edit && !hermesReadOnly.copy && !hermesReadOnly.del,
  );
  const hermesReadOnlyText = await fixture('text("hermes-readonly-list")');
  check(
    'a read-only Hermes provider does not pretend its key is its endpoint',
    !hermesReadOnlyText.includes('(只读)'),
  );
  // `reorder_hermes_providers` rewrites the custom list only, so a grip on a
  // `providers:` dict row would move a card and persist nothing.
  check(
    'a read-only Hermes provider carries no drag grip',
    await evaluate(`!document.querySelector('[data-fixture="hermes-readonly-list"] [data-provider-card-handle]')`),
  );

  /* ---------------------------------------------- selection mode (checkbox) */

  /** Reads the left-slot control of a fixture block. */
  const readSelectionSlot = name => evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="${name}"]');
      const input = root?.querySelector('input[type="checkbox"]');
      return {
        hasCheckbox: Boolean(input),
        disabled: input ? input.disabled : null,
        hasHandle: Boolean(root?.querySelector('[data-provider-card-handle]')),
      };
    })()
  `);

  const eligibleSlot = await readSelectionSlot("hermes-selection-list");
  check('an eligible row in selection mode shows a live checkbox',
    eligibleSlot.hasCheckbox && eligibleSlot.disabled === false);
  check('the checkbox replaces the drag handle in selection mode', !eligibleSlot.hasHandle);

  const blockedSlot = await readSelectionSlot("hermes-selection-disabled-list");
  check('an ineligible row still shows a checkbox instead of a blank slot',
    blockedSlot.hasCheckbox);
  check('the ineligible row’s checkbox is disabled', blockedSlot.disabled === true);
  check('the ineligible row shows no drag handle either', !blockedSlot.hasHandle);

  // Hover the disabled checkbox: its reason must be reachable, which a bare
  // disabled <input> would swallow.
  // These fixture blocks stack past the fold, and a hover on an off-screen
  // element never reaches the tooltip: scroll it in first, then measure.
  await evaluate(`document.querySelector('[data-fixture="hermes-selection-disabled-list"]')
    ?.scrollIntoView({ block: 'center' })`);
  await delay(300);
  const blockedCheckboxCenter = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="hermes-selection-disabled-list"]');
      const box = root?.querySelector('input[type="checkbox"]')?.parentElement;
      if (!box) return null;
      const rect = box.getBoundingClientRect();
      if (rect.width === 0 || rect.top < 0 || rect.bottom > innerHeight) return null;
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()
  `);
  assert.ok(blockedCheckboxCenter, 'the disabled checkbox must be on screen');
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...blockedCheckboxCenter });
  await delay(600);
  check(
    'hovering the disabled checkbox explains why the row cannot be picked',
    await evaluate(`document.querySelector('.ant-tooltip')?.textContent?.trim() ?? null`),
    '该渠道已设为默认，不可多选',
  );

  const readOnlySlot = await readSelectionSlot("hermes-readonly-selection-list");
  check('a read-only row in selection mode is disabled too, not blank',
    readOnlySlot.hasCheckbox && readOnlySlot.disabled === true);

  const hermesCardSubtitle = await fixture('subtitle("hermes-card")');
  check(
    'the Hermes cell states the SDK family and the default model',
    Boolean(hermesCardSubtitle
      && hermesCardSubtitle.startsWith('@ai-sdk/anthropic')
      && hermesCardSubtitle.includes('当前主模型')),
  );

  await screenshot('migrated-provider-cards-light');

  /* ---------------------------------------------------------------- dark theme */

  await openFixture('theme=dark');
  const darkCardBg = await evaluate(`
    (() => {
      const root = document.querySelector('[data-fixture="claude-desktop-card"]');
      const card = root.querySelector('[class*="card"]');
      return card ? getComputedStyle(card).backgroundColor : null;
    })()
  `);
  check('the migrated cards render in dark theme', Boolean(darkCardBg) && darkCardBg !== 'rgb(255, 255, 255)');
  await screenshot('migrated-provider-cards-dark');

  return checks;
}
