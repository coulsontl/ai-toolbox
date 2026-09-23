import type { GatewayAggregateNamingMode, GatewayCliTakeoverStatus } from '@/services';
import type { GatewayAggregateReengageConfig } from './providerSaveReengage';

/** Takeover mode a provider save has to replay around itself. */
export type GatewayReengageMode = 'single' | 'failover' | 'aggregate' | null | undefined;

/**
 * Narrow a stored takeover mode to one this flow can replay.
 *
 * Lives here, beside `resolveGatewayReengageMode`, so the aggregate helpers and
 * the re-engage flow can share one definition without an import cycle.
 */
export const isGatewayReengageMode = (
  gatewayMode: GatewayReengageMode,
): gatewayMode is 'single' | 'failover' | 'aggregate' =>
  gatewayMode === 'single' || gatewayMode === 'failover' || gatewayMode === 'aggregate';

/**
 * Aggregate-mode helpers shared by the gateway settings panel and the provider
 * save/re-engage flow.
 *
 * Backend contract (mirrors `cli_proxy/manifest.rs` and `aggregate_naming.rs`):
 * a provider id matches `^[A-Za-z0-9_-]+$`, a *prefix token* additionally
 * allows any non-ASCII character (CJK site names are a first-class case) and
 * must not contain the separator, whitespace or control characters. The
 * separator must be non-empty and must not contain letters, digits, `_` or `-`,
 * otherwise `<site_id><sep><model>` cannot be split back into its parts. Keep
 * this module free of i18n text so the callers decide how to phrase the error.
 */

export type GatewayAggregateSeparatorInvalidReason = 'empty' | 'reservedCharacters';

const SITE_ID_PATTERN = /^[A-Za-z0-9_-]+$/;
export const AGGREGATE_ALIAS_MAX_LENGTH = 32;

/** Site ids the backend can address; anything else must be dropped before engaging. */
export const isAggregateSiteId = (siteId: string): boolean =>
  SITE_ID_PATTERN.test(siteId.trim());

/**
 * Validate a user-supplied separator. Returns `null` when the separator is
 * usable; the caller maps the reason code to a localized message.
 */
export const validateGatewayAggregateSeparator = (
  separator: string,
): GatewayAggregateSeparatorInvalidReason | null => {
  if (separator.length === 0) {
    return 'empty';
  }
  if (/[A-Za-z0-9_-]/.test(separator)) {
    return 'reservedCharacters';
  }
  return null;
};

/**
 * Mirror of the backend `validate_aggregate_alias`.
 *
 * Only *structural* characters are refused — whitespace, control characters
 * and the configured separator. Every other character (including CJK) is
 * allowed, because the default prefix is the user's own site name. Passing an
 * empty separator skips the separator check (the field is optional in preview
 * call sites).
 */
export const validateGatewayAggregateAlias = (alias: string, separator = ''): boolean => {
  if (alias.length === 0 || alias.length > AGGREGATE_ALIAS_MAX_LENGTH) return false;
  if (/\s/.test(alias)) return false;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(alias)) return false;
  if (separator.length > 0 && alias.includes(separator)) return false;
  return true;
};

/**
 * Mirror of the backend `derive_site_prefix_from_name`.
 *
 * The aggregate catalog defaults a site's prefix to its display name so the
 * model list reads `思源888 pro · gpt-5.6-luna` instead of an opaque provider
 * id. Names are free-form, so normalise: strip control characters, fold
 * whitespace runs to `-`, replace the separator with `-`, and trim dashes.
 * Returns `null` when nothing usable is left (caller keeps the provider id).
 */
export const deriveGatewayAggregateSitePrefix = (
  name: string,
  separator: string,
): string | null => {
  const replaced = separator.length > 0 ? name.split(separator).join('-') : name;
  const out: string[] = [];
  let pendingDash = false;
  for (const ch of replaced.trim()) {
    // Whitespace is checked before the control check: tab/newline are both, and
    // folding them into '-' reads far better than deleting them.
    if (/\s/.test(ch)) {
      pendingDash = out.length > 0;
      continue;
    }
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u001f\u007f]/.test(ch)) continue;
    if (pendingDash) {
      out.push('-');
      pendingDash = false;
    }
    out.push(ch);
  }
  const prefix = out.join('').replace(/^-+/, '').replace(/-+$/, '');
  return prefix.length > 0 ? prefix : null;
};

