import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MINI_BROWSER_ACCOUNTS_STORAGE_KEY,
  MINI_BROWSER_MAX_URL_LENGTH,
  MINI_BROWSER_PREFS_STORAGE_KEY,
  MINI_BROWSER_SITES_STORAGE_KEY,
  dropMiniBrowserProfiles,
  ensureMiniBrowserAccounts,
  nextMiniBrowserAccountLabel,
  normaliseMiniBrowserUrl,
  parseMiniBrowserPrefs,
  stableMiniBrowserProfileId,
  toMiniBrowserProfileId,
  type MiniBrowserAccount,
  type MiniBrowserPrefs,
  type MiniBrowserSite,
} from '../../services/miniBrowserApi';

// ---- address normalisation -------------------------------------------------

test('mini browser adds https to a bare relay host', () => {
  assert.equal(normaliseMiniBrowserUrl('relay.example.com'), 'https://relay.example.com/');
  assert.equal(
    normaliseMiniBrowserUrl('  relay.example.com/console  '),
    'https://relay.example.com/console',
  );
  // A local dashboard keeps its explicit scheme and port.
  assert.equal(
    normaliseMiniBrowserUrl('http://127.0.0.1:3000/balance'),
    'http://127.0.0.1:3000/balance',
  );
});

test('mini browser keeps an explicit scheme and query', () => {
  assert.equal(
    normaliseMiniBrowserUrl('https://api.example.com/usage?tab=credits'),
    'https://api.example.com/usage?tab=credits',
  );
});

test('mini browser refuses schemes that would escape the web sandbox', () => {
  // These are the strings an <iframe> or a naive shell-open would execute.
  assert.equal(normaliseMiniBrowserUrl('javascript:alert(1)'), null);
  assert.equal(normaliseMiniBrowserUrl('file:///C:/Windows/win.ini'), null);
  assert.equal(normaliseMiniBrowserUrl('data:text/html,<h1>x</h1>'), null);
  assert.equal(normaliseMiniBrowserUrl('ms-settings:privacy'), null);
});

test('mini browser refuses empty, overlong and control-character input', () => {
  assert.equal(normaliseMiniBrowserUrl(''), null);
  assert.equal(normaliseMiniBrowserUrl('   '), null);
  assert.equal(normaliseMiniBrowserUrl('https://ok.example.com/\nheader'), null);
  assert.equal(
    normaliseMiniBrowserUrl(`https://example.com/${'a'.repeat(MINI_BROWSER_MAX_URL_LENGTH)}`),
    null,
  );
});

// ---- account persistence and legacy-site migration --------------------------

const site = (id: string, name = id): MiniBrowserSite => ({
  id,
  name,
  url: `https://${id}.example.com/`,
});

test('mini browser profile ids only keep characters the backend accepts', () => {
  // The backend rejects anything outside [a-z0-9-] to block path traversal.
  assert.equal(toMiniBrowserProfileId('Site-1'), 'site-1');
  assert.equal(toMiniBrowserProfileId('a/../b'), 'ab');
  assert.equal(toMiniBrowserProfileId('..'), null);
  assert.equal(toMiniBrowserProfileId('站点'), null);
  assert.equal(toMiniBrowserProfileId(''), null);
  assert.equal(toMiniBrowserProfileId('a'.repeat(100))?.length, 64);
});

test('mini browser maps an unusable site id to a stable, reused profile id', () => {
  // A site id from an older version may contain characters the backend rejects;
  // the derived profile must still be identical on every reload, otherwise the
  // account would lose its login state each time the app restarts.
  const first = stableMiniBrowserProfileId('legacy/../site id');
  const second = stableMiniBrowserProfileId('legacy/../site id');
  assert.equal(first, second);
  assert.notEqual(first, stableMiniBrowserProfileId('legacy/../site id2'));
  assert.equal(toMiniBrowserProfileId(first), first);
  // A modern UUID site id needs no transformation, which keeps its profile
  // identical to the one already in use.
  const uuid = '7f3e1c2a-4b5d-4e6f-8a9b-0c1d2e3f4a5b';
  assert.equal(stableMiniBrowserProfileId(uuid), uuid);
});

