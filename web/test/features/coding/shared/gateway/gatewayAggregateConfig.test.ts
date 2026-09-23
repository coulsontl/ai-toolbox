import assert from 'node:assert/strict';
import test from 'node:test';

import type { GatewayCliTakeoverStatus } from '../../../../../services/proxyGatewayApi.ts';
import {
  buildGatewayAggregateModelSlug,
  buildGatewayAggregateSitePreviewSlug,
  canReplaySubagentExposureSelection,
  deriveGatewayAggregateSitePrefix,
  isAggregateSiteId,
  normalizeGatewayAggregateAliases,
  normalizeGatewayAggregateSiteIds,
  normalizeSubagentExposedModels,
  resolveEffectiveSubagentExposedModels,
  resolveCanonicalSubagentExposedModels,
  resolveStaleSubagentExposedModels,
  resolveGatewayAggregateEffectiveAliases,
  resolveGatewayReengageMode,
  resolveSubagentExposureCandidates,
  isSubagentExposureSelectionComplete,
  SUBAGENT_EXPOSED_MODEL_LIMIT,
  toGatewayAggregateReengageConfig,
  validateGatewayAggregateAlias,
  validateGatewayAggregateSeparator,
} from '../../../../../features/coding/shared/gateway/gatewayAggregateConfig.ts';
import {
  isGatewayProxyMode,
  isGatewayAggregateMode,
  isGatewayFailoverMode,
} from '../../../../../features/coding/shared/gateway/providerProtocol.ts';

const status = (
  partial: Partial<GatewayCliTakeoverStatus> & Pick<GatewayCliTakeoverStatus, 'mode'>,
): GatewayCliTakeoverStatus => ({
  cli_key: 'codex',
  state: 'takeover_applied',
  dot: 'green',
  can_takeover: true,
  can_restore_direct: true,
  gateway_origin: 'http://127.0.0.1:37124',
  runtime_root: null,
  managed_targets: [],
  primary_provider_id: null,
  provider_priorities: [],
  message: null,
  ...partial,
});

// ---- mode guards -----------------------------------------------------------

test('mode guards keep aggregate distinct from failover', () => {
  assert.equal(isGatewayProxyMode('single'), true);
  assert.equal(isGatewayProxyMode('failover'), true);
  assert.equal(isGatewayProxyMode('aggregate'), true);
  assert.equal(isGatewayProxyMode(null), false);
  assert.equal(isGatewayProxyMode(undefined), false);

  // Aggregate must never be reported as failover: the failover UI pins a
  // primary provider and shows P0/P1 priorities, neither of which exists here.
  assert.equal(isGatewayFailoverMode('aggregate'), false);
  assert.equal(isGatewayFailoverMode('failover'), true);
  assert.equal(isGatewayAggregateMode('failover'), false);
  assert.equal(isGatewayAggregateMode('aggregate'), true);
});

// ---- separator validation --------------------------------------------------

test('separator must be non-empty and free of site-id characters', () => {
  assert.equal(validateGatewayAggregateSeparator('.'), null);
  assert.equal(validateGatewayAggregateSeparator('::'), null);
  assert.equal(validateGatewayAggregateSeparator('/'), null);
  assert.equal(validateGatewayAggregateSeparator('.'), null);

  assert.equal(validateGatewayAggregateSeparator(''), 'empty');
  assert.equal(validateGatewayAggregateSeparator('a'), 'reservedCharacters');
  assert.equal(validateGatewayAggregateSeparator('9'), 'reservedCharacters');
  assert.equal(validateGatewayAggregateSeparator('_'), 'reservedCharacters');
  assert.equal(validateGatewayAggregateSeparator('-'), 'reservedCharacters');
  // One bad character inside an otherwise fine separator is still rejected.
  assert.equal(validateGatewayAggregateSeparator('.-'), 'reservedCharacters');
});

// ---- site ids --------------------------------------------------------------

