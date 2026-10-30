import assert from 'node:assert/strict';
import test from 'node:test';

import {
  calculateCacheHitRate,
  deriveGatewayRequestDisplay,
  gatewayUsageRangeKey,
  gatewayUsageRangeToStored,
  formatCacheHitRate,
  formatDuration,
  formatDurationPair,
  formatModelRoute,
  formatModelWithEffort,
  formatModelQualifiers,
  formatTps,
  formatUsd,
  gatewayTransportBadgeKey,
  nativeToolBadgeKey,
  nativeToolBadges,
  gatewayProjectName,
  compactionModeKey,
  gatewayRequestDisplayKind,
  gatewayWebSocketStatusKey,
  getGatewayRequestsPerMinute,
  isGatewayRequestUsageApplicable,
  normalizeAttemptCounts,
  requestExportPrefix,
  requestLineText,
  sessionDurationText,
  resolveGatewayRequestRange,
  resolveGatewayUsageRange,
  serviceTierLabel,
  shouldShowBodyComparison,
  unpricedModelIds,
} from '../../../../../features/coding/gateway/utils/gatewayFormatters.ts';

test('WebSocket request status uses the delivered terminal and keeps handshake fallback distinct', () => {
  for (const outcome of ['completed', 'failed', 'incomplete', 'canceled']) {
    assert.equal(gatewayWebSocketStatusKey({ transport: 'websocket', stream_outcome: outcome, status_code: 0, success: outcome === 'completed' }), `gateway.page.requests.websocket.${outcome}`);
  }
  assert.equal(gatewayWebSocketStatusKey({ transport: 'websocket', request_kind: 'websocket_handshake', status_code: 426, success: false }), 'gateway.page.requests.websocket.fallback');
  assert.equal(gatewayWebSocketStatusKey({ transport: 'websocket', request_kind: 'websocket_handshake', status_code: 401, success: false }), null);
  assert.equal(gatewayWebSocketStatusKey({ status_code: 200, success: true }), null);
  assert.equal(gatewayWebSocketStatusKey({ transport: 'websocket', status_code: null, success: false }), 'gateway.page.requests.websocket.failed');
});

test('WebSocket handshake and warmup do not pretend to be ordinary model usage', () => {
  const request = { method: 'WS', path: '/openai/v1/responses', requested_model: 'gpt-5', transport: 'websocket' as const };
  assert.equal(gatewayRequestDisplayKind(request), 'model');
  assert.equal(isGatewayRequestUsageApplicable(request), true);
  const handshake = { ...request, method: 'GET', request_kind: 'websocket_handshake' as const };
  assert.equal(gatewayRequestDisplayKind(handshake), 'connectionProbe');
  assert.equal(isGatewayRequestUsageApplicable(handshake), false);
  const warmup = { ...request, request_kind: 'websocket_warmup' as const };
  assert.equal(isGatewayRequestUsageApplicable(warmup), false);
  assert.equal(isGatewayRequestUsageApplicable({ ...warmup, total_tokens: 0 }), false);
  assert.equal(isGatewayRequestUsageApplicable({ ...warmup, total_tokens: 12 }), true);
});

test('request presets use the statistics ranges and refresh their relative end time', () => {
  const now = new Date(2026, 8, 12, 22, 24, 0).getTime();
  for (const preset of ['today', 'thisWeek', 'thisMonth', 'thisYear'] as const) {
    const expected = resolveGatewayUsageRange({ preset }, now);
    assert.deepEqual(resolveGatewayRequestRange({ preset }, now), {
      start_date: expected.startDate,
      end_date: expected.endDate,
    });
    assert.equal(resolveGatewayRequestRange({ preset }, now + 60_000).end_date, expected.endDate + 60);
  }
  // Calendar past ranges are fixed: refreshing must not slide the window.
  for (const preset of ['yesterday', 'lastWeek', 'lastMonth'] as const) {
    const expected = resolveGatewayUsageRange({ preset }, now);
    assert.deepEqual(resolveGatewayRequestRange({ preset }, now + 60_000), {
      start_date: expected.startDate,
      end_date: expected.endDate,
    });
  }
  assert.equal(resolveGatewayRequestRange({ preset: 'today' }, now).start_date, new Date(2026, 8, 12).getTime() / 1000);
});