export const normalizeGatewayAggregateAliases = (
  aliases: Record<string, string> | null | undefined,
  selectedSiteIds: readonly string[],
  allSiteIds: readonly string[] = selectedSiteIds,
  separator = '',
): Record<string, string> | null => {
  const selected = new Set(selectedSiteIds);
  const normalized: Record<string, string> = {};
  for (const [siteId, rawAlias] of Object.entries(aliases ?? {})) {
    const alias = rawAlias.trim();
    if (!alias) continue;
    if (!selected.has(siteId) || !validateGatewayAggregateAlias(alias, separator)) return null;
    normalized[siteId] = alias;
  }
  // Every candidate must answer to exactly one prefix, mirroring the backend's
  // `validate_aggregate_site_prefixes`: a site is addressed by its alias when it
  // has one, otherwise by its provider id. Unselected candidates always keep
  // their id (request-time routing leaves them addressable as fallbacks), so an
  // alias may not shadow one of those either — the form must not submit a
  // selection the engage command would refuse.
  const prefixes = new Set<string>();
  for (const siteId of allSiteIds) {
    const prefix = (selected.has(siteId) ? normalized[siteId] : undefined) ?? siteId;
    const key = prefix.toLowerCase();
    if (prefixes.has(key)) return null;
    prefixes.add(key);
  }
  return normalized;
};

/** Drop duplicate/non-addressable site ids while preserving the user's order. */
export const normalizeGatewayAggregateSiteIds = (siteIds: readonly string[]): string[] => {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const siteId of siteIds) {
    const trimmed = siteId.trim();
    if (!isAggregateSiteId(trimmed) || seen.has(trimmed)) {
      continue;
    }
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
};

/**
 * Drop blank and duplicate bare model names, keeping the first occurrence.
 *
 * An empty result is the backend's "publish every bare model" default, never
 * "publish none", so callers must not read `[]` as a narrowed selection.
 */
export const normalizeSubagentExposedModels = (
  models?: readonly string[] | null,
): string[] => {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const model of models ?? []) {
    const trimmed = model.trim();
    if (!trimmed || seen.has(trimmed)) {
      continue;
    }
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
};

/** Values that can be addressed by the current aggregate catalog. */
export const resolveEffectiveSubagentExposedModels = (
  selectedModels: readonly string[] | null | undefined,
  addressableModels: readonly string[],
): string[] => {
  const addressable = new Set(normalizeSubagentExposedModels(addressableModels));
  return normalizeSubagentExposedModels(selectedModels).filter((model) =>
    addressable.has(model),
  );
};

/** Selected names that cannot be addressed by the current aggregate catalog. */
export const resolveStaleSubagentExposedModels = (
  selectedModels: readonly string[] | null | undefined,
  addressableModels: readonly string[],
): string[] => {
  const effective = new Set(
    resolveEffectiveSubagentExposedModels(selectedModels, addressableModels),
  );
  return normalizeSubagentExposedModels(selectedModels).filter(
    (model) => !effective.has(model),
  );
};

/** Normalize the canonical exposure list returned by an aggregate API call. */
export const resolveCanonicalSubagentExposedModels = (
  response:
    | {
        subagent_exposed_models?: readonly string[] | null;
        aggregate?: { subagent_exposed_models?: readonly string[] | null } | null;
      }
    | null
    | undefined,
): string[] =>
  normalizeSubagentExposedModels(
    response?.aggregate?.subagent_exposed_models ?? response?.subagent_exposed_models,
  );

/**
 * Candidate bare names the exposure block may offer.
 *
 * The backend's `bare_models` universe is the primary source because it is what
 * keeps a name selectable again after the exposure set narrowed it out of the
 * published catalog. Only when the backend omits it (older build, or a catalog
 * read that could not resolve the sites) does this fall back to the published
 * bare-name rows (promoted visible entries or hidden aliases), filtered to the
 * configured takeover's selected sites so a stale catalog entry cannot be
 * offered.
 */
