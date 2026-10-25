import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/** Real CodexPage, real API wrappers, synthetic account/usage IPC only. */
export async function verifyCodexCredits({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      // CDP returnByValue cannot serialize DOM nodes with React internals.
      if (await evaluate(`Boolean(${expression})`)) return;
      await delay(100);
    }
    throw new Error('Timed out waiting for: ' + label);
  };
  const listLoading = "document.querySelector('#codex-providers .ant-spin-spinning')";
  const row = email => `codexCardViewFixture.accountRow(${JSON.stringify(email)})`;
  const first = 'first@example.invalid';
  const second = 'second@example.invalid';
  const click = async (email, icon) => evaluate(
    `codexCardViewFixture.clickAccountAction(${JSON.stringify(email)}, ${JSON.stringify(icon)}), true`,
  );
  const respond = async (patch, error, email = first) => {
    await evaluate(`codexCardViewFixture.respondUsage(${JSON.stringify(patch)}, ${JSON.stringify(error)}), true`);
    await waitFor(`!${row(email)}?.querySelector('.ant-btn-loading')`, 'refresh finished');
    await delay(100);
  };
  const refresh = async (patch) => {
    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'usage request pending');
    await respond(patch);
  };
  const closeModal = async () => {
    await evaluate("document.querySelector('.ant-modal-close')?.click(), true");
    await waitFor("!document.querySelector('.ant-modal-wrap:not([style*=\"display: none\"])')", 'details closed');
  };

  for (const [language, theme, viewMode] of [
    ['zh-CN', 'light', 'list'], ['en-US', 'dark', 'card'],
    ['zh-CN', 'dark', 'card'], ['en-US', 'light', 'list'],
  ]) {
    const prefix = `${language}/${theme}/${viewMode}`;
    const label = language === 'zh-CN' ? '额外积分余额' : 'Additional credits';
    const unit = language === 'zh-CN' ? '积分' : 'credits';
    const unlimited = language === 'zh-CN' ? '不限量' : 'Unlimited';
    const unknown = language === 'zh-CN' ? '未知，请刷新额度' : 'Unknown; refresh usage';
    const runId = `${prefix}-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?credits=1&language=${language}&theme=${theme}&viewMode=${viewMode}&runId=${encodeURIComponent(runId)}` });
    await waitFor(`window.codexCardViewFixture?.state.runId === ${JSON.stringify(runId)}
      && document.querySelector('[data-provider-id="provider-c"]')`, 'page loaded');
    if (viewMode === 'card') {
      const clickable = '[data-provider-id="provider-c"] [data-provider-card-clickable]';
      await waitFor(`document.querySelector(${JSON.stringify(clickable)})`, 'provider card action');
      await evaluate(`document.querySelector(${JSON.stringify(clickable)}).click(), true`);
    } else {
      await waitFor("document.querySelector('[data-provider-id=\"provider-c\"] .anticon-right')", 'account toggle');
      await evaluate("document.querySelector('[data-provider-id=\"provider-c\"] .anticon-right').closest('button').click(), true");
    }
    await waitFor(row(first), 'account row');
    check(prefix + ' omits unknown credits from account summary', await evaluate(`${row(first)}.textContent.includes(${JSON.stringify(label)})`), false);
    await click(first, 'eye');
    await waitFor("document.querySelector('.ant-descriptions')", 'account details');
    check(prefix + ' shows unknown rather than zero in details', await evaluate(`document.querySelector('.ant-descriptions').textContent.includes(${JSON.stringify(unknown)})`));
    await closeModal();

    await evaluate(`window.pendingCreditsRefreshButton = codexCardViewFixture.accountAction(${JSON.stringify(first)}, 'sync'), true`);
    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'pending credits read');
    check(prefix + ' same refresh button indicates loading without inventing credits', await evaluate(`
      window.pendingCreditsRefreshButton === codexCardViewFixture.accountAction(${JSON.stringify(first)}, 'sync')
      && window.pendingCreditsRefreshButton.classList.contains('ant-btn-loading')
      && !${row(first)}.textContent.includes(${JSON.stringify(label)})`));
    await click(first, 'sync');
    check(prefix + ' suppresses duplicate refresh while pending', await evaluate('codexCardViewFixture.pendingUsageCount()'), 1);
    await respond({ credits: { hasCredits: true, unlimited: false, balance: 62500.25 }, limitWeeklyText: '75%', resetCreditsAvailable: 2 });
    check(prefix + ' keeps weekly and extra credits together', await evaluate(`${row(first)}.textContent.includes('75%') && ${row(first)}.textContent.includes('62500.25 ${unit}')`));
    check(prefix + ' does not leak credits to another account', await evaluate(`${row(second)}.textContent.includes('62500.25')`), false);
    await click(first, 'eye');
    await waitFor("document.querySelector('.ant-descriptions')", 'refreshed details');
    check(prefix + ' details carries same balance and last successful fetch', await evaluate("document.querySelector('.ant-descriptions').textContent.includes('62500.25') && document.querySelector('.ant-descriptions').textContent.includes('2026')"));
    const screenshot = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, `${language}-${theme}-credits.png`), Buffer.from(screenshot.data, 'base64'));
    await closeModal();

    for (const [balance, hasCredits] of [[0, true], [-0.25, false], [123, false]]) {
      await refresh({ credits: { hasCredits, unlimited: false, balance } });
      check(prefix + ` preserves ${balance} balance with availability=${hasCredits}`, await evaluate(`${row(first)}.textContent.includes('${balance} ${unit}')`));
    }
    await refresh({ credits: { hasCredits: false, unlimited: true, balance: 0 } });
    check(prefix + ' unlimited takes precedence over finite balance', await evaluate(`${row(first)}.textContent.includes(${JSON.stringify(unlimited)})`));
    await refresh({ credits: { hasCredits: true } });
    check(prefix + ' unknown balance is not zero', await evaluate(`${row(first)}.textContent.includes(${JSON.stringify(language === 'zh-CN' ? '可用（余额未知）' : 'Available (balance unknown)')})`));
    await refresh({ credits: null });
    check(prefix + ' missing fresh credits clears previous display', await evaluate(`${row(first)}.textContent.includes(${JSON.stringify(label)})`), false);
    await refresh({ credits: { hasCredits: true, balance: 7.25 }, limitWeeklyText: null, limitMonthlyText: null, resetCreditsAvailable: null });
    check(prefix + ' credits do not require a weekly window', await evaluate(`${row(first)}.textContent.includes('7.25 ${unit}')`));

    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'failure read pending');
    await respond(null, 'Synthetic usage unavailable');
    check(prefix + ' failed refresh shows an error and hides stale balance', await evaluate(`${row(first)}.textContent.includes('Synthetic usage unavailable') && !${row(first)}.textContent.includes('7.25')`));
    await click(first, 'eye');
    await waitFor("document.querySelector('.ant-descriptions')", 'failed details');
    check(prefix + ' failed details shows unknown balance', await evaluate(`document.querySelector('.ant-descriptions').textContent.includes(${JSON.stringify(unknown)})`));
    await closeModal();
    await refresh({ credits: { hasCredits: true, balance: 9 } });
    check(prefix + ' retry recovers credits', await evaluate(`${row(first)}.textContent.includes('9 ${unit}')`));

    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'same-login refresh pending');
    await click(first, 'eye');
    await waitFor("document.querySelector('.ant-descriptions')", 'details while usage is pending');
    const listReads = await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length");
    await evaluate('codexCardViewFixture.reloadAccounts(codexCardViewFixture.state.officialAccounts), true');
    await waitFor(`codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > ${listReads}`, 'same-login list reload');
    await respond(null, 'Failure after list reload');
    await waitFor(`${row(first)}?.textContent.includes('Failure after list reload')`, 'late failure reconciled');
    check(prefix + ' list reload cannot resurrect stale credits after failure', await evaluate(`${row(first)}.textContent.includes('9 ${unit}')`), false);
    check(prefix + ' open details reconciles the same late failure', await evaluate(`document.querySelector('.ant-descriptions').textContent.includes(${JSON.stringify(unknown)})`));
    await closeModal();
    await refresh({ credits: { hasCredits: true, balance: 9 } });

    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'usage before deferred list');
    await evaluate('codexCardViewFixture.deferNextAccountList(), codexCardViewFixture.reloadAccounts(codexCardViewFixture.state.officialAccounts), true');
    await waitFor('codexCardViewFixture.pendingAccountListCount() === 1', 'old list snapshot pending');
    await waitFor(listLoading, 'old list is visibly loading');
    await respond({ credits: { hasCredits: true, balance: 16 } });
    await evaluate('codexCardViewFixture.respondAccountList(), true');
    // Wait for React to apply the released list, not a balance already on screen.
    await waitFor(`!(${listLoading})`, 'old list finished applying');
    check(prefix + ' late old account list cannot roll back a completed refresh', await evaluate(`${row(first)}.textContent.includes('16 ${unit}')`));

    await click(first, 'eye');
    await waitFor("document.querySelector('.ant-descriptions')", 'details before pending close');
    await closeModal();
    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'late result');
    await respond({ credits: { hasCredits: true, balance: 10 } });
    check(prefix + ' refresh does not reopen dismissed details', await evaluate("Boolean(document.querySelector('.ant-modal-wrap:not([style*=\"display: none\"])'))"), false);

    // Reuse the same account id after a root/login reload, then complete its old read.
    await click(first, 'sync');
    await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'old login pending');
    await evaluate(`codexCardViewFixture.reloadAccounts(codexCardViewFixture.state.officialAccounts.map(a => a.id === 'account-1'
      ? { ...a, email: 'replacement@example.invalid', accountId: 'replacement', credits: {hasCredits: true, balance: 88} } : a)), true`);
    await waitFor(row('replacement@example.invalid'), 'new login list');
    await respond({ credits: { hasCredits: true, balance: 999 } });
    check(prefix + ' late previous login never overwrites replacement credits', await evaluate(`${row('replacement@example.invalid')}.textContent.includes('88 ${unit}') && !${row('replacement@example.invalid')}.textContent.includes('999')`));
    check(prefix + ' no horizontal viewport overflow', await evaluate('document.documentElement.scrollWidth <= window.innerWidth'));
  }

  const live = 'live@example.invalid';
  const runId = `virtual-${Date.now()}`;
  await send('Page.navigate', { url: `${baseUrl}/?credits=1&accounts=virtual&viewMode=list&runId=${runId}` });
  await waitFor(`window.codexCardViewFixture?.state.runId === '${runId}'
    && document.querySelector('[data-provider-id="provider-c"] .anticon-right')`, 'virtual page');
  await evaluate("document.querySelector('[data-provider-id=\"provider-c\"] .anticon-right').closest('button').click(), true");
  await waitFor(row(live), 'virtual login');
  await click(live, 'sync');
  await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'virtual read');
  const reads = await evaluate("codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length");
  await evaluate('codexCardViewFixture.reloadAccounts(codexCardViewFixture.state.officialAccounts), true');
  await waitFor(`codexCardViewFixture.state.requests.filter(r => r.command === 'list_codex_official_accounts').length > ${reads}`, 'virtual reload');
  await respond({ credits: { hasCredits: true, balance: 15.5 } }, undefined, live);
  check('virtual successful usage survives an intervening same-login list reload', await evaluate(`${row(live)}.textContent.includes('15.5 积分')`));
  // The real backend does not persist virtual usage: the next list omits it.
  await evaluate('codexCardViewFixture.deferNextAccountList(), codexCardViewFixture.reloadAccounts(codexCardViewFixture.state.officialAccounts.map(a => a.isVirtual ? {...a, credits: undefined, lastLimitsFetchedAt: undefined} : a)), true');
  await waitFor('codexCardViewFixture.pendingAccountListCount() === 1', 'virtual list snapshot pending');
  await waitFor(listLoading, 'virtual list is visibly loading');
  await evaluate('codexCardViewFixture.respondAccountList(), true');
  await waitFor(`!(${listLoading})`, 'virtual list finished applying');
  check('virtual cached usage survives a later same-login list reload', await evaluate(`${row(live)}.textContent.includes('15.5 积分')`));
  await click(live, 'sync');
  await waitFor('codexCardViewFixture.pendingUsageCount() === 1', 'previous virtual login read');
  await evaluate("codexCardViewFixture.reloadAccounts(codexCardViewFixture.state.officialAccounts.map(a => a.isVirtual ? {...a, email: 'new-live@example.invalid', accountId: 'new-live', credits: undefined} : a)), true");
  await waitFor(row('new-live@example.invalid'), 'changed virtual login');
  await respond({ credits: { hasCredits: true, balance: 999 } }, undefined, live);
  check('reused __local__ id cannot carry credits across runtime logins', await evaluate(`${row('new-live@example.invalid')}.textContent.includes('999') || ${row('new-live@example.invalid')}.textContent.includes('15.5')`), false);
  return checks;
}