test('calendar presets use local day, Monday-start week, month and year boundaries', () => {
  // 2026-09-12 is a Saturday, 22:24 local time.
  const now = new Date(2026, 8, 12, 22, 24, 0).getTime();
  const midnight = new Date(2026, 8, 12).getTime() / 1000;
  const day = 86_400;

  assert.deepEqual(resolveGatewayUsageRange({ preset: 'yesterday' }, now), {
    startDate: midnight - day,
    endDate: midnight - 1,
  });
  // Monday 2026-09-07 00:00 up to now.
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'thisWeek' }, now), {
    startDate: new Date(2026, 8, 7).getTime() / 1000,
    endDate: Math.floor(now / 1000),
  });
  // The previous Monday through the last second before this week starts.
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'lastWeek' }, now), {
    startDate: new Date(2026, 7, 31).getTime() / 1000,
    endDate: Math.floor(new Date(2026, 8, 7).getTime() / 1000) - 1,
  });
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'thisMonth' }, now), {
    startDate: new Date(2026, 8, 1).getTime() / 1000,
    endDate: Math.floor(now / 1000),
  });
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'thisYear' }, now), {
    startDate: new Date(2026, 0, 1).getTime() / 1000,
    endDate: Math.floor(now / 1000),
  });
  // The previous calendar month through the last second before this month.
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'lastMonth' }, now), {
    startDate: new Date(2026, 7, 1).getTime() / 1000,
    endDate: Math.floor(new Date(2026, 8, 1).getTime() / 1000) - 1,
  });
});

test('last month steps back across a year boundary', () => {
  // Friday 2026-01-09 12:00 local time.
  const january = new Date(2026, 0, 9, 12, 0, 0).getTime();
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'lastMonth' }, january), {
    startDate: new Date(2025, 11, 1).getTime() / 1000,
    endDate: Math.floor(new Date(2026, 0, 1).getTime() / 1000) - 1,
  });
});

test('week presets stay correct across a year boundary and on a Monday', () => {
  // Thursday 2026-01-01 08:00: this week starts Monday 2025-12-29.
  const newYear = new Date(2026, 0, 1, 8, 0, 0).getTime();
  assert.equal(
    resolveGatewayUsageRange({ preset: 'thisWeek' }, newYear).startDate,
    new Date(2025, 11, 29).getTime() / 1000,
  );
  assert.equal(
    resolveGatewayUsageRange({ preset: 'lastWeek' }, newYear).startDate,
    new Date(2025, 11, 22).getTime() / 1000,
  );

  // On a Monday, "last week" is the Monday seven days before.
  const monday = new Date(2026, 8, 14, 9, 0, 0).getTime();
  assert.deepEqual(resolveGatewayUsageRange({ preset: 'thisWeek' }, monday), {
    startDate: Math.floor(new Date(2026, 8, 14).getTime() / 1000),
    endDate: Math.floor(monday / 1000),
  });
  assert.equal(
    resolveGatewayUsageRange({ preset: 'lastWeek' }, monday).startDate,
    new Date(2026, 8, 7).getTime() / 1000,
  );
});

test('request all-time and cleared custom ranges preserve the unbounded search', () => {
  assert.deepEqual(resolveGatewayRequestRange({ preset: 'all' }), { start_date: null, end_date: null });
  assert.deepEqual(resolveGatewayRequestRange({ preset: 'custom', customRange: null }), { start_date: null, end_date: null });
});

test('request custom timestamps remain fixed when refreshing and keep open-ended bounds', () => {
  const start = { toDate: () => new Date(2026, 8, 10, 9, 30) };
  const end = { toDate: () => new Date(2026, 8, 12, 22, 24) };
  const selection = { preset: 'custom' as const, customRange: [start, end] as [typeof start, typeof end] };
  const expected = { start_date: start.toDate().getTime() / 1000, end_date: end.toDate().getTime() / 1000 };
  assert.deepEqual(resolveGatewayRequestRange(selection, 0), expected);
  assert.deepEqual(resolveGatewayRequestRange(selection, 60_000), expected);
  assert.deepEqual(resolveGatewayRequestRange({ preset: 'custom', customRange: [start, null] }), { ...expected, end_date: null });
});