test('site ids follow the backend addressable charset', () => {
  assert.equal(isAggregateSiteId('76a6ef74'), true);
  assert.equal(isAggregateSiteId('site-1'), true);
  assert.equal(isAggregateSiteId('site_1'), true);
  assert.equal(isAggregateSiteId(''), false);
  assert.equal(isAggregateSiteId('site.1'), false);
  assert.equal(isAggregateSiteId('site:1'), false);
  assert.equal(isAggregateSiteId('site 1'), false);
});

test('normalize keeps order, drops duplicates and unusable ids', () => {
  assert.deepEqual(
    normalizeGatewayAggregateSiteIds(['a', 'b', 'a', ' site:1 ', 'c', '']),
    ['a', 'b', 'c'],
  );
  assert.deepEqual(normalizeGatewayAggregateSiteIds([]), []);
});

// ---- re-engage resolution --------------------------------------------------

test('re-engage mode only accepts aggregate when its selection is complete', () => {
  assert.equal(resolveGatewayReengageMode(status({ mode: 'single' })), 'single');
  assert.equal(resolveGatewayReengageMode(status({ mode: 'failover' })), 'failover');
  assert.equal(resolveGatewayReengageMode(status({ mode: null })), null);
  assert.equal(resolveGatewayReengageMode(null), null);
  assert.equal(resolveGatewayReengageMode(undefined), null);

  // Aggregate without manifest details must not re-engage: engaging with an
  // empty site list would silently drop the whole cross-site model list.
  assert.equal(resolveGatewayReengageMode(status({ mode: 'aggregate' })), null);
  assert.equal(
    resolveGatewayReengageMode(
      status({ mode: 'aggregate', aggregate: { provider_ids: [], separator: '.' } }),
    ),
    null,
  );
  assert.equal(
    resolveGatewayReengageMode(
      status({ mode: 'aggregate', aggregate: { provider_ids: ['site1'], separator: '-' } }),
    ),
    null,
  );
  assert.equal(
    resolveGatewayReengageMode(
      status({ mode: 'aggregate', aggregate: { provider_ids: ['site1'], separator: '.' } }),
    ),
    'aggregate',
  );
});

test('aggregate reengage config is only built for a valid aggregate manifest', () => {
  assert.equal(toGatewayAggregateReengageConfig(status({ mode: 'single' })), null);
  // An invalid separator cannot be replayed: the backend would reject it.
  assert.equal(
    toGatewayAggregateReengageConfig(
      status({ mode: 'aggregate', aggregate: { provider_ids: ['b', 'a'], separator: '-' } }),
    ),
    null,
  );
  assert.deepEqual(
    toGatewayAggregateReengageConfig(
      status({
        mode: 'aggregate',
        aggregate: { provider_ids: ['b', 'a', 'b'], separator: '::' },
      }),
    ),
    {
      providerIds: ['b', 'a'],
      separator: '::',
      aliases: {},
      naming: 'site_model',
      crossSiteFailover: false,
    },
  );
});

test('aggregate slug joins site id and model with the configured separator', () => {
  assert.equal(buildGatewayAggregateModelSlug('76a6ef74', 'deepseek-v4-flash', '.'), '76a6ef74.deepseek-v4-flash');
  assert.equal(buildGatewayAggregateModelSlug('site-1', 'glm-5.3', '::'), 'site-1::glm-5.3');
});

test('aggregate preview uses the effective alias and naming template', () => {
  assert.equal(
    buildGatewayAggregateSitePreviewSlug('site-a', '.', 'site_model', { 'site-a': 'relay' }),
    'relay.<model>',
  );
  assert.equal(
    buildGatewayAggregateSitePreviewSlug('site-a', '@', 'model_at_site', { 'site-a': 'relay' }),
    '<model>@relay',
  );
  assert.equal(
    buildGatewayAggregateSitePreviewSlug('site-a', '.', 'model_only', { 'site-a': 'relay' }),
    '<model>',
  );
});

