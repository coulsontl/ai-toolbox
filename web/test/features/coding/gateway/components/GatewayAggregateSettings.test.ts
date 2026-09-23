import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const readSource = (relativePath: string) =>
  readFileSync(new URL(relativePath, import.meta.url), 'utf8');

const settingsSource = readSource(
  '../../../../../features/coding/gateway/components/GatewayAggregateSettings.tsx',
);
const settingsStyles = readSource(
  '../../../../../features/coding/gateway/components/GatewayAggregateSettings.module.less',
);

/**
 * Site priority has exactly one source: the CLI provider list order (`sort_index`,
 * the order the user drags in the provider list). The aggregate panel used to keep
 * a second, panel-local order with drag handles and up/down buttons, which made the
 * two lists disagree. The panel now only renders and writes the provider order.
 */
test('the aggregate panel never owns a local site order', () => {
  assert.match(
    settingsSource,
    /const orderedSiteIds = orderAggregateSiteIdsByCandidates\(nextSiteIds, candidates\);/,
  );

  // No panel-local reordering path may come back.
  assert.doesNotMatch(settingsSource, /@dnd-kit/);
  assert.doesNotMatch(settingsSource, /DndContext|SortableContext|useSortable/);
  assert.doesNotMatch(settingsSource, /moveAggregateSite/);
  assert.doesNotMatch(settingsStyles, /\.dragHandle \{/);
  assert.doesNotMatch(settingsStyles, /\.siteActions \{/);

  // The panel explains where the order actually comes from.
  assert.match(settingsSource, /t\('gateway\.aggregate\.orderHint'\)/);
});

/**
 * A site id is the provider's 32-character UUID primary key, which is unreadable
 * in the dense list. The rows show an eight-character display handle and keep the
 * full id in the tooltip; everything written to the backend keeps the full id.
 */
test('site rows show a short display handle instead of the raw provider id', () => {
  assert.match(settingsSource, /import \{[^}]*shortAggregateSiteId[^}]*\}/s);
  assert.match(
    settingsSource,
    /<code className=\{styles\.siteSlug\} title=\{candidate\.id\}>\s*\{shortAggregateSiteId\(candidate\.id\)\}/,
  );
  assert.doesNotMatch(settingsSource, /\{candidate\.id\}\s*<\/code>/);
});

/**
 * The backend refuses to change `primary_provider_id` while a manifest is
 * enabled, and aggregate names the first selected site as the primary. The panel
 * therefore has to restore direct mode *before* engaging whenever the first site
 * would change; otherwise the user sees the raw English guard sentence again.
 */
test('an engage that switches the primary restores direct first', () => {
  assert.match(settingsSource, /aggregateEngageRequiresDirectRestore/);
  assert.match(
    settingsSource,
    /if \(requiresDirectRestore\) \{[\s\S]*?await restoreProxyGatewayCliDirect\(cliKey\);[\s\S]*?\n\s*\}\n\s*return engageProxyGatewayAggregate\(/,
  );
  // The block case must be refused with the localized hint, never by writing a
  // direct config the CLI cannot use.
  assert.match(settingsSource, /setNotice\(\{ kind: 'error', text: restoreDirectBlockedHint \}\)/);
  assert.match(settingsSource, /aggregateEngageErrorNoticeKey/);
});

/**
 * Only the five lowest-priority visible catalog entries reach Codex's
 * `spawn_agent` model hint, so the panel has to refuse a sixth tick instead of
 * accepting one the subagent will never see. The refusal must be an explicit
 * notice plus a disabled checkbox, and the block must tell the user which of the
 * ticked names are inside the hint window.
 */
test('the exposure block refuses more ticks than the hint window holds', () => {
  assert.match(
    settingsSource,
    /if \(checked && effectiveSubagentExposedModels\.length >= SUBAGENT_EXPOSED_MODEL_LIMIT\) \{/,
  );
  assert.match(settingsSource, /subagentExposedLimit/);
  assert.match(
    settingsSource,
    /disabled=\{\s*busy \|\|\s*\(!ticked &&\s*effectiveSubagentExposedModels\.length >=\s*SUBAGENT_EXPOSED_MODEL_LIMIT\)/s,
  );
});

/**
 * The two states a subagent can see must be labelled per row, because the Codex
 * side shows no difference between a promoted name and an addressable-only one.
 */
test('every exposure row is labelled with what the subagent can actually pick', () => {
  assert.match(settingsSource, /subagentExposedBadgePromoted/);
  assert.match(settingsSource, /subagentExposedBadgeAddressable/);
  assert.match(settingsSource, /subagentExposedOrderHint/);
});

/**
 * A tick that the current site selection no longer declares cannot be
 * published: it must remain visible so the user can explicitly deselect it,
 * and both draft save and engage must refuse while it remains selected.
 */
test('stale exposure ticks require explicit deselection before save or engage', () => {
  assert.match(settingsSource, /staleSubagentExposedModels/);
  assert.match(settingsSource, /subagentExposedStale/);
  assert.match(settingsSource, /staleSubagentExposedModels\.length === 0/);
  assert.match(
    settingsSource,
    /const staleEntries = staleSubagentExposedModels[\s\S]*?allCandidates = \[\.\.\.subagentExposureCandidates, \.\.\.staleEntries\]/,
  );
  assert.match(settingsSource, /if \(nextStaleExposedModels\.length > 0\) \{[\s\S]*?return false;/);
  assert.match(settingsSource, /if \(staleExposedModels\.length > 0\) \{[\s\S]*?return;/);
  assert.match(
    settingsSource,
    /onChange=\{\(event\) =>\s*handleToggleExposedModel\(entry\.model, event\.currentTarget\.checked\)/,
  );
});

/**
 * The selection has to be made *before* the takeover engages, so the block may
 * not hide itself behind an engaged-only branch or skip loading its candidates.
 */
test('the exposure block is usable before the takeover engages', () => {
  assert.doesNotMatch(settingsSource, /subagentExposedRequiresEngaged/);
  assert.doesNotMatch(settingsSource, /if \(!engaged\) \{\s*return;\s*\}/);
});