test('duration pairs preserve subsecond TTFT and long-request precision', () => {
  assert.equal(formatDurationPair(13_600, 400), '0.4s/13.6s');
  assert.equal(formatDurationPair(13_600, 0), '0.0s/13.6s');
  assert.equal(formatDurationPair(13_600, null), '13.6s');
  assert.equal(formatDurationPair(400), '400ms');
  assert.equal(formatDurationPair(400, 800), '400ms');
  assert.equal(formatDurationPair(400, -1), '400ms');
  assert.equal(formatDuration(Number.NaN), '-');
});

test('TPS formats output speed with units and subtracts TTFT only for streaming generation', () => {
  const record = {
    method: 'POST',
    path: '/openai/v1/responses',
    requested_model: 'gpt-5',
    output_tokens: 297,
    duration_ms: 13_600,
    first_token_ms: 400,
    is_streaming: true,
  };
  assert.equal(formatTps(record), '22.5 tok/s');
  assert.equal(formatTps({ ...record, is_streaming: false }), '21.8 tok/s');
  assert.equal(formatTps({ ...record, first_token_ms: null }), '21.8 tok/s');
  assert.equal(formatTps({ ...record, first_token_ms: 0 }), '21.8 tok/s');
  assert.equal(formatTps({ ...record, output_tokens: 66, duration_ms: 2600 }), '30 tok/s');
  for (const invalid of [
    { duration_ms: 0 },
    { duration_ms: Number.NaN },
    { first_token_ms: 13_600 },
    { first_token_ms: 15_000 },
    { first_token_ms: -1 },
    { output_tokens: 0 },
    { output_tokens: null },
  ]) {
    assert.equal(formatTps({ ...record, ...invalid }), null);
  }
  assert.equal(formatTps({ ...record, method: 'GET', path: '/openai/v1/models' }), null);
});

test('session rows rate end-to-end because they have no first-token mark', () => {
  const session = {
    method: '',
    path: '',
    requested_model: 'gpt-5',
    output_tokens: 297,
    duration_ms: 13_600,
    first_token_ms: null,
    is_streaming: false,
    data_source: 'session',
  };
  // The whole duration counts: 297 / 13.6s, and a stray first-token mark on a
  // session row is ignored rather than subtracted.
  assert.equal(formatTps(session), '21.8 tok/s');
  assert.equal(formatTps({ ...session, first_token_ms: 400 }), '21.8 tok/s');
  assert.equal(formatTps({ ...session, duration_ms: 0 }), null);
  assert.equal(formatTps({ ...session, output_tokens: 0 }), null);
});

test('session duration shows an estimate and keeps missing values as a dash', () => {
  assert.equal(sessionDurationText(13_600), '13.6s');
  assert.equal(sessionDurationText(400), '400ms');
  assert.equal(sessionDurationText(0), '-');
  assert.equal(sessionDurationText(null), '-');
  assert.equal(sessionDurationText(undefined), '-');
  assert.equal(sessionDurationText(Number.NaN), '-');
});

test('effort display uses explicit metadata and cache rate distinguishes zero from no data', () => {
  assert.equal(formatModelWithEffort('gpt-6-astra', 'high'), 'gpt-6-astra (high)');
  assert.equal(formatModelWithEffort('gpt-6-astra', null), 'gpt-6-astra');
  assert.equal(formatModelWithEffort('gpt-6-astra', ' '), 'gpt-6-astra');
  assert.equal(formatCacheHitRate(0.4), '40.0%');
  assert.equal(formatCacheHitRate(0), '0.0%');
  assert.equal(formatCacheHitRate(1), '100.0%');
  assert.equal(formatCacheHitRate(null), '-');
});

test('service tier labels only recognize the canonical Fast/Ultrafast ids', () => {
  assert.equal(serviceTierLabel('fast'), 'Fast');
  assert.equal(serviceTierLabel(' Fast '), 'Fast');
  assert.equal(serviceTierLabel('ultrafast'), 'Ultrafast');
  assert.equal(serviceTierLabel('ULTRAFAST'), 'Ultrafast');
  for (const tier of ['priority', 'default', 'auto', 'flex', '', null, undefined]) {
    assert.equal(serviceTierLabel(tier), null);
  }
});