test('mini browser gives a legacy saved site one default account', () => {
  const sites = [site('relay-a'), site('relay-b')];
  const migrated = ensureMiniBrowserAccounts(sites, []);
  assert.equal(migrated.length, 2);
  assert.deepEqual(
    migrated.map((account) => account.id),
    ['relay-a', 'relay-b'],
  );
  // The account id doubles as the backend profile, so the existing site id is
  // reused verbatim: that site keeps the login it already had.
  assert.deepEqual(
    migrated.map((account) => account.siteId),
    ['relay-a', 'relay-b'],
  );
});

test('mini browser keeps multiple accounts per site and drops orphans', () => {
  const sites = [site('relay-a', 'Relay A')];
  const accounts: MiniBrowserAccount[] = [
    { id: 'relay-a', siteId: 'relay-a', label: 'Relay A-1' },
    { id: 'account-two', siteId: 'relay-a', label: 'Relay A-2' },
    { id: 'orphan', siteId: 'deleted-site', label: 'gone' },
  ];
  const reconciled = ensureMiniBrowserAccounts(sites, accounts);
  // No account is invented for a site that already has one.
  assert.deepEqual(
    reconciled.map((account) => account.id),
    ['relay-a', 'account-two'],
  );
});

test('mini browser numbers new accounts with the lowest free index', () => {
  const relay = site('relay-a', 'Relay A');
  assert.equal(nextMiniBrowserAccountLabel(relay, []), 'Relay A-1');
  const one: MiniBrowserAccount[] = [{ id: 'a', siteId: 'relay-a', label: 'Relay A-1' }];
  assert.equal(nextMiniBrowserAccountLabel(relay, one), 'Relay A-2');
  // A deleted account frees its number again; the profile id is what actually
  // identifies the account, so the label may be reused.
  const withGap: MiniBrowserAccount[] = [{ id: 'c', siteId: 'relay-a', label: 'Relay A-3' }];
  assert.equal(nextMiniBrowserAccountLabel(relay, withGap), 'Relay A-1');
  // Labels of a different site never interfere.
  const other: MiniBrowserAccount[] = [{ id: 'x', siteId: 'relay-b', label: 'Relay A-2' }];
  assert.equal(nextMiniBrowserAccountLabel(relay, other), 'Relay A-1');
});

test('mini browser storage keys stay stable for existing installs', () => {
  assert.equal(MINI_BROWSER_SITES_STORAGE_KEY, 'ai-router.mini-browser.sites');
  assert.equal(MINI_BROWSER_ACCOUNTS_STORAGE_KEY, 'ai-router.mini-browser.accounts');
});

// ---- remembered session ----------------------------------------------------

test('mini browser prefs decode a stored session', () => {
  assert.deepEqual(
    parseMiniBrowserPrefs({
      openTabs: ['relay-a', 'relay-a', 'relay-b'],
      activeProfile: 'relay-b',
      lastUrl: {
        'relay-a': 'https://relay.example.com/console',
        'relay-b': 'relay.example.com/usage',
      },
    }),
    {
      openTabs: ['relay-a', 'relay-b'],
      activeProfile: 'relay-b',
      lastUrl: {
        'relay-a': 'https://relay.example.com/console',
        'relay-b': 'https://relay.example.com/usage',
      },
    },
  );
});

test('mini browser prefs drop an active tab that is not in the strip', () => {
  // Otherwise the panel would highlight nothing while claiming a tab is active.
  assert.deepEqual(
    parseMiniBrowserPrefs({ openTabs: ['relay-a'], activeProfile: 'relay-b' }).activeProfile,
    null,
  );
  assert.deepEqual(parseMiniBrowserPrefs({ openTabs: [], activeProfile: 'relay-a' }).activeProfile, null);
});

test('mini browser prefs survive junk without inventing a tab', () => {
  const empty = { openTabs: [], activeProfile: null, lastUrl: {} };
  assert.deepEqual(parseMiniBrowserPrefs(null), empty);
  assert.deepEqual(parseMiniBrowserPrefs([]), empty);
  assert.deepEqual(parseMiniBrowserPrefs('nonsense'), empty);
  assert.deepEqual(
    parseMiniBrowserPrefs({ openTabs: ['ok', 7, null], activeProfile: 5, lastUrl: 'no' }),
    { openTabs: ['ok'], activeProfile: null, lastUrl: {} },
  );
});