export const resolveSubagentExposureCandidates = (
  catalog:
    | {
        entries?: readonly { model: string; provider_id: string; provider_name: string }[];
        bare_models?: readonly { model: string; provider_id: string; provider_name: string }[];
      }
    | null
    | undefined,
  selectedSiteIds: readonly string[] = [],
): { model: string; provider_id: string; provider_name: string }[] => {
  if (!catalog) {
    return [];
  }
  const selected = new Set(selectedSiteIds);
  const source =
    catalog.bare_models && catalog.bare_models.length > 0
      ? catalog.bare_models
      : (catalog.entries ?? []).filter(
          (entry) =>
            entry.provider_id.length === 0 ||
            selected.size === 0 ||
            selected.has(entry.provider_id),
        );
  const seen = new Set<string>();
  const candidates: { model: string; provider_id: string; provider_name: string }[] = [];
  for (const entry of source) {
    const model = entry.model.trim();
    if (!model || seen.has(model)) {
      continue;
    }
    seen.add(model);
    candidates.push({ ...entry, model });
  }
  return candidates;
};

/**
 * How many ticked bare names fit in Codex's `spawn_agent` model hints.
 *
 * Codex takes the five lowest-priority `visibility = "list"` entries as the
 * model list it shows the model, so only that many names can be promoted into
 * the hint. Mirrors `MAX_SPAWN_AGENT_MODEL_OVERRIDES` on the Codex side.
 */
export const SUBAGENT_EXPOSED_MODEL_LIMIT = 5;

/**
 * Whether the stored selection is complete enough to engage.
 *
 * The backend treats an empty list as the legacy "publish every bare model and
 * promote none", which is not what a user who opened this panel wants: the
 * whole point of the new flow is that the ticked names are what the subagent
 * can see. An engage therefore requires between 1 and
 * `SUBAGENT_EXPOSED_MODEL_LIMIT` usable names.
 */
export const isSubagentExposureSelectionComplete = (
  models?: readonly string[] | null,
): boolean => {
  const count = normalizeSubagentExposedModels(models).length;
  return count > 0 && count <= SUBAGENT_EXPOSED_MODEL_LIMIT;
};

/**
 * Whether a persisted exposure selection can be replayed by an engage at all.
 *
 * Mirrors the backend's `validate_subagent_exposed_model_limit`, which is
 * fail-closed: more than `SUBAGENT_EXPOSED_MODEL_LIMIT` names is a hard error,
 * never a truncation. A selection that exceeds the limit therefore makes *every*
 * aggregate engage fail, and because the provider-save round trip restores
 * direct mode first, replaying one strands the CLI in direct mode — the takeover
 * is dropped and each later provider save fails the same way.
 *
 * An empty or absent selection is the legacy "publish every bare model, promote
 * none" default and is always replayable, so callers must not read `[]` as an
 * un-engageable state.
 */
export const canReplaySubagentExposureSelection = (
  models?: readonly string[] | null,
): boolean =>
  normalizeSubagentExposedModels(models).length <= SUBAGENT_EXPOSED_MODEL_LIMIT;

/**
 * Mirror of the backend `resolve_effective_site_aliases`.
 *
 * Explicit aliases win unchanged. A selected site without one falls back to its
 * normalised display name, unless that derived prefix would steal another
 * site's address (another provider id, or an explicit alias): those cases keep
 * the provider id. The backend applies exactly the same rule, so previews and
 * the engaged catalog cannot drift apart.
 */
export const resolveGatewayAggregateEffectiveAliases = (
  explicit: Record<string, string> | null | undefined,
  selectedSites: readonly { id: string; name: string }[],
  allSiteIds: readonly string[],
  separator: string,
): Record<string, string> => {
  const effective: Record<string, string> = { ...(explicit ?? {}) };
  const taken = new Set<string>();
  for (const id of allSiteIds) taken.add(id.toLowerCase());
  for (const alias of Object.values(effective)) taken.add(alias.toLowerCase());

  for (const site of selectedSites) {
    if (effective[site.id]) continue;
    const prefix = deriveGatewayAggregateSitePrefix(site.name, separator);
    if (!prefix || prefix === site.id) continue;
    if (prefix.length > AGGREGATE_ALIAS_MAX_LENGTH) continue;
    if (!validateGatewayAggregateAlias(prefix, separator)) continue;
    const key = prefix.toLowerCase();
    if (taken.has(key)) continue;
    taken.add(key);
    effective[site.id] = prefix;
  }
  return effective;
};