test('aliases may not shadow any candidate site id', () => {
  // Another *selected* site's id: the auto-derived prefix already collides.
  assert.equal(
    normalizeGatewayAggregateAliases({ 'site-a': 'site-b' }, ['site-a', 'site-b']),
    null,
  );
  // An *unselected* candidate keeps answering to its provider id at request
  // time, so the backend refuses this prefix; the form must not submit it.
  assert.equal(
    normalizeGatewayAggregateAliases({ 'site-a': 'site-c' }, ['site-a'], ['site-a', 'site-c']),
    null,
  );
  // Prefix matching is case-insensitive, so the shadow check must be too.
  assert.equal(
    normalizeGatewayAggregateAliases({ 'site-a': 'SITE-C' }, ['site-a'], ['site-a', 'site-c']),
    null,
  );
  // A real alias, and an alias equal to its own site id, both stay usable.
  assert.deepEqual(
    normalizeGatewayAggregateAliases({ 'site-a': 'relay' }, ['site-a'], ['site-a', 'site-c']),
    { 'site-a': 'relay' },
  );
  assert.deepEqual(
    normalizeGatewayAggregateAliases({ 'site-a': 'site-a' }, ['site-a'], ['site-a', 'site-c']),
    { 'site-a': 'site-a' },
  );
  // Only *effective* prefixes must be unique: site-b answers to its own alias,
  // so site-a may take over the raw id `site-b` (the backend accepts this too).
  assert.deepEqual(
    normalizeGatewayAggregateAliases(
      { 'site-a': 'site-b', 'site-b': 'relay-b' },
      ['site-a', 'site-b'],
    ),
    { 'site-a': 'site-b', 'site-b': 'relay-b' },
  );
  // Blank aliases are dropped rather than treated as an alias.
  assert.deepEqual(
    normalizeGatewayAggregateAliases({ 'site-a': '  ' }, ['site-a'], ['site-a']),
    {},
  );
});

// ---- site-name prefixes and bare-name aliases ------------------------------

test('explicit aliases allow CJK but reject structural characters', () => {
  assert.equal(validateGatewayAggregateAlias('思源888', '.'), true);
  assert.equal(validateGatewayAggregateAlias('中文', '.'), true);
  // A space is the separator between a display name and a model, so an
  // explicit alias can never contain one.
  assert.equal(validateGatewayAggregateAlias('思源888 pro', '.'), false);
  assert.equal(validateGatewayAggregateAlias('with.dot', '.'), false);
  assert.equal(validateGatewayAggregateAlias('with/slash', '.'), true);
  assert.equal(validateGatewayAggregateAlias('with/slash', '/'), false);
  assert.equal(validateGatewayAggregateAlias('', '.'), false);
  assert.equal(validateGatewayAggregateAlias('a'.repeat(33), '.'), false);
});

test('derived site prefixes normalise a display name into a usable token', () => {
  assert.equal(deriveGatewayAggregateSitePrefix('思源888 pro', '.'), '思源888-pro');
  assert.equal(deriveGatewayAggregateSitePrefix('ai.nexfaro.com', '.'), 'ai-nexfaro-com');
  assert.equal(deriveGatewayAggregateSitePrefix('  a\t\tb  ', '.'), 'a-b');
  assert.equal(deriveGatewayAggregateSitePrefix('   ', '.'), null);
});

