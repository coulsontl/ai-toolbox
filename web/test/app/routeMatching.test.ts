import assert from 'node:assert/strict';
import test from 'node:test';
import type { ComponentType } from 'react';

import {
  getRouteChrome,
  getRouteScrollKey,
  matchRouteEntry,
  normalizeRouteChrome,
  resolveInitialTabPath,
  shouldShowRouteAppHeader,
} from '../../app/routeMatching.ts';
import type { RouteEntry } from '../../app/routeConfig.ts';

const TestComponent = (() => null) as ComponentType;

const RESTORE_ROUTES: RouteEntry[] = [
  { path: '/coding/opencode', component: TestComponent },
  { path: '/coding/codex', component: TestComponent },
  { path: '/coding/codex/sessions/detail', component: TestComponent, chrome: { mode: 'secondary' } },
  { path: '/settings', component: TestComponent },
];

const RESTORE_SUB_TABS = [
  { key: 'opencode', path: '/coding/opencode' },
  { key: 'codex', path: '/coding/codex' },
];

test('resolveInitialTabPath restores the saved tab on the /index.html cold boot', () => {
  assert.equal(
    resolveInitialTabPath('/index.html', RESTORE_ROUTES, RESTORE_SUB_TABS, 'codex'),
    '/coding/codex',
  );
});

test('resolveInitialTabPath restores the saved tab on the bare root path', () => {
  assert.equal(
    resolveInitialTabPath('/', RESTORE_ROUTES, RESTORE_SUB_TABS, 'codex'),
    '/coding/codex',
  );
});

test('resolveInitialTabPath ignores the saved tab once a route is matched', () => {
  // Hidden-tab redirects land on a real route pathname and must keep the
  // first-visible-tab fallback instead of bouncing back to the saved tab.
  assert.equal(
    resolveInitialTabPath('/coding/opencode', RESTORE_ROUTES, RESTORE_SUB_TABS, 'codex'),
    null,
  );
  assert.equal(
    resolveInitialTabPath('/coding/codex/sessions/detail', RESTORE_ROUTES, RESTORE_SUB_TABS, 'codex'),
    null,
  );
});

test('resolveInitialTabPath falls back when the saved tab is missing or hidden', () => {
  assert.equal(
    resolveInitialTabPath('/index.html', RESTORE_ROUTES, RESTORE_SUB_TABS, 'removed_tab'),
    null,
  );
  assert.equal(
    resolveInitialTabPath('/index.html', RESTORE_ROUTES, RESTORE_SUB_TABS, ''),
    null,
  );
  assert.equal(
    resolveInitialTabPath('/index.html', RESTORE_ROUTES, RESTORE_SUB_TABS, undefined),
    null,
  );
});

test('matchRouteEntry returns the longest route match', () => {
  const routes: RouteEntry[] = [
    { path: '/coding/opencode', component: TestComponent },
    {
      path: '/coding/opencode/sessions/detail',
      component: TestComponent,
      chrome: { mode: 'secondary', contentPadding: 'compact' },
    },
  ];

  const matched = matchRouteEntry(routes, '/coding/opencode/sessions/detail/extra');

  assert.equal(matched?.path, '/coding/opencode/sessions/detail');
});

test('matchRouteEntry does not match partial sibling prefixes', () => {
  const routes: RouteEntry[] = [
    { path: '/settings', component: TestComponent },
  ];

  assert.equal(matchRouteEntry(routes, '/settings-panel'), undefined);
});

test('plugins workspace sub-routes inherit the borderless content chrome', () => {
  const routes: RouteEntry[] = [
    {
      path: '/plugins',
      routePath: '/plugins/*',
      component: TestComponent,
      chrome: { contentPadding: 'none' },
    },
  ];

  // Every sidebar tab and custom plugin page lives below `/plugins`, so they
  // all have to pick up the same zero-padding chrome from the parent route.
  for (const pathname of [
    '/plugins',
    '/plugins/installed',
    '/plugins/accounts',
    '/plugins/market',
    '/plugins/runtime',
    '/plugins/page/demo-plugin/demo-page',
  ]) {
    assert.equal(
      getRouteChrome(matchRouteEntry(routes, pathname)).contentPadding,
      'none',
      pathname,
    );
  }
});

test('settings page fills the viewport below the app chrome', () => {
  // The split layout owns its own scrolling columns, so the shared content
  // area must not add the default 24px padding around it.
  const routes: RouteEntry[] = [
    { path: '/settings', component: TestComponent, chrome: { contentPadding: 'none' } },
  ];

  assert.equal(
    getRouteChrome(matchRouteEntry(routes, '/settings')).contentPadding,
    'none',
  );
});

test('route chrome defaults to the standard app chrome', () => {
  assert.deepEqual(getRouteChrome(undefined), {
    mode: 'default',
    contentPadding: 'default',
  });
});

test('route chrome preserves secondary page metadata', () => {
  const matched = normalizeRouteChrome({
    mode: 'secondary',
    contentPadding: 'compact',
    ownerTabKey: 'codex',
    parentPath: '/coding/codex',
  });

  assert.deepEqual(matched, {
    mode: 'secondary',
    contentPadding: 'compact',
    ownerTabKey: 'codex',
    parentPath: '/coding/codex',
  });
});

test('secondary route chrome hides the app header', () => {
  assert.equal(shouldShowRouteAppHeader(normalizeRouteChrome(undefined)), true);
  assert.equal(
    shouldShowRouteAppHeader(normalizeRouteChrome({ mode: 'secondary' })),
    false,
  );
});

test('route scroll key keeps tab pages stable and isolates secondary page queries', () => {
  const parentRoute: RouteEntry = {
    path: '/coding/codex',
    component: TestComponent,
  };
  const secondaryRoute: RouteEntry = {
    path: '/coding/codex/sessions/detail',
    component: TestComponent,
    chrome: { mode: 'secondary' },
  };

  assert.equal(
    getRouteScrollKey(parentRoute, '/coding/codex', '?panel=sessions'),
    '/coding/codex',
  );
  assert.equal(
    getRouteScrollKey(secondaryRoute, '/coding/codex/sessions/detail', '?sourcePath=a'),
    '/coding/codex/sessions/detail?sourcePath=a',
  );
  assert.equal(
    getRouteScrollKey(undefined, '/missing', '?q=1'),
    '/missing?q=1',
  );
});