test('compaction badges name the side that wrote the item', () => {
  // The wire looks identical whichever side answered, so this badge is the only
  // place a user can tell a native upstream from the gateway's own summary.
  assert.equal(compactionModeKey('native'), 'gateway.page.requests.compaction.native');
  assert.equal(compactionModeKey(' Native '), 'gateway.page.requests.compaction.native');
  assert.equal(compactionModeKey('fallback'), 'gateway.page.requests.compaction.fallback');
  assert.equal(compactionModeKey('simulated'), 'gateway.page.requests.compaction.simulated');
  // Codex's own checkpoint compaction produces no `compaction` item at all; the
  // badge still reports that the turn was a compaction.
  assert.equal(compactionModeKey('local'), 'gateway.page.requests.compaction.local');
  // An ordinary turn, and any value the backend stops sending, stays unlabelled.
  for (const mode of [null, undefined, '', '  ', 'unknown']) {
    assert.equal(compactionModeKey(mode), null);
  }
});

test('native tool badges cover only tools that were actually invoked', () => {
  const cases: Array<[string, string]> = [
    ['web_search', 'gateway.page.requests.nativeTools.webSearch'],
    ['image_generation', 'gateway.page.requests.nativeTools.imageGeneration'],
    ['code_interpreter', 'gateway.page.requests.nativeTools.codeInterpreter'],
    ['file_search', 'gateway.page.requests.nativeTools.fileSearch'],
    ['mcp', 'gateway.page.requests.nativeTools.mcp'],
    ['computer_use', 'gateway.page.requests.nativeTools.computerUse'],
    ['local_shell', 'gateway.page.requests.nativeTools.localShell'],
  ];
  for (const [kind, key] of cases) {
    assert.equal(nativeToolBadgeKey(kind), key, kind);
    assert.equal(nativeToolBadgeKey(` ${kind.toUpperCase()} `), key, `${kind} is case/space insensitive`);
  }
  // Unknown kinds render nothing rather than the raw machine key.
  for (const kind of [null, undefined, '', 'declared_only', 'web_search_call']) {
    assert.equal(nativeToolBadgeKey(kind), null);
  }
});

test('native tool badges cap at two and collapse the rest into a count', () => {
  const empty = nativeToolBadges(undefined);
  assert.deepEqual(empty, { keys: [], labels: [], hiddenCount: 0, allKinds: [] });
  assert.deepEqual(nativeToolBadges([]), { keys: [], labels: [], hiddenCount: 0, allKinds: [] });

  const one = nativeToolBadges(['web_search']);
  assert.deepEqual(one, {
    keys: ['gateway.page.requests.nativeTools.webSearch'],
    labels: ['gateway.page.requests.nativeTools.webSearch'],
    hiddenCount: 0,
    allKinds: ['web_search'],
  });

  const many = nativeToolBadges(['web_search', 'mcp', 'code_interpreter']);
  assert.deepEqual(many.keys, [
    'gateway.page.requests.nativeTools.webSearch',
    'gateway.page.requests.nativeTools.mcp',
  ]);
  assert.equal(many.hiddenCount, 1);
  // The tooltip names every invoked tool, including the collapsed one, and in
  // the user's words rather than wire-level kind ids.
  assert.deepEqual(many.allKinds, ['web_search', 'mcp', 'code_interpreter']);
  assert.deepEqual(many.labels, [
    'gateway.page.requests.nativeTools.webSearch',
    'gateway.page.requests.nativeTools.mcp',
    'gateway.page.requests.nativeTools.codeInterpreter',
  ]);

  // Unknown kinds are not tools this list can name, so they neither render nor
  // inflate the overflow count.
  const withUnknown = nativeToolBadges(['web_search', 'mystery_tool', 'mcp']);
  assert.equal(withUnknown.keys.length, 2);
  assert.equal(withUnknown.labels.length, 2);
  assert.equal(withUnknown.hiddenCount, 0);
});

test('transport badge reports the transport actually used, not the one attempted', () => {
  // A handshake is the Gateway declining an upgrade, not a WebSocket turn.
  assert.equal(
    gatewayTransportBadgeKey({ transport: 'websocket', request_kind: 'websocket_handshake', status_code: 426 }),
    'gateway.page.requests.websocket.fallback',
  );
  // Other handshake failures are already reported by the status column.
  for (const status of [401, 500, 0, null]) {
    assert.equal(
      gatewayTransportBadgeKey({ transport: 'websocket', request_kind: 'websocket_handshake', status_code: status }),
      null,
      String(status),
    );
  }
  assert.equal(
    gatewayTransportBadgeKey({ transport: 'websocket', request_kind: 'request' }),
    'gateway.page.requests.websocket.badge',
  );
  assert.equal(
    gatewayTransportBadgeKey({ transport: 'websocket', request_kind: 'websocket_warmup' }),
    'gateway.page.requests.websocket.badge',
  );
  assert.equal(gatewayTransportBadgeKey({ transport: 'http', request_kind: 'request' }), null);
  // Imported session usage was never a gateway request at all.
  assert.equal(
    gatewayTransportBadgeKey({ transport: 'http', request_kind: 'request', data_source: 'session' }),
    null,
  );
});