test('effective aliases default to the site name but never steal an address', () => {
  const selected = [
    { id: '76a6ef74af6c4151812787cc519b534b', name: '思源888 pro' },
    { id: 'ccex', name: 'Unsee Relay' },
  ];
  const allIds = ['76a6ef74af6c4151812787cc519b534b', 'ccex'];

  assert.deepEqual(resolveGatewayAggregateEffectiveAliases({}, selected, allIds, '.'), {
    '76a6ef74af6c4151812787cc519b534b': '思源888-pro',
    ccex: 'Unsee-Relay',
  });

  // An explicit alias always wins over the derived display name.
  assert.deepEqual(
    resolveGatewayAggregateEffectiveAliases({ ccex: 'relay' }, selected, allIds, '.'),
    { '76a6ef74af6c4151812787cc519b534b': '思源888-pro', ccex: 'relay' },
  );

  // A derived name that collides with another address keeps the provider id
  // instead of shadowing it, so engaging never fails on a duplicate name.
  assert.deepEqual(
    resolveGatewayAggregateEffectiveAliases(
      {},
      [
        { id: 'a', name: 'b' },
        { id: 'b', name: 'Bee' },
      ],
      ['a', 'b'],
      '.',
    ),
    { b: 'Bee' },
  );

  // Two sites sharing a display name: the first keeps it, the second falls
  // back to its provider id (no silent overwrite).
  assert.deepEqual(
    resolveGatewayAggregateEffectiveAliases(
      {},
      [
        { id: 'a', name: 'Relay' },
        { id: 'b', name: 'Relay' },
      ],
      ['a', 'b'],
      '.',
    ),
    { a: 'Relay' },
  );
});

test('effective alias map round-trips a saved aggregate manifest', () => {
  // A manifest saved before names were derived stores only explicit aliases;
  // re-engaging must not lose them, and a CJK alias must survive the trip.
  const manifest = {
    provider_ids: ['76a6ef74', 'ccex'],
    separator: '.',
    aliases: { '76a6ef74': '思源888' },
  };
  assert.deepEqual(
    toGatewayAggregateReengageConfig(
      status({ mode: 'aggregate', aggregate: manifest }),
    ),
    {
      providerIds: ['76a6ef74', 'ccex'],
      separator: '.',
      aliases: { '76a6ef74': '思源888' },
      naming: 'site_model',
      // The failover policy is replayed explicitly: a manifest that predates
      // the field means `false`, and dropping the key would let a provider save
      // flip the safety switch back on.
      crossSiteFailover: false,
    },
  );
});

// ---- managed [agents] subagent defaults ------------------------------------

test('managed subagent defaults survive the re-engage config round trip', () => {
  // Restoring direct mode drops the keys, so a provider save must replay them
  // or the user's subagent default would be silently unset.
  const config = toGatewayAggregateReengageConfig(
    status({
      mode: 'aggregate',
      aggregate: {
        provider_ids: ['site1'],
        separator: '.',
        subagent: { model: 'gpt-5.6-luna', reasoning_effort: 'xhigh' },
      },
    }),
  );
  assert.deepEqual(config, {
    providerIds: ['site1'],
    separator: '.',
    aliases: {},
    naming: 'site_model',
    subagentModel: 'gpt-5.6-luna',
    subagentReasoningEffort: 'xhigh',
    crossSiteFailover: false,
  });
});

// ---- cross-site failover ---------------------------------------------------

test('an enabled cross-site failover survives the re-engage round trip', () => {
  // Restoring direct mode drops the routing policy, so a provider save must
  // replay it; a `false` that is dropped would silently re-enable cross-site
  // spending, and a `true` that is dropped would break failover.
  const config = toGatewayAggregateReengageConfig(
    status({
      mode: 'aggregate',
      aggregate: {
        provider_ids: ['site1'],
        separator: '.',
        cross_site_failover: true,
      },
    }),
  );
  assert.deepEqual(config, {
    providerIds: ['site1'],
    separator: '.',
    aliases: {},
    naming: 'site_model',
    crossSiteFailover: true,
  });
});

test('a manifest without the failover field replays the safe default', () => {
  // Manifests written before the field existed mean `false`; the re-engage has
  // to say so explicitly instead of omitting the key.
  const config = toGatewayAggregateReengageConfig(
    status({ mode: 'aggregate', aggregate: { provider_ids: ['site1'], separator: '.' } }),
  );
  assert.equal(config?.crossSiteFailover, false);
});