test('mini browser prefs refuse an address the backend would reject', () => {
  // A remembered `javascript:` entry would be refused at open time and leave the
  // tab apparently broken; dropping it makes the tab fall back to the site.
  assert.deepEqual(
    parseMiniBrowserPrefs({
      openTabs: ['relay-a'],
      activeProfile: 'relay-a',
      lastUrl: { 'relay-a': 'javascript:alert(1)' },
    }).lastUrl,
    {},
  );
  assert.deepEqual(
    parseMiniBrowserPrefs({
      openTabs: ['relay-a'],
      activeProfile: 'relay-a',
      lastUrl: { 'relay-a': 'file:///c:/secret.txt' },
    }).lastUrl,
    {},
  );
});

test('mini browser prefs keep their own storage key', () => {
  assert.equal(MINI_BROWSER_PREFS_STORAGE_KEY, 'ai-router.mini-browser.prefs');
});

// ---- dropping profiles from the session --------------------------------------

const session = (): MiniBrowserPrefs => ({
  openTabs: ['tab-a', 'tab-b', 'tab-c'],
  activeProfile: 'tab-b',
  lastUrl: {
    'tab-a': 'https://relay.example.com/home',
    'tab-b': 'https://relay.example.com/usage',
    'tab-c': 'https://other.example.com/model-plaza',
  },
});

test('mini browser closing a tab keeps its remembered page', () => {
  // The address is a habit of the account, not a property of the tab: closing a
  // tab and reopening that account later has to resume on the same page.
  const next = dropMiniBrowserProfiles(session(), ['tab-a'], 'closed');
  assert.deepEqual(next.openTabs, ['tab-b', 'tab-c']);
  assert.equal(next.lastUrl['tab-a'], 'https://relay.example.com/home');
  assert.deepEqual(next.lastUrl, session().lastUrl);
});

test('mini browser close-all keeps every remembered page', () => {
  // The regression this exists for: "close all" reused the delete-account rule,
  // so every address was dropped and each tab reopened at its site root. A tab
  // whose site does not redirect then looked like it had never remembered
  // anything, while its neighbours only "remembered" because the site bounced
  // them to a deep address that was recorded again.
  const source = session();
  const next = dropMiniBrowserProfiles(source, [...source.openTabs], 'closed');
  assert.deepEqual(next.openTabs, []);
  assert.equal(next.activeProfile, null);
  assert.deepEqual(next.lastUrl, source.lastUrl);
});

test('mini browser deleting an account forgets its remembered page', () => {
  // A deleted account must not leave an address behind: the profile id could be
  // handed to a new account, which would then resume a stranger's page.
  const next = dropMiniBrowserProfiles(session(), ['tab-a', 'tab-c'], 'deleted');
  assert.deepEqual(next.openTabs, ['tab-b']);
  assert.deepEqual(next.lastUrl, { 'tab-b': 'https://relay.example.com/usage' });
});

test('mini browser moves the front tab when the front one is dropped', () => {
  // Dropping the tab that was in front promotes the last remaining one…
  const other = dropMiniBrowserProfiles(
    { ...session(), activeProfile: 'tab-c' },
    ['tab-c'],
    'closed',
  );
  assert.equal(other.activeProfile, 'tab-b');
  // …and dropping a background tab leaves the front one alone.
  assert.equal(dropMiniBrowserProfiles(session(), ['tab-a'], 'closed').activeProfile, 'tab-b');
});

test('mini browser close-all clears the front tab without touching addresses', () => {
  const source = session();
  const next = dropMiniBrowserProfiles(source, [...source.openTabs], 'closed');
  assert.equal(next.activeProfile, null);
  assert.deepEqual(next.lastUrl, source.lastUrl);
});

test('mini browser dropping nothing is a no-op', () => {
  const source = session();
  assert.equal(dropMiniBrowserProfiles(source, [], 'closed'), source);
});
