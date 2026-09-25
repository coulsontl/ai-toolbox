import assert from 'node:assert/strict';
import test from 'node:test';

import {
  calculateCacheHitRate,
  deriveGatewayRequestDisplay,
  formatCacheHitRate,
  formatDuration,
  formatDurationPair,
  formatModelRoute,
  formatModelWithEffort,
  formatTps,
  formatUsd,
  gatewayRequestDisplayKind,
  gatewayWebSocketStatusKey,
  getGatewayRequestsPerMinute,
  isGatewayRequestUsageApplicable,
  normalizeAttemptCounts,
  requestExportPrefix,
  requestLineText,
  resolveGatewayRequestRange,
  resolveGatewayUsageRange,
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
  for (const preset of ['today', '1d', '7d', '14d', '30d'] as const) {
    const expected = resolveGatewayUsageRange({ preset }, now);
    assert.deepEqual(resolveGatewayRequestRange({ preset }, now), {
      start_date: expected.startDate,
      end_date: expected.endDate,
    });
    assert.equal(resolveGatewayRequestRange({ preset }, now + 60_000).end_date, expected.endDate + 60);
  }
  assert.equal(resolveGatewayRequestRange({ preset: 'today' }, now).start_date, new Date(2026, 8, 12).getTime() / 1000);
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
  assert.equal(formatTps({ ...record, data_source: 'session' }), null);
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