test('a takeover that manages no [agents] keys stays that way', () => {
  // No `subagent` block → the form must not claim ownership of any key, so the
  // re-engage config carries neither field and the user's own [agents] settings
  // are never written.
  const config = toGatewayAggregateReengageConfig(
    status({ mode: 'aggregate', aggregate: { provider_ids: ['site1'], separator: '.' } }),
  );
  assert.equal(config && 'subagentModel' in config, false);
  assert.equal(config && 'subagentReasoningEffort' in config, false);

  // Blank values inside the block are equally "unmanaged".
  const blank = toGatewayAggregateReengageConfig(
    status({
      mode: 'aggregate',
      aggregate: {
        provider_ids: ['site1'],
        separator: '.',
        subagent: { model: '   ', reasoning_effort: '' },
      },
    }),
  );
  assert.equal(blank && 'subagentModel' in blank, false);
  assert.equal(blank && 'subagentReasoningEffort' in blank, false);
});

// ---- programmable bare-name exposure ---------------------------------------

test('a narrowed exposure set survives the re-engage config round trip', () => {
  // "Expose only these" and "expose everything" are both valid states of the
  // same field ([] is the publish-all default), so a provider save that dropped
  // the narrowed set would silently widen the catalog again.
  const narrowed = toGatewayAggregateReengageConfig(
    status({
      mode: 'aggregate',
      aggregate: {
        provider_ids: ['site1'],
        separator: '.',
        subagent_exposed_models: [' gpt-5.6-luna ', 'gpt-5.6-luna', 'glm-5'],
      },
    }),
  );
  assert.deepEqual(narrowed && narrowed.subagentExposedModels, [
    'gpt-5.6-luna',
    'glm-5',
  ]);

  // Publish-all stays publish-all: the field is absent rather than an explicit
  // empty list, so callers keep the backend default.
  const all = toGatewayAggregateReengageConfig(
    status({
      mode: 'aggregate',
      aggregate: {
        provider_ids: ['site1'],
        separator: '.',
        subagent_exposed_models: [],
      },
    }),
  );
  assert.equal(all && 'subagentExposedModels' in all, false);
});

test('an engage needs between one and the hint limit of usable names', () => {
  // An empty list is the legacy "promote nothing" default, never a valid choice.
  assert.equal(isSubagentExposureSelectionComplete(undefined), false);
  assert.equal(isSubagentExposureSelectionComplete([]), false);
  assert.equal(isSubagentExposureSelectionComplete(['  ']), false);
  assert.equal(isSubagentExposureSelectionComplete(['gpt-5.6-luna']), true);
  // Exactly the limit is fine; one more would be silently invisible to the
  // subagent, so it is incomplete rather than quietly truncated.
  const atLimit = Array.from(
    { length: SUBAGENT_EXPOSED_MODEL_LIMIT },
    (_, index) => `model-${index}`,
  );
  assert.equal(isSubagentExposureSelectionComplete(atLimit), true);
  assert.equal(isSubagentExposureSelectionComplete([...atLimit, 'model-extra']), false);
  // Duplicates and blanks do not count toward the limit.
  assert.equal(
    isSubagentExposureSelectionComplete([...atLimit, ' model-0 ', '']),
    true,
  );
});

test('a stored exposure selection is only replayable within the hint limit', () => {
  const atLimit = Array.from(
    { length: SUBAGENT_EXPOSED_MODEL_LIMIT },
    (_, index) => `model-${index}`,
  );

  // An empty/absent selection is the legacy "promote nothing" default, which the
  // backend accepts, so it must stay replayable rather than be read as broken.
  assert.equal(canReplaySubagentExposureSelection(undefined), true);
  assert.equal(canReplaySubagentExposureSelection(null), true);
  assert.equal(canReplaySubagentExposureSelection([]), true);
  assert.equal(canReplaySubagentExposureSelection(atLimit), true);
  // Duplicates and blanks are normalized away before the limit is applied.
  assert.equal(
    canReplaySubagentExposureSelection([...atLimit, ' model-0 ', '']),
    true,
  );

  // One more than the hint window is exactly the selection every engage rejects,
  // which is why the provider-save round trip must detect it before restoring
  // direct mode.
  assert.equal(canReplaySubagentExposureSelection([...atLimit, 'model-extra']), false);
  assert.equal(
    canReplaySubagentExposureSelection([' m0 ', 'm1', 'm2', 'm3', 'm4', 'm5']),
    false,
  );
});