test('model qualifiers join effort and service tier without inventing placeholders', () => {
  assert.equal(formatModelQualifiers('gpt-5.6-sol', ['high', 'Fast']), 'gpt-5.6-sol (high · Fast)');
  assert.equal(formatModelQualifiers('gpt-5.6-sol', [null, 'Ultrafast']), 'gpt-5.6-sol (Ultrafast)');
  assert.equal(formatModelQualifiers('gpt-5.6-sol', [' high ', ' ']), 'gpt-5.6-sol (high)');
  assert.equal(formatModelQualifiers('gpt-5.6-sol', []), 'gpt-5.6-sol');
  assert.equal(formatModelQualifiers('gpt-5.6-sol', [null, undefined]), 'gpt-5.6-sol');
});

test('formatUsd uses two decimals by default and allows precise small values', () => {
  assert.equal(formatUsd('0.000001'), '$0.00');
  assert.equal(formatUsd('0.000001', 6), '$0.000001');
});

test('overview cache hit rate uses all input categories and distinguishes no usage from no hits', () => {
  assert.equal(calculateCacheHitRate(100, 80, 20), 0.4);
  assert.equal(calculateCacheHitRate(0, 0, 0), null);
  assert.equal(calculateCacheHitRate(100, 0, 20), 0);
  assert.equal(calculateCacheHitRate(0, 100, 0), 1);
});

test('request rate follows the CLI filter and keeps no traffic distinct from unloaded status', () => {
  const status = {
    requests_per_minute: 30,
    requests_per_minute_by_cli: { claude: 8, claude_desktop: 6, codex: 16 },
  };
  assert.equal(getGatewayRequestsPerMinute(status), 30);
  assert.equal(getGatewayRequestsPerMinute(status, 'claude'), 8);
  assert.equal(getGatewayRequestsPerMinute(status, 'claude_desktop'), 6);
  assert.equal(getGatewayRequestsPerMinute(status, 'codex'), 16);
  assert.equal(getGatewayRequestsPerMinute(status, 'gemini'), 0);
  assert.equal(getGatewayRequestsPerMinute(status, 'pi'), null);
  assert.equal(getGatewayRequestsPerMinute(status, 'hermes'), null);
  assert.equal(getGatewayRequestsPerMinute(null), null);
  assert.equal(getGatewayRequestsPerMinute(undefined, 'codex'), null);
  assert.equal(getGatewayRequestsPerMinute({ requests_per_minute: 0, requests_per_minute_by_cli: {} }, 'claude'), 0);
});

/**
 * The two products that are both usage tools and takeover targets.
 *
 * A request rate is counted under the *takeover* spelling
 * (`workbuddy_desktop` / `codebuddy_cli`) while the filter passes the *usage*
 * spelling, so a plain dictionary lookup never found the entry. Listing them as
 * usage-only made the rate `-` even with real traffic; dropping the usage-only
 * entry alone would have made it a confident `0`, which is worse.
 */
test('request rate resolves the takeover spelling of a dual-spelling tool', () => {
  const status = {
    requests_per_minute: 30,
    requests_per_minute_by_cli: {
      workbuddy_desktop: 4,
      codebuddy_cli: 7,
    } as Record<string, number>,
  };
  assert.equal(getGatewayRequestsPerMinute(status, 'workbuddy'), 4);
  assert.equal(getGatewayRequestsPerMinute(status, 'codebuddy'), 7);
  // The international WorkBuddy edition has no takeover path at all.
  assert.equal(getGatewayRequestsPerMinute(status, 'workbuddy_ai'), null);
});

test('normalizeAttemptCounts falls back total attempts for legacy request logs', () => {
  assert.deepEqual(normalizeAttemptCounts({ attempt_count: 2, total_attempt_count: 0 }), {
    current: 2,
    total: 2,
  });
});