/**
 * Read the aggregate selection that must be replayed when re-engaging.
 *
 * Returns `null` when the backend did not expose aggregate details, so callers
 * can skip the aggregate round trip instead of silently re-engaging with an
 * empty site list (which would drop the whole cross-site model list).
 */
export const toGatewayAggregateReengageConfig = (
  status?: GatewayCliTakeoverStatus | null,
): GatewayAggregateReengageConfig | null => {
  if (status?.mode !== 'aggregate') {
    return null;
  }
  const aggregate = status.aggregate ?? null;
  if (!aggregate) {
    return null;
  }
  const providerIds = normalizeGatewayAggregateSiteIds(aggregate.provider_ids);
  const separator = aggregate.separator ?? '';
  if (providerIds.length === 0 || validateGatewayAggregateSeparator(separator) !== null) {
    return null;
  }
  const naming: GatewayAggregateNamingMode = aggregate.naming ?? 'site_model';
  if (!['site_model', 'model_at_site', 'model_only'].includes(naming)) {
    return null;
  }
  const aliases = normalizeGatewayAggregateAliases(
    aggregate.aliases,
    providerIds,
    providerIds,
    separator,
  );
  if (!aliases) {
    return null;
  }
  // Carry the managed Codex `[agents]` defaults through the re-engage round
  // trip. Restoring direct drops them, so without this a provider save would
  // silently unset the user's subagent default.
  const subagentModel = aggregate.subagent?.model?.trim();
  const subagentReasoningEffort = aggregate.subagent?.reasoning_effort?.trim();
  // The failover policy is always replayed: a dropped `false` would silently
  // re-enable cross-site spending on the next provider save.
  const crossSiteFailover = aggregate.cross_site_failover === true;
  // A narrowed exposure set must be replayed verbatim, in tick order (the order
  // is the promotion priority). An empty array is the legacy default and is
  // omitted below, which is exactly right for a takeover engaged before the
  // exposure block existed: the backend keeps every bare name addressable,
  // without promoting any, instead of the provider save widening or breaking it.
  const subagentExposedModels = normalizeSubagentExposedModels(
    aggregate.subagent_exposed_models,
  );
  return {
    providerIds,
    separator,
    aliases,
    naming,
    // Only present when the takeover actually manages the key: an explicit
    // `undefined` would read as "managed but blank" to callers that inspect the
    // object's own keys.
    ...(subagentModel ? { subagentModel } : {}),
    ...(subagentReasoningEffort ? { subagentReasoningEffort } : {}),
    crossSiteFailover,
    ...(subagentExposedModels.length > 0 ? { subagentExposedModels } : {}),
  };
};

/**
 * Decide which takeover mode a provider save must replay around itself.
 *
 * `single` and `failover` behave exactly as before. `aggregate` is only
 * replayable when its selection is available, so a missing/incomplete status
 * degrades to "no re-engage" instead of engaging a mode with no sites.
 */
export const resolveGatewayReengageMode = (
  status?: GatewayCliTakeoverStatus | null,
): 'single' | 'failover' | 'aggregate' | null => {
  const mode = status?.mode ?? null;
  if (!isGatewayReengageMode(mode)) {
    return null;
  }
  if (mode === 'aggregate' && !toGatewayAggregateReengageConfig(status)) {
    return null;
  }
  return mode;
};

/** Label of the model list entry Codex sees for one (site, model) pair. */
export const buildGatewayAggregateModelSlug = (
  siteId: string,
  modelId: string,
  separator: string,
  naming: GatewayAggregateNamingMode = 'site_model',
): string => {
  if (naming === 'model_only') return modelId;
  return naming === 'model_at_site'
    ? `${modelId}${separator}${siteId}`
    : `${siteId}${separator}${modelId}`;
};

/** Preview slug shown for one selected site in the aggregate takeover dialog. */
export const buildGatewayAggregateSitePreviewSlug = (
  siteId: string,
  separator: string,
  naming: GatewayAggregateNamingMode = 'site_model',
  aliases?: Record<string, string> | null,
): string => {
  const alias = aliases?.[siteId]?.trim();
  return buildGatewayAggregateModelSlug(alias || siteId, '<model>', separator, naming);
};