test('effective exposure keeps only addressable names while preserving tick order', () => {
  assert.deepEqual(
    resolveEffectiveSubagentExposedModels(
      [' stale ', 'gpt-5.6-luna', 'gpt-5.6-luna', 'gpt-5.6-sol'],
      ['gpt-5.6-luna', 'gpt-5.6-sol'],
    ),
    ['gpt-5.6-luna', 'gpt-5.6-sol'],
  );
  assert.deepEqual(
    resolveEffectiveSubagentExposedModels(['stale-only'], ['gpt-5.6-luna']),
    [],
  );
});

test('mixed exposure keeps stale selections visible for explicit deselection', () => {
  const selected = ['stale-model', 'model-1', 'model-2'];
  const addressable = ['model-1', 'model-2'];

  assert.deepEqual(
    resolveEffectiveSubagentExposedModels(selected, addressable),
    ['model-1', 'model-2'],
  );
  assert.deepEqual(
    resolveStaleSubagentExposedModels(selected, addressable),
    ['stale-model'],
  );
  assert.deepEqual(
    resolveStaleSubagentExposedModels(['stale-model'], addressable),
    ['stale-model'],
  );
});

test('successful API responses are the canonical exposure source', () => {
  assert.deepEqual(
    resolveCanonicalSubagentExposedModels({
      subagent_exposed_models: [' gpt-5.6-sol ', 'gpt-5.6-sol', ''],
    }),
    ['gpt-5.6-sol'],
  );
  assert.deepEqual(resolveCanonicalSubagentExposedModels(null), []);
});

test('exposure candidates prefer the universe so a removed name can return', () => {
  // The published `entries` only carry the names the catalog still hides. The
  // `bare_models` universe is what keeps a name that was narrowed away tickable
  // again — without it the user could only switch back to "expose all".
  const candidates = resolveSubagentExposureCandidates(
    {
      entries: [{ model: 'glm-5', provider_id: 'site1', provider_name: 'Site 1' }],
      bare_models: [
        { model: 'gpt-5.6-luna', provider_id: 'site1', provider_name: 'Site 1' },
        { model: 'glm-5', provider_id: 'site1', provider_name: 'Site 1' },
      ],
    },
    ['site1'],
  );
  assert.deepEqual(candidates.map((entry) => entry.model), ['gpt-5.6-luna', 'glm-5']);

  // Older payloads omit the universe: fall back to the published hidden
  // aliases, and drop entries belonging to sites this takeover no longer routes.
  const fallback = resolveSubagentExposureCandidates(
    {
      entries: [
        { model: 'gpt-5.6-luna', provider_id: 'site1', provider_name: 'Site 1' },
        { model: 'stale-model', provider_id: 'gone', provider_name: 'Gone' },
      ],
    },
    ['site1'],
  );
  assert.deepEqual(fallback.map((entry) => entry.model), ['gpt-5.6-luna']);

  assert.deepEqual(resolveSubagentExposureCandidates(null, ['site1']), []);
});

test('exposed model names are trimmed, de-duplicated and drop blanks', () => {
  assert.deepEqual(
    normalizeSubagentExposedModels([' gpt-5.6-luna ', '', 'gpt-5.6-luna', 'glm-5']),
    ['gpt-5.6-luna', 'glm-5'],
  );
  assert.deepEqual(normalizeSubagentExposedModels(null), []);
});