test('normalizeAttemptCounts keeps total attempts when present', () => {
  assert.deepEqual(normalizeAttemptCounts({ attempt_count: 1, total_attempt_count: 3 }), {
    current: 1,
    total: 3,
  });
});

test('shouldShowBodyComparison only shows distinct stored bodies', () => {
  assert.equal(shouldShowBodyComparison(null, '{"ok":true}'), false);
  assert.equal(shouldShowBodyComparison('{"ok":true}', '{"ok":true}'), false);
  assert.equal(shouldShowBodyComparison('{"upstream":true}', '{"client":true}'), true);
});

test('formatModelRoute ignores placeholder model values', () => {
  assert.equal(formatModelRoute('unknown', 'anthropic/claude-sonnet-4-5', '-'), 'anthropic/claude-sonnet-4-5');
  assert.equal(formatModelRoute('claude-sonnet-4-5', 'anthropic/claude-sonnet-4-5', '-'), 'claude-sonnet-4-5 -> anthropic/claude-sonnet-4-5');
  assert.equal(formatModelRoute('unknown', 'unknown', '-'), '-');
});

test('gateway request display detects model list endpoints across CLIs', () => {
  assert.equal(gatewayRequestDisplayKind({
    method: 'GET',
    path: '/anthropic/v1/models',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'modelList');
  assert.equal(gatewayRequestDisplayKind({
    method: 'GET',
    path: '/openai/v1/models',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'modelList');
  assert.equal(gatewayRequestDisplayKind({
    method: 'GET',
    path: '/gemini/v1beta/models?key=xxx',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'modelList');
  assert.equal(gatewayRequestDisplayKind({
    method: 'GET',
    path: '/gemini/v1beta/models:listModels',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'modelList');
});

test('gateway request display detects compact and connection probes', () => {
  assert.equal(gatewayRequestDisplayKind({
    method: 'POST',
    path: '/openai/v1/responses/compact',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'contextCompact');
  assert.equal(gatewayRequestDisplayKind({
    method: 'HEAD',
    path: '/anthropic',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'connectionProbe');
  assert.equal(gatewayRequestDisplayKind({
    method: 'GET',
    path: '/openai/v1',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'connectionProbe');
  assert.equal(gatewayRequestDisplayKind({
    method: 'HEAD',
    path: '/gemini/v1beta',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'connectionProbe');
});

test('deriveGatewayRequestDisplay exposes title keys and request line metadata', () => {
  const display = deriveGatewayRequestDisplay({
    method: 'GET',
    path: '/openai/v1/models',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  });

  assert.equal(display.kind, 'modelList');
  assert.equal(display.titleKey, 'gateway.page.requests.requestTypes.modelList');
  assert.equal(display.requestLine, 'GET /openai/v1/models');
  assert.equal(display.modelApplicable, false);
});

test('usage display is only applicable to model and compact requests', () => {
  assert.equal(isGatewayRequestUsageApplicable({
    data_source: 'session', requested_model: 'unknown', upstream_model_id: 'unknown',
  }), true);
  assert.equal(isGatewayRequestUsageApplicable({
    method: 'POST',
    path: '/openai/v1/responses',
    requested_model: 'gpt-5',
    upstream_model_id: 'openai/gpt-5',
  }), true);
  assert.equal(isGatewayRequestUsageApplicable({
    method: 'POST',
    path: '/openai/v1/responses/compact',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), true);
  assert.equal(isGatewayRequestUsageApplicable({
    method: 'GET',
    path: '/openai/v1/models',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), false);
  assert.equal(isGatewayRequestUsageApplicable({
    method: 'HEAD',
    path: '/gemini/v1beta',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), false);
  assert.equal(isGatewayRequestUsageApplicable({
    method: 'POST',
    path: '/openai/v1/embeddings',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), false);
});

test('requestLineText falls back when method and path are absent', () => {
  assert.equal(requestLineText({}, 'Path not recorded'), 'Path not recorded');
  assert.equal(requestLineText({ method: 'get' }, 'Path not recorded'), 'GET');
  assert.equal(requestLineText({ path: '/openai/v1/models' }, 'Path not recorded'), '/openai/v1/models');
});

test('requestExportPrefix avoids unknown filenames for non-model requests', () => {
  assert.equal(requestExportPrefix({
    method: 'GET',
    path: '/openai/v1/models',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'models-list');
  assert.equal(requestExportPrefix({
    method: 'POST',
    path: '/responses/compact',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'compact');
  assert.equal(requestExportPrefix({
    method: 'HEAD',
    path: '/gemini/v1beta',
    requested_model: 'unknown',
    upstream_model_id: 'unknown',
  }), 'probe');
  assert.equal(requestExportPrefix({
    method: 'POST',
    path: '/openai/v1/responses',
    requested_model: 'gpt-5',
    upstream_model_id: 'openai/gpt-5',
  }), 'gpt-5-openai-gpt-5');
});

test('unpriced model ids dedupe across CLIs, sort, and skip placeholders and priced rows', () => {
  assert.deepEqual(
    unpricedModelIds([
      { model: 'gpt-5.4-pro', has_pricing: false },
      { model: 'unknown', has_pricing: false },
      { model: '  Unknown ', has_pricing: false },
      { model: '', has_pricing: false },
      { model: 'claude-sonnet-4-5', has_pricing: true },
      { model: 'gpt-5.4-pro', has_pricing: false },
      { model: 'glm-5-turbo', has_pricing: false },
      { model: 'free-but-priced', has_pricing: true },
    ]),
    ['glm-5-turbo', 'gpt-5.4-pro'],
  );
  assert.deepEqual(unpricedModelIds([]), []);
  // `has_pricing: true` means the price table resolves the id, not that the
  // displayed amount is non-zero, so priced rows never enter the list.
  assert.deepEqual(unpricedModelIds([{ model: 'claude-opus-4-5', has_pricing: true }]), []);
});

test('project names come from the last path segment and tolerate both separators', () => {
  assert.equal(gatewayProjectName(String.raw`D:\GitHub\ai-toolbox`), 'ai-toolbox');
  assert.equal(gatewayProjectName('/home/me/repos/ai-toolbox'), 'ai-toolbox');
  assert.equal(gatewayProjectName(String.raw`D:/mixed\style/repo`), 'repo');
  assert.equal(gatewayProjectName('  /home/me/repo/  '), 'repo');
  assert.equal(gatewayProjectName('/'), null);
  assert.equal(gatewayProjectName('   '), null);
  // A bare drive prefix has no project segment left after trimming.
  assert.equal(gatewayProjectName('C:' + '\\'), 'C:');
});

test('stored ranges drop absolute bounds for relative presets and freeze custom ones', () => {
  assert.deepEqual(gatewayUsageRangeToStored({ preset: 'today' }), {
    preset: 'today',
    start_date: null,
    end_date: null,
  });
  assert.deepEqual(gatewayUsageRangeToStored({ preset: 'lastMonth' }), {
    preset: 'lastMonth',
    start_date: null,
    end_date: null,
  });

  const start = new Date(2026, 8, 1, 9, 30, 0);
  const end = new Date(2026, 8, 5, 18, 0, 0);
  const asDateLike = (date: Date) => ({ toDate: () => date });
  assert.deepEqual(
    gatewayUsageRangeToStored({ preset: 'custom', customRange: [asDateLike(start), asDateLike(end)] }),
    {
      preset: 'custom',
      start_date: Math.floor(start.getTime() / 1000),
      end_date: Math.floor(end.getTime() / 1000),
    },
  );
  // A half-filled custom range keeps the missing side null so the backend can
  // fall back to "now" / "24h before end" instead of inventing a bound.
  assert.deepEqual(
    gatewayUsageRangeToStored({ preset: 'custom', customRange: [asDateLike(start), null] }),
    {
      preset: 'custom',
      start_date: Math.floor(start.getTime() / 1000),
      end_date: null,
    },
  );
});

test('range keys distinguish presets from custom bounds', () => {
  assert.equal(
    gatewayUsageRangeKey({ preset: 'today' }),
    gatewayUsageRangeKey({ preset: 'today' }),
  );
  assert.notEqual(
    gatewayUsageRangeKey({ preset: 'today' }),
    gatewayUsageRangeKey({ preset: 'lastMonth' }),
  );
  const start = new Date(2026, 8, 1, 9, 30, 0);
  const end = new Date(2026, 8, 5, 18, 0, 0);
  const asDateLike = (date: Date) => ({ toDate: () => date });
  assert.notEqual(
    gatewayUsageRangeKey({ preset: 'custom', customRange: [asDateLike(start), asDateLike(end)] }),
    gatewayUsageRangeKey({ preset: 'custom', customRange: [asDateLike(start), null] }),
  );
});
