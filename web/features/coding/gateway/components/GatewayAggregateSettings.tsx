import React from 'react';
import { Switch } from 'antd';
import { Loader2, Route } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  DEFAULT_AGGREGATE_SEPARATOR,
  engageProxyGatewayAggregate,
  getProxyGatewayAggregateDraft,
  getProxyGatewayCliStatuses,
  getProxyGatewaySubagentCatalog,
  restoreProxyGatewayCliDirect,
  saveProxyGatewayAggregateDraft,
  type GatewayAggregateConfig,
  type GatewayCliKey,
  type GatewayAggregateNamingMode,
  type GatewayCliTakeoverStatus,
  type GatewaySubagentCatalog,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { listCodexProviders } from '@/services/codexApi';
import type { CodexProvider } from '@/types/codex';
import { isCodexLocalProviderId } from '@/features/coding/codex/utils/localProvider';
import { primaryCodexProviderNeedsGatewayProxy } from '@/features/coding/codex/utils/codexGatewayProxyNeed';
import {
  aliasesForSelectedSites,
  aggregateEngageErrorNoticeKey,
  aggregateEngageRequiresDirectRestore,
  buildGatewayAggregateSitePreviewSlug,
  getGatewayProviderProfilesVersion,
  getGatewayAggregateConfigVersion,
  notifyGatewayAggregateConfigChanged,
  isSubagentExposureSelectionComplete,
  isAggregateSiteId,
  normalizeGatewayAggregateAliases,
  normalizeGatewayAggregateSiteIds,
  normalizeSubagentExposedModels,
  orderAggregateSiteIdsByCandidates,
  resolveCanonicalSubagentExposedModels,
  resolveAggregateFormSeed,
  resolveEffectiveSubagentExposedModels,
  resolveGatewayAggregateEffectiveAliases,
  resolveSubagentExposureCandidates,
  resolveStaleSubagentExposedModels,
  restoreDirectUnavailableHintKey,
  shortAggregateSiteId,
  subscribeGatewayProviderProfiles,
  subscribeGatewayAggregateConfig,
  runGatewayAggregateMutation,
  SUBAGENT_EXPOSED_MODEL_LIMIT,
  toAggregateSiteCandidates,
  validateGatewayAggregateSeparator,
  validateGatewayAggregateAlias,
  type GatewayAggregateSiteCandidate,
} from '@/features/coding/shared/gateway';
import styles from './GatewayAggregateSettings.module.less';

// The backend aggregate manifest and model catalog are Codex-specific for now.
// Keep the selector honest instead of exposing CLI choices that the command
// layer will reject.
type AggregateCliKey = Extract<GatewayCliKey, 'codex'>;

const AGGREGATE_CLI_KEYS: AggregateCliKey[] = [
  'codex',
];

/**
 * Reuse the providers page data source per CLI; the gateway cannot reach any
 * provider the CLI page does not already own, so no extra request is invented.
 *
 * Returns the raw records too: the settings block has to re-check the primary
 * provider's protocol requirement before restoring direct mode, and that needs
 * the provider's `meta`/`settingsConfig`, not just its id and name.
 */
const loadProviders = async (
  cliKey: AggregateCliKey,
): Promise<{ providers: CodexProvider[]; candidates: GatewayAggregateSiteCandidate[] }> => {
  switch (cliKey) {
    case 'codex': {
      const providers = await listCodexProviders();
      return { providers, candidates: toAggregateSiteCandidates(providers) };
    }
    default:
      return { providers: [], candidates: [] };
  }
};

const formatError = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

/**
 * Failure notices the engage flow can show. `enableFailedAfterRestore` exists
 * because the primary switch first restores direct mode: if the engage then
 * fails, the user needs to know the CLI was left in direct mode, not that
 * "nothing happened".
 */
type AggregateFailureNoticeKey =
  | 'enableFailed'
  | 'enableFailedAfterRestore'
  | 'disableFailed';

interface SelectedSiteRowProps {
  candidate: GatewayAggregateSiteCandidate;
  /** Aggregate mode cannot be empty, so the only selected site stays selected. */
  canDeselect: boolean;
  routePreview: string;
  onToggleSite: (siteId: string, checked: boolean) => void;
  alias: string;
  separator: string;
  onAliasChange: (siteId: string, alias: string) => void;
  onAliasCommit: () => void;
}

/**
 * Selected site row. Order is the aggregate fallback priority, but it is not a
 * panel-local choice: rows render in the CLI provider-list order, and that list
 * is the only place a site can be moved (see `orderAggregateSiteIdsByCandidates`).
 * So this row exposes no drag handle and no up/down buttons.
 */
const SelectedSiteRow: React.FC<SelectedSiteRowProps> = ({
  candidate,
  canDeselect,
  routePreview,
  onToggleSite,
  alias,
  separator,
  onAliasChange,
  onAliasCommit,
}) => {
  const { t } = useTranslation();

  return (
    <li className={styles.siteItem}>
      <input
        type="checkbox"
        checked
        disabled={!canDeselect}
        aria-label={candidate.name}
        title={canDeselect ? undefined : t('gateway.aggregate.lastSiteRequired')}
        onChange={(event) => onToggleSite(candidate.id, event.currentTarget.checked)}
      />
      <span className={styles.siteName} title={candidate.name}>
        {candidate.name}
        <span className={styles.siteRoute}>
          {t('gateway.aggregate.routePreview')}: <code>{routePreview}</code>
        </span>
      </span>
      <code className={styles.siteSlug} title={candidate.id}>
        {shortAggregateSiteId(candidate.id)}
      </code>
      <input
        className={styles.aliasInput}
        value={alias}
        maxLength={32}
        placeholder={t('gateway.aggregate.aliasPlaceholder')}
        aria-label={`${candidate.name}: ${t('gateway.aggregate.alias')}`}
        aria-invalid={alias.length > 0 && !validateGatewayAggregateAlias(alias, separator)}
        onChange={(event) => onAliasChange(candidate.id, event.currentTarget.value)}
        onBlur={onAliasCommit}
      />
    </li>
  );
};

interface GatewayAggregateSettingsProps {
  /** Gateway must be running before any engage command can succeed. */
  running: boolean;
  /**
   * Notified after a successful engage/disengage so the settings panel can
   * refresh its own takeover list. Must be stable (useCallback with no deps) —
   * it is not called on load, so it cannot feed back into the seed effect.
   */
  onTakeoverChange?: () => void;
}

const GatewayAggregateSettings: React.FC<GatewayAggregateSettingsProps> = ({
  running,
  onTakeoverChange,
}) => {
  const { t } = useTranslation();
  const [cliKey, setCliKey] = React.useState<AggregateCliKey>('codex');
  const [candidates, setCandidates] = React.useState<GatewayAggregateSiteCandidate[]>([]);
  const [providers, setProviders] = React.useState<CodexProvider[]>([]);
  const [loadingSites, setLoadingSites] = React.useState(true);
  const [siteIds, setSiteIds] = React.useState<string[]>([]);
  const [separator, setSeparator] = React.useState<string>(DEFAULT_AGGREGATE_SEPARATOR);
  const [aliases, setAliases] = React.useState<Record<string, string>>({});
  const [naming, setNaming] = React.useState<GatewayAggregateNamingMode>('site_model');
  // Optional Codex `[agents]` defaults. Blank means "don't manage this key", so
  // an untouched form never writes to the user's `[agents]` section.
  const [subagentModel, setSubagentModel] = React.useState('');
  const [subagentReasoningEffort, setSubagentReasoningEffort] = React.useState('');
  // Cross-site failover is off by default: a request stays on the site its slug
  // names, so a failure reports an error instead of spending another site.
  const [crossSiteFailover, setCrossSiteFailover] = React.useState(false);
  /**
   * Programmable bare-name exposure, in tick order. The order *is* the
   * priority: the first ticked name wins the lowest `spawn_agent` model hint.
   * An empty list is the backend's legacy "publish every bare model and promote
   * none" default, which is why an engage requires a non-empty selection.
   */
  const [subagentExposedModels, setSubagentExposedModels] = React.useState<string[]>([]);
  const [subagentCatalog, setSubagentCatalog] = React.useState<GatewaySubagentCatalog | null>(
    null,
  );
  const [subagentCatalogLoading, setSubagentCatalogLoading] = React.useState(false);
  const [cliStatuses, setCliStatuses] = React.useState<GatewayCliTakeoverStatus[]>([]);
  const [busy, setBusy] = React.useState(false);
  // Bumped when the persisted draft has been (re)loaded, so the seed effect can
  // pick up a fresh draft without re-seeding on every unrelated status refresh —
  // re-seeding mid-edit would revert what the user just changed.
  const [draftLoadRevision, setDraftLoadRevision] = React.useState(0);
  const [droppedDraftSites, setDroppedDraftSites] = React.useState(false);
  const [notice, setNotice] = React.useState<{ kind: 'error' | 'success'; text: string } | null>(
    null,
  );
  const aggregateConfigVersion = React.useSyncExternalStore(
    subscribeGatewayAggregateConfig,
    getGatewayAggregateConfigVersion,
    getGatewayAggregateConfigVersion,
  );
  const revisionRef = React.useRef(0);
  const statusRequestRef = React.useRef(0);
  const mutationRevisionRef = React.useRef(0);
  const draftRequestRef = React.useRef(0);
  const catalogRequestRef = React.useRef(0);
  const savedDraftRef = React.useRef<GatewayAggregateConfig | null>(null);
  const mountedRef = React.useRef(true);
  const selectedStatus = React.useMemo(
    () => cliStatuses.find((status) => status.cli_key === cliKey) ?? null,
    [cliKey, cliStatuses],
  );
  const engaged = selectedStatus?.mode === 'aggregate';
  // The primary provider's protocol is partly read from the gateway provider
  // profile store (`getGatewayProviderApiFormatFromMeta`), which changes without
  // the provider list identity changing. Subscribe the same way the Codex page
  // and provider card do, otherwise editing that profile while this panel is open
  // leaves the guard below answering about the previous protocol.
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  // Aggregate names the first selected site as `primary_provider_id`. The shared
  // gateway dialog refuses to restore direct while that provider still needs the
  // gateway for protocol conversion, so this entry point must refuse too —
  // otherwise the switch silently writes a direct config Codex cannot use.
  const primaryNeedsProxy = React.useMemo(
    () =>
      primaryCodexProviderNeedsGatewayProxy(
        providers,
        selectedStatus?.primary_provider_id,
        isCodexLocalProviderId,
      ),
    [gatewayProviderProfilesVersion, providers, selectedStatus?.primary_provider_id],
  );
  const restoreDirectBlocked = engaged && primaryNeedsProxy.needsProxy;
  const restoreDirectBlockedHint = t(
    restoreDirectUnavailableHintKey(primaryNeedsProxy.reason),
    { cli: t(`settings.gateway.cli.${cliKey}`) },
  );
  const separatorError = validateGatewayAggregateSeparator(separator);
  // Unselected candidates keep answering to their provider id, so aliases are
  // validated against the full candidate set — the same set the backend checks.
  const candidateSiteIds = React.useMemo(
    () => candidates.map((candidate) => candidate.id),
    [candidates],
  );
  const normalizedSiteIds = normalizeGatewayAggregateSiteIds(siteIds);
  // The backend refuses to change the primary provider while a manifest is
  // enabled (`prepare_manifest`), so an engage whose first site differs has to
  // restore direct first — the same round trip the provider list uses when it
  // switches the primary of a single/failover takeover.
  const engageRequiresDirectRestore = aggregateEngageRequiresDirectRestore(
    normalizedSiteIds,
    selectedStatus,
  );
  // Restoring direct is only safe while the provider it falls back to can run
  // without the gateway; otherwise the user has to change or disable that site
  // first instead of us writing a config Codex cannot call.
  const engageBlockedByProtocol = engageRequiresDirectRestore && primaryNeedsProxy.needsProxy;
  const staleSiteIds = React.useMemo(() => {
    const addressable = new Set(candidateSiteIds);
    return siteIds.filter((siteId) => !addressable.has(siteId));
  }, [candidateSiteIds, siteIds]);
  const staleAliasSiteIds = React.useMemo(() => {
    const addressable = new Set(candidateSiteIds);
    return Object.keys(aliases).filter((siteId) => !addressable.has(siteId));
  }, [aliases, candidateSiteIds]);
  const hasStaleConfig = staleSiteIds.length > 0 || staleAliasSiteIds.length > 0;
  const normalizedAliases = normalizeGatewayAggregateAliases(
    aliases,
    siteIds,
    candidateSiteIds,
    separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR,
  );
  /**
   * The exposure block's candidate list: the backend's full bare-name universe,
    * falling back to published bare-name rows (promoted entries or hidden
    * aliases) when that field is absent.
   * Keeping the universe here is what lets a name be ticked back on after the
   * exposure set previously narrowed it out of the catalog.
   */
  const subagentExposureCandidates = React.useMemo(
    () => resolveSubagentExposureCandidates(subagentCatalog, normalizedSiteIds),
    [normalizedSiteIds, subagentCatalog],
  );
  // Only names the backend still knows about may be submitted: a stale tick for
  // a model no site declares would make the catalog name something the router
  // cannot resolve.
  const effectiveSubagentExposedModels = React.useMemo(() => {
    if (!subagentCatalog) {
      // Until the backend has supplied the addressable universe, no value is
      // safe to submit. This keeps a stale draft from crossing the API during
      // the catalog refresh window.
      return [];
    }
    return resolveEffectiveSubagentExposedModels(
      subagentExposedModels,
      subagentExposureCandidates.map((entry) => entry.model),
    );
  }, [subagentExposedModels, subagentCatalog, subagentExposureCandidates]);
  // Ticks that name a model the current site selection no longer declares. They
  // cannot be submitted (the catalog would name something the router rejects),
  // so they are surfaced instead of silently dropped.
  const staleSubagentExposedModels = React.useMemo(() => {
    if (!subagentCatalog) {
      return [];
    }
    return resolveStaleSubagentExposedModels(
      subagentExposedModels,
      subagentExposureCandidates.map((entry) => entry.model),
    );
  }, [subagentCatalog, subagentExposedModels, subagentExposureCandidates]);
  // The ticked names are what the subagent can see, so an engage needs at least
  // one and at most the five hint slots. Two states are incomplete: nothing
  // ticked (the backend would read the empty list as the legacy "promote
  // nothing" default), and more than the limit (only the first five would be
  // promoted, which the panel refuses instead of silently dropping the rest).
  const subagentExposureOverflow =
    effectiveSubagentExposedModels.length > SUBAGENT_EXPOSED_MODEL_LIMIT;
  const subagentExposureInvalid = !isSubagentExposureSelectionComplete(
    effectiveSubagentExposedModels,
  );
  /**
   * Candidates in render order: ticked names first, in tick order, so the panel
   * shows the priority the backend will apply. The rest keep the backend order.
   */
  const orderedSubagentExposureCandidates = React.useMemo(() => {
    const ticked = new Set(subagentExposedModels);
    const candidateModels = new Set(
      subagentExposureCandidates.map((entry) => entry.model),
    );
    // Keep unavailable ticks visible as removable rows; otherwise the warning
    // could block saving with no way for the user to explicitly clear them.
    const staleEntries = staleSubagentExposedModels
      .filter((model) => !candidateModels.has(model))
      .map((model) => ({ model, provider_id: '', provider_name: '' }));
    const allCandidates = [...subagentExposureCandidates, ...staleEntries];
    const selected = allCandidates.filter((entry) => ticked.has(entry.model));
    const rest = allCandidates.filter((entry) => !ticked.has(entry.model));
    return [
      ...selected.sort(
        (left, right) =>
          subagentExposedModels.indexOf(left.model) - subagentExposedModels.indexOf(right.model),
      ),
      ...rest,
    ];
  }, [subagentExposureCandidates, subagentExposedModels, staleSubagentExposedModels]);
  const canEngage =
    running &&
    normalizedSiteIds.length > 0 &&
    !hasStaleConfig &&
    staleSubagentExposedModels.length === 0 &&
    separatorError === null &&
    normalizedAliases !== null &&
    !subagentExposureInvalid &&
    !busy;

  const applyCliStatuses = React.useCallback(
    (statuses: GatewayCliTakeoverStatus[]) => {
      if (mountedRef.current) {
        setCliStatuses(statuses);
      }
    },
    [],
  );

  const refreshCliStatuses = React.useCallback(async () => {
    const request = statusRequestRef.current + 1;
    statusRequestRef.current = request;
    const statuses = await getProxyGatewayCliStatuses();
    if (mountedRef.current && statusRequestRef.current === request) {
      applyCliStatuses(statuses);
    }
    return statuses;
  }, [applyCliStatuses]);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      statusRequestRef.current += 1;
      mutationRevisionRef.current += 1;
      catalogRequestRef.current += 1;
    };
  }, []);

  // Seed the takeover state from the backend manifest so reopening the settings
  // page shows what is actually routing, not an empty form.
  React.useEffect(() => {
    void refreshCliStatuses().catch(() => {
      if (mountedRef.current) {
        applyCliStatuses([]);
      }
    });
  }, [applyCliStatuses, refreshCliStatuses]);

  // Another editor (for example provider-save re-engagement) may rewrite the
  // aggregate manifest while this drawer is mounted. Re-read canonical status
  // so this draft cannot overwrite a newer configuration.
  React.useEffect(() => {
    if (aggregateConfigVersion === 0) {
      return;
    }
    void refreshCliStatuses().catch(() => undefined);
  }, [aggregateConfigVersion, refreshCliStatuses]);
  React.useEffect(() => {
    let disposed = false;
    const request = revisionRef.current + 1;
    revisionRef.current = request;
    setLoadingSites(true);

    const load = async () => {
      try {
        // The draft is a convenience file: a failed read must not block the site
        // list, it only means the form falls back to the applied provider.
        const [next, draft] = await Promise.all([
          loadProviders(cliKey),
          getProxyGatewayAggregateDraft(cliKey).catch(() => null),
        ]);
        if (disposed || revisionRef.current !== request) {
          return;
        }
        setProviders(next.providers);
        setCandidates(next.candidates);
        savedDraftRef.current = draft;
        setDraftLoadRevision((revision) => revision + 1);
      } catch (error) {
        if (!disposed && revisionRef.current === request) {
          setProviders([]);
          setCandidates([]);
          savedDraftRef.current = null;
          setNotice({ kind: 'error', text: formatError(error) });
        }
      } finally {
        if (!disposed && revisionRef.current === request) {
          setLoadingSites(false);
        }
      }
    };

    void load();
    return () => {
      disposed = true;
    };
  }, [cliKey]);

  // Seed the form for the selected CLI. An engaged manifest wins because it is
  // what is actually routing (and keeps unavailable sites visible as an invalid
  // config instead of silently rewriting a running takeover); otherwise the
  // persisted draft is used, and an empty draft falls back to the CLI's
  // currently applied provider. Aggregate mode cannot represent "no site", so the
  // form is never seeded empty while the CLI has an eligible site.
  //
  // The seed deliberately keys off the manifest's *content* (`activeAggregateKey`)
  // rather than the status object identity: statuses are re-fetched on unrelated
  // refreshes, and re-seeding on those would revert edits the user just made.
  const activeAggregateKey = React.useMemo(
    () =>
      JSON.stringify(
        selectedStatus?.mode === 'aggregate' ? selectedStatus.aggregate ?? null : null,
      ),
    [selectedStatus],
  );
  React.useEffect(() => {
    const activeConfig =
      selectedStatus?.mode === 'aggregate' ? selectedStatus.aggregate ?? null : null;
    const seed = resolveAggregateFormSeed({
      activeConfig,
      draftConfig: savedDraftRef.current,
      candidates,
      appliedProviderId: providers.find((provider) => provider.isApplied)?.id ?? null,
    });
    setSeparator(seed.separator);
    setNaming(seed.naming);
    setAliases(seed.aliases);
    setSiteIds(seed.siteIds);
    setDroppedDraftSites(seed.droppedDraftSites);
    setCrossSiteFailover(seed.crossSiteFailover);
    // Tick order is priority order, and the stored array is what the backend
    // will promote. An empty value is the legacy "publish every bare model and
    // promote none" default: it is shown as-is and the block explains that a
    // new selection has to be made before the takeover may engage.
    setSubagentExposedModels(seed.subagentExposedModels);
    // The `[agents]` defaults are engage-time state, so they are only shown back
    // from a running takeover: seeding them from the draft would make keys this
    // takeover does not manage look managed.
    setSubagentModel(activeConfig?.subagent?.model ?? '');
    setSubagentReasoningEffort(activeConfig?.subagent?.reasoning_effort ?? '');
    // `selectedStatus` is intentionally absent: its identity changes on every
    // status refresh, while `activeAggregateKey` only changes when the engaged
    // configuration actually changed. Re-seeding on a refresh would revert the
    // user's in-progress edits.
  }, [activeAggregateKey, candidates, draftLoadRevision, providers]);

  const runGatewayOperation = React.useCallback(
    async <T,>(
      execute: () => Promise<T>,
      successText: string,
      /**
       * Resolved *after* the failure, because the same engage reports differently
       * depending on how far it got: a failed engage that already restored direct
       * mode leaves the CLI in a different state than one that failed outright.
       */
      failureKey: () => AggregateFailureNoticeKey,
    ): Promise<T | null> => {
      const request = mutationRevisionRef.current + 1;
      mutationRevisionRef.current = request;
      setBusy(true);
      setNotice(null);

      let result: T | null = null;
      try {
        await runGatewayAggregateMutation(async () => {
          try {
            const operationResult = await execute();
            // Refresh inside the lane so the next queued mutation cannot race
            // this operation's canonical status read.
            await refreshCliStatuses().catch(() => undefined);
            result = operationResult;
            return operationResult;
          } catch (error) {
            // Failed commands can still leave a partial restore or stale local
            // draft. Always re-read canonical status before the next mutation.
            await refreshCliStatuses().catch(() => undefined);
            throw error;
          }
        });
        notifyGatewayAggregateConfigChanged();
        void refreshTrayMenu().catch(() => undefined);
      } catch (error) {
        if (mountedRef.current && mutationRevisionRef.current === request) {
          const message = formatError(error);
          // A known backend rule is shown as a translated hint instead of the raw
          // English sentence; every other failure keeps the backend detail.
          const mappedNoticeKey = aggregateEngageErrorNoticeKey(message);
          setNotice({
            kind: 'error',
            text: mappedNoticeKey
              ? t(mappedNoticeKey)
              : t(`gateway.aggregate.notice.${failureKey()}`, { error: message }),
          });
        }
      } finally {
        // The parent owns provider locks and must refresh on both success and
        // failure; the child only lets the newest request update its notice.
        onTakeoverChange?.();
        if (mountedRef.current && mutationRevisionRef.current === request) {
          if (result !== null) {
            setNotice({ kind: 'success', text: successText });
          }
          setBusy(false);
        }
      }
      return result;
    },
    [onTakeoverChange, refreshCliStatuses, t],
  );

  const runEngage = React.useCallback(
    async (
      nextSiteIds: string[],
      nextSeparator: string,
      nextAliases: Record<string, string>,
      nextNaming: GatewayAggregateNamingMode,
      /**
       * Values the caller just changed. `React.setState` has not flushed yet
       * when an event handler engages, so anything derived from state would
       * write the previous value back. Omitted fields come from the form.
       */
      overrides?: { crossSiteFailover?: boolean; subagentExposedModels?: string[] },
    ) => {
      const nextCrossSiteFailover = overrides?.crossSiteFailover ?? crossSiteFailover;
      const requestedExposedModels =
        overrides?.subagentExposedModels ?? subagentExposedModels;
      const nextStaleExposedModels = subagentCatalog
        ? resolveStaleSubagentExposedModels(
            requestedExposedModels,
            subagentExposureCandidates.map((entry) => entry.model),
          )
        : [];
      if (nextStaleExposedModels.length > 0) {
        setNotice({
          kind: 'error',
          text: t('gateway.aggregate.subagentExposedStale', {
            models: nextStaleExposedModels.join(', '),
          }),
        });
        return false;
      }
      const nextExposedModels = subagentCatalog
        ? resolveEffectiveSubagentExposedModels(
            requestedExposedModels,
            subagentExposureCandidates.map((entry) => entry.model),
          )
        : normalizeSubagentExposedModels(requestedExposedModels);
      if (
        nextExposedModels.length === 0 ||
        nextExposedModels.length > SUBAGENT_EXPOSED_MODEL_LIMIT
      ) {
        setNotice({
          kind: 'error',
          text:
            nextExposedModels.length > SUBAGENT_EXPOSED_MODEL_LIMIT
              ? t('gateway.aggregate.subagentExposedLimit', {
                  limit: SUBAGENT_EXPOSED_MODEL_LIMIT,
                })
              : t('gateway.aggregate.subagentExposedRequired'),
        });
        return false;
      }
      const requiresDirectRestore = aggregateEngageRequiresDirectRestore(
        nextSiteIds,
        selectedStatus,
      );
      if (requiresDirectRestore && primaryNeedsProxy.needsProxy) {
        setNotice({ kind: 'error', text: restoreDirectBlockedHint });
        return false;
      }
      // Only set once the restore actually succeeded, so the failure notice can
      // tell "the CLI is back on direct now" apart from "nothing changed".
      let restoredDirect = false;
      const result = await runGatewayOperation(
        async () => {
          if (requiresDirectRestore) {
            // Switching the primary of an enabled takeover is only supported as
            // restore direct -> engage again; engaging straight away would be
            // rejected by the backend guard.
            await restoreProxyGatewayCliDirect(cliKey);
            restoredDirect = true;
          }
          return engageProxyGatewayAggregate(
            cliKey,
            nextSiteIds,
            nextSeparator,
            nextAliases,
            nextNaming,
            subagentModel,
            subagentReasoningEffort,
            nextCrossSiteFailover,
            nextExposedModels,
          );
        },
        t('gateway.aggregate.notice.enabled'),
        () => (restoredDirect ? 'enableFailedAfterRestore' : 'enableFailed'),
      );
      if (result) {
        const canonicalExposedModels = resolveCanonicalSubagentExposedModels(
          result as GatewayCliTakeoverStatus,
        );
        setSubagentExposedModels(canonicalExposedModels);
        // The backend persists the accepted selection to the draft file too; keep
        // the in-memory copy in step so leaving aggregate mode re-seeds this form
        // with the last engaged selection instead of an older draft.
        savedDraftRef.current = {
          provider_ids: [...nextSiteIds],
          separator: nextSeparator,
          aliases: { ...nextAliases },
          naming: nextNaming,
          cross_site_failover: nextCrossSiteFailover,
          subagent_exposed_models: [...canonicalExposedModels],
        };
      }
      return Boolean(result);
    },
    [
      cliKey,
      crossSiteFailover,
      primaryNeedsProxy.needsProxy,
      restoreDirectBlockedHint,
      runGatewayOperation,
      selectedStatus,
      subagentExposedModels,
      subagentCatalog,
      subagentExposureCandidates,
      subagentModel,
      subagentReasoningEffort,
      t,
    ],
  );

  /**
   * Persist the form as the aggregate draft, so the selection survives closing
   * this editor. Only used while the mode is not engaged: an engaged takeover
   * re-engages instead, because the manifest is what actually routes.
   */
  const persistAggregateDraft = React.useCallback(
    (next: {
      siteIds: string[];
      separator: string;
      aliases: Record<string, string>;
      naming: GatewayAggregateNamingMode;
      /** Same "just changed" rule as `runEngage`. */
      crossSiteFailover?: boolean;
      subagentExposedModels?: string[];
    }) => {
      const nextCrossSiteFailover = next.crossSiteFailover ?? crossSiteFailover;
      const requestedExposedModels = next.subagentExposedModels ?? subagentExposedModels;
      const request = draftRequestRef.current + 1;
      draftRequestRef.current = request;
      const staleExposedModels = subagentCatalog
        ? resolveStaleSubagentExposedModels(
            requestedExposedModels,
            subagentExposureCandidates.map((entry) => entry.model),
          )
        : [];
      if (staleExposedModels.length > 0) {
        setNotice({
          kind: 'error',
          text: t('gateway.aggregate.subagentExposedStale', {
            models: staleExposedModels.join(', '),
          }),
        });
        return;
      }
      const nextExposedModels = subagentCatalog
        ? resolveEffectiveSubagentExposedModels(
            requestedExposedModels,
            subagentExposureCandidates.map((entry) => entry.model),
        )
        : normalizeSubagentExposedModels(requestedExposedModels);
      // Serialized through the aggregate mutation lane so a slow, older write
      // cannot land after a newer one and resurrect a stale selection.
      void runGatewayAggregateMutation(() =>
        saveProxyGatewayAggregateDraft(
          cliKey,
          next.siteIds,
          next.separator,
          aliasesForSelectedSites(next.aliases, next.siteIds),
          next.naming,
          nextCrossSiteFailover,
          nextExposedModels,
        ),
      )
        .then((saved) => {
          if (mountedRef.current && draftRequestRef.current === request) {
            const canonicalExposedModels = resolveCanonicalSubagentExposedModels(saved);
            setSubagentExposedModels(canonicalExposedModels);
            savedDraftRef.current = {
              ...saved,
              subagent_exposed_models: canonicalExposedModels,
            };
            setDroppedDraftSites(false);
          }
        })
        .catch((error) => {
          if (mountedRef.current && draftRequestRef.current === request) {
            setNotice({
              kind: 'error',
              text: t('gateway.aggregate.notice.draftSaveFailed', { error: formatError(error) }),
            });
          }
        });
    },
    [
      cliKey,
      crossSiteFailover,
      subagentCatalog,
      subagentExposedModels,
      subagentExposureCandidates,
      t,
    ],
  );

  /**
   * Apply a new site selection. Engaged: re-engage so the running
   * takeover follows immediately. Not engaged: save the draft, which is what
   * makes the selection survive leaving this editor.
   *
   * The selection is always normalised to the CLI provider-list order before it
   * is stored, rendered or written: that list is the single source of the site
   * order, so the panel must never persist a second, panel-local ordering.
   */
  const applySiteSelection = (nextSiteIds: string[]) => {
    const orderedSiteIds = orderAggregateSiteIdsByCandidates(nextSiteIds, candidates);
    setSiteIds(orderedSiteIds);
    setDroppedDraftSites(false);
    // Aliases only address selected sites (the backend refuses anything else), so
    // deselecting a site drops its alias. Keeping it would leave the form
    // permanently invalid and disable the switch without any visible reason.
    setAliases((current) => aliasesForSelectedSites(current, orderedSiteIds));
    if (!engaged) {
      persistAggregateDraft({ siteIds: orderedSiteIds, separator, aliases, naming });
      return;
    }
    const nextAliases = normalizeGatewayAggregateAliases(
      aliases,
      orderedSiteIds,
      candidateSiteIds,
      separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR,
    );
    if (orderedSiteIds.length > 0 && separatorError === null && nextAliases) {
      void runEngage(orderedSiteIds, separator, nextAliases, naming);
    }
  };

  const handleToggle = async (checked: boolean) => {
    setNotice(null);
    if (!checked) {
      if (restoreDirectBlocked) {
        setNotice({ kind: 'error', text: restoreDirectBlockedHint });
        return;
      }
      await runGatewayOperation(
        () => restoreProxyGatewayCliDirect(cliKey),
        t('gateway.aggregate.notice.disabled'),
        () => 'disableFailed',
      );
      return;
    }

    if (hasStaleConfig) {
      setNotice({ kind: 'error', text: t('gateway.aggregate.notice.invalidConfig') });
      return;
    }
    if (normalizedSiteIds.length === 0) {
      setNotice({ kind: 'error', text: t('gateway.aggregate.sitesRequired') });
      return;
    }
    if (validateGatewayAggregateSeparator(separator) !== null) {
      setNotice({ kind: 'error', text: t('gateway.aggregate.notice.invalidConfig') });
      return;
    }
    if (!normalizedAliases) {
      setNotice({ kind: 'error', text: t('gateway.aggregate.aliasInvalid') });
      return;
    }
    if (staleSubagentExposedModels.length > 0) {
      setNotice({
        kind: 'error',
        text: t('gateway.aggregate.subagentExposedStale', {
          models: staleSubagentExposedModels.join(', '),
        }),
      });
      return;
    }
    // The takeover may not engage without a usable exposure selection: an empty
    // list would engage the legacy "promote nothing" default, so the subagent
    // would see no bare model at all — the exact problem this panel exists to
    // fix. Say why instead of letting the disabled switch look broken.
    if (subagentExposureInvalid) {
      setNotice({
        kind: 'error',
        text: subagentExposureOverflow
          ? t('gateway.aggregate.subagentExposedLimit', {
              limit: SUBAGENT_EXPOSED_MODEL_LIMIT,
            })
          : t('gateway.aggregate.subagentExposedRequired'),
      });
      return;
    }
    await runEngage(normalizedSiteIds, separator, normalizedAliases, naming);
  };

  const handleToggleSite = (siteId: string, checked: boolean) => {
    const nextSiteIds = checked
      ? normalizeGatewayAggregateSiteIds([...siteIds, siteId])
      : siteIds.filter((item) => item !== siteId);
    // Aggregate mode cannot represent an empty site list, so the last selected
    // site stays selected; leaving aggregate mode is the switch's job.
    if (nextSiteIds.length === 0) {
      setNotice({ kind: 'error', text: t('gateway.aggregate.lastSiteRequired') });
      return;
    }
    applySiteSelection(nextSiteIds);
  };

  const handleSeparatorCommit = () => {
    if (validateGatewayAggregateSeparator(separator) !== null) {
      return;
    }
    if (!engaged) {
      persistAggregateDraft({ siteIds, separator, aliases, naming });
      return;
    }
    if (siteIds.length > 0 && normalizedAliases) {
      void runEngage(siteIds, separator, normalizedAliases, naming);
    }
  };

  const handleAliasCommit = () => {
    if (siteIds.length === 0) {
      return;
    }
    const nextAliases = normalizeGatewayAggregateAliases(
      aliases,
      siteIds,
      candidateSiteIds,
      separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR,
    );
    if (!nextAliases) {
      setNotice({ kind: 'error', text: t('gateway.aggregate.aliasInvalid') });
      return;
    }
    if (!engaged) {
      persistAggregateDraft({ siteIds, separator, aliases, naming });
      return;
    }
    if (separatorError === null) {
      void runEngage(siteIds, separator, nextAliases, naming);
    }
  };

  const handleSelectAllSites = () => {
    if (candidates.length === 0) {
      return;
    }
    applySiteSelection(candidates.map((candidate) => candidate.id));
  };

  /**
   * Flip the cross-site failure policy. Engaged: re-engage so the running
   * takeover follows immediately. Not engaged: persist the draft, which is what
   * makes the choice survive leaving this editor.
   */
  const handleToggleCrossSiteFailover = (checked: boolean) => {
    setNotice(null);
    setCrossSiteFailover(checked);
    if (!engaged) {
      persistAggregateDraft({ siteIds, separator, aliases, naming, crossSiteFailover: checked });
      return;
    }
    if (siteIds.length > 0 && normalizedAliases && separatorError === null) {
      void runEngage(siteIds, separator, normalizedAliases, naming, {
        crossSiteFailover: checked,
      });
    }
  };

  // Render from the normalised selection so what the user sees is exactly the
  // order that will be sent to the backend.
  const selectedCandidates = normalizedSiteIds
    .map((siteId) => candidates.find((candidate) => candidate.id === siteId))
    .filter((candidate): candidate is GatewayAggregateSiteCandidate => Boolean(candidate));
  const unselectedCandidates = candidates.filter(
    (candidate) => !normalizedSiteIds.includes(candidate.id),
  );
  // Mirrors the backend `resolve_effective_site_aliases`: a site without an
  // explicit alias is addressed by its normalised display name, so the preview
  // shows `思源888 pro.<model>` rather than the opaque provider id.
  const effectiveAliases = React.useMemo(
    () =>
      resolveGatewayAggregateEffectiveAliases(
        aliases,
        selectedCandidates.map((candidate) => ({ id: candidate.id, name: candidate.name })),
        candidateSiteIds,
        separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR,
      ),
    [aliases, candidateSiteIds, selectedCandidates, separator, separatorError],
  );
  const separatorExample = buildGatewayAggregateSitePreviewSlug(
    candidates[0]?.id || 'site-id',
    separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR,
    naming,
    effectiveAliases,
  );
  const buildSiteRoutePreview = React.useCallback(
    (siteId: string) =>
      buildGatewayAggregateSitePreviewSlug(
        siteId,
        separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR,
        naming,
        effectiveAliases,
      ),
    [effectiveAliases, naming, separator, separatorError],
  );
  const invalidSiteIds = siteIds.filter((siteId) => !isAggregateSiteId(siteId));

  /** Read bare-name candidates for the selected sites; edits share the mutation lane. */
  const loadSubagentCatalog = React.useCallback(async (providerIds?: string[]) => {
    const request = catalogRequestRef.current + 1;
    catalogRequestRef.current = request;
    setSubagentCatalogLoading(true);
    setSubagentCatalog(null);
    try {
      const catalog = await runGatewayAggregateMutation(() =>
        getProxyGatewaySubagentCatalog(cliKey, providerIds),
      );
      if (mountedRef.current && catalogRequestRef.current === request) {
        setSubagentCatalog(catalog);
      }
    } catch (error) {
      if (mountedRef.current && catalogRequestRef.current === request) {
        setNotice({
          kind: 'error',
          text: t('gateway.aggregate.notice.subagentExposedLoadFailed', {
            error: formatError(error),
          }),
        });
      }
    } finally {
      if (mountedRef.current && catalogRequestRef.current === request) {
        setSubagentCatalogLoading(false);
      }
    }
  }, [cliKey, t]);

  // The block is editable before the takeover engages, so the candidate
  // universe is read for the current form selection. A fresh install has no
  // saved draft yet; pass its selected site IDs directly instead of showing an
  // empty list until the user first saves one.
  React.useEffect(() => {
    void loadSubagentCatalog(engaged ? undefined : normalizedSiteIds);
  }, [loadSubagentCatalog, normalizedSiteIds.join(','), engaged]);

  const handleToggleExposedModel = (model: string, checked: boolean) => {
    setNotice(null);
    if (checked && effectiveSubagentExposedModels.length >= SUBAGENT_EXPOSED_MODEL_LIMIT) {
      // Refusing is the point: Codex only shows the five lowest-priority visible
      // models, so a sixth tick would be silently invisible to the subagent even
      // though the panel looked like it had accepted it.
      setNotice({
        kind: 'error',
        text: t('gateway.aggregate.subagentExposedLimit', {
          limit: SUBAGENT_EXPOSED_MODEL_LIMIT,
        }),
      });
      return;
    }
    // Appending keeps tick order, which is the priority order the backend
    // applies; removing keeps the relative order of the rest.
    const nextModels = normalizeSubagentExposedModels(
      checked
        ? [...subagentExposedModels, model]
        : subagentExposedModels.filter((value) => value !== model),
    );
    if (engaged && nextModels.length === 0) {
      // Untick-last would write the legacy empty list, which the backend reads as
      // "promote nothing" — the opposite of what this panel is for. The mode is
      // kept unchanged so the visible draft still matches the active manifest.
      setNotice({ kind: 'error', text: t('gateway.aggregate.subagentExposedRequired') });
      return;
    }
    setSubagentExposedModels(nextModels);
    if (!engaged) {
      persistAggregateDraft({
        siteIds,
        separator,
        aliases,
        naming,
        subagentExposedModels: nextModels,
      });
      return;
    }
    if (separatorError === null && siteIds.length > 0 && normalizedAliases) {
      void runEngage(siteIds, separator, normalizedAliases, naming, {
        subagentExposedModels: nextModels,
      });
    }
  };

  return (
    <div className={styles.settings}>
      <div className={styles.toolbar}>
        <label className={styles.cliPicker}>
          <span>{t('gateway.aggregate.cliLabel')}</span>
          <select
            className={styles.select}
            value={cliKey}
            onChange={(event) => setCliKey(event.currentTarget.value as AggregateCliKey)}
          >
            {AGGREGATE_CLI_KEYS.map((option) => (
              <option key={option} value={option}>
                {t(`settings.gateway.cli.${option}`)}
              </option>
            ))}
          </select>
        </label>
        <div className={styles.toggle}>
          <span className={styles.state} role="status">
            {engaged ? t('gateway.aggregate.enabledLabel') : t('gateway.aggregate.disabledLabel')}
          </span>
          <Switch
            size="small"
            checked={engaged}
            disabled={
              busy ||
              (!engaged && !canEngage) ||
              (!engaged && !running) ||
              restoreDirectBlocked
            }
            loading={busy}
            title={restoreDirectBlocked ? restoreDirectBlockedHint : undefined}
            aria-label={
              engaged ? t('gateway.aggregate.disable') : t('gateway.aggregate.enable')
            }
            onChange={(checked) => {
              void handleToggle(checked);
            }}
          />
        </div>
      </div>

      <div className={styles.fieldRow}>
        <div className={styles.fieldMeta}>
          <span className={styles.fieldLabel}>{t('gateway.aggregate.naming')}</span>
          <span className={styles.fieldHelp}>{t('gateway.aggregate.namingHint')}</span>
        </div>
        <div className={styles.fieldControl}>
           <select
             className={styles.select}
             value={naming}
              disabled={busy}
              aria-label={t('gateway.aggregate.naming')}
             onChange={(event) => {
               const nextNaming = event.currentTarget.value as GatewayAggregateNamingMode;
               setNaming(nextNaming);
               if (!engaged) {
                 persistAggregateDraft({ siteIds, separator, aliases, naming: nextNaming });
               } else if (normalizedAliases && siteIds.length > 0) {
                 void runEngage(siteIds, separator, normalizedAliases, nextNaming);
               }
            }}
          >
            <option value="site_model">{t('gateway.aggregate.namingSiteModel')}</option>
            <option value="model_at_site">{t('gateway.aggregate.namingModelAtSite')}</option>
            <option value="model_only">{t('gateway.aggregate.namingModelOnly')}</option>
          </select>
        </div>
      </div>

       <p className={styles.helper}>{t('gateway.aggregate.modeHint')}</p>
      {droppedDraftSites ? (
        <p className={styles.helper}>{t('gateway.aggregate.draftSitesDropped')}</p>
      ) : null}
      {restoreDirectBlocked || engageBlockedByProtocol ? (
        <p className={styles.helper}>{restoreDirectBlockedHint}</p>
      ) : null}
      {!running ? <p className={styles.helper}>{t('gateway.aggregate.takeoverHint')}</p> : null}
      {/*
        The exposure block can block the takeover switch, and it sits further
        down the panel; repeating the reason here keeps the disabled switch from
        looking broken.
      */}
      {subagentExposureInvalid ? (
        <p className={styles.error} role="alert">
          {t('gateway.aggregate.subagentExposedRequired')}
        </p>
      ) : null}

      <div className={styles.fieldRow}>
        <div className={styles.fieldMeta}>
          <span className={styles.fieldLabel}>{t('gateway.aggregate.subagentModel')}</span>
          <span className={styles.fieldHelp}>{t('gateway.aggregate.subagentModelHint')}</span>
        </div>
        <div className={styles.fieldControl}>
          <input
            className={styles.separatorInput}
            value={subagentModel}
            disabled={busy}
            placeholder={t('gateway.aggregate.subagentModelPlaceholder')}
            aria-label={t('gateway.aggregate.subagentModel')}
            onChange={(event) => setSubagentModel(event.currentTarget.value)}
            onBlur={() => {
              if (engaged && normalizedAliases && siteIds.length > 0 && separatorError === null) {
                void runEngage(siteIds, separator, normalizedAliases, naming);
              }
            }}
          />
        </div>
      </div>

      <div className={styles.fieldRow}>
        <div className={styles.fieldMeta}>
          <span className={styles.fieldLabel}>{t('gateway.aggregate.subagentEffort')}</span>
          <span className={styles.fieldHelp}>{t('gateway.aggregate.subagentEffortHint')}</span>
        </div>
        <div className={styles.fieldControl}>
          <select
            className={styles.select}
            value={subagentReasoningEffort}
            disabled={busy}
            aria-label={t('gateway.aggregate.subagentEffort')}
            onChange={(event) => {
              const nextEffort = event.currentTarget.value;
              setSubagentReasoningEffort(nextEffort);
              if (engaged && normalizedAliases && siteIds.length > 0 && separatorError === null) {
                void runEngage(siteIds, separator, normalizedAliases, naming);
              }
            }}
          >
            <option value="">{t('gateway.aggregate.subagentEffortUnset')}</option>
            {['minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'].map((effort) => (
              <option key={effort} value={effort}>
                {effort}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className={styles.fieldRow}>
        <div className={styles.fieldMeta}>
          <span className={styles.fieldLabel}>{t('gateway.aggregate.crossSiteFailover')}</span>
          <span className={styles.fieldHelp}>{t('gateway.aggregate.crossSiteFailoverHint')}</span>
        </div>
        <div className={styles.fieldControl}>
          <Switch
            size="small"
            checked={crossSiteFailover}
            disabled={busy}
            aria-label={t('gateway.aggregate.crossSiteFailover')}
            onChange={(checked) => handleToggleCrossSiteFailover(checked)}
          />
        </div>
      </div>

      <div className={styles.fieldRow}>
        <div className={styles.fieldMeta}>
          <span className={styles.fieldLabel}>{t('gateway.aggregate.separator')}</span>
           <span className={styles.fieldHelp}>
             {t('gateway.aggregate.separatorHint', { example: separatorExample })}
           </span>
        </div>
        <div className={styles.fieldControl}>
          <input
             className={styles.separatorInput}
             value={separator}
              disabled={busy}
            placeholder={t('gateway.aggregate.separatorPlaceholder')}
            aria-label={t('gateway.aggregate.separator')}
            aria-invalid={separatorError !== null}
            onChange={(event) => setSeparator(event.currentTarget.value)}
            onBlur={handleSeparatorCommit}
          />
        </div>
      </div>
      {separatorError ? (
        <div className={styles.error} role="alert">
          {separatorError === 'empty'
            ? t('gateway.aggregate.separatorInvalidEmpty')
            : t('gateway.aggregate.separatorInvalidReserved')}
        </div>
      ) : null}

      {loadingSites ? (
        <div className={styles.loading}>
          <Loader2 size={14} className={styles.spin} aria-hidden="true" />
        </div>
      ) : candidates.length === 0 ? (
        <div className={styles.emptyState}>
          <span>{t('gateway.aggregate.noSites')}</span>
          <p>{t('gateway.aggregate.noSitesHint', { cli: t(`settings.gateway.cli.${cliKey}`) })}</p>
        </div>
      ) : (
        <>
           <div className={styles.listHeader}>
             <span className={styles.listTitle}>
               <Route size={12} aria-hidden="true" />
               {t('gateway.aggregate.selectedCount', { count: normalizedSiteIds.length })}
             </span>
             <span className={styles.listActions}>
                {selectedCandidates.length < candidates.length ? (
                  <button
                    type="button"
                    className={styles.textButton}
                    onClick={handleSelectAllSites}
                  >
                    {t('gateway.aggregate.selectAll')}
                  </button>
                ) : null}
              </span>
            </div>
            <p className={styles.helper}>{t('gateway.aggregate.orderHint')}</p>

            <ul className={styles.siteList}>
              {selectedCandidates.map((candidate) => (
                <SelectedSiteRow
                  key={candidate.id}
                  candidate={candidate}
                  canDeselect={selectedCandidates.length > 1}
                  routePreview={buildSiteRoutePreview(candidate.id)}
                  onToggleSite={handleToggleSite}
                  alias={aliases[candidate.id] ?? ''}
                  separator={separatorError === null ? separator : DEFAULT_AGGREGATE_SEPARATOR}
                  onAliasChange={(siteId, alias) => {
                    const nextAliases = { ...aliases, [siteId]: alias };
                    if (!alias.trim()) delete nextAliases[siteId];
                    setAliases(nextAliases);
                  }}
                  onAliasCommit={handleAliasCommit}
                />
              ))}
            </ul>

            {unselectedCandidates.length > 0 ? (
              <ul className={styles.siteList}>
                {unselectedCandidates.map((candidate) => (
                  <li key={candidate.id} className={styles.siteItem}>
                    <input
                      type="checkbox"
                      checked={false}
                      aria-label={candidate.name}
                      onChange={(event) =>
                        handleToggleSite(candidate.id, event.currentTarget.checked)
                      }
                    />
                    <span className={styles.siteName} title={candidate.name}>
                      {candidate.name}
                      <span className={styles.siteRoute}>
                        {t('gateway.aggregate.routePreview')}:{' '}
                        <code>{buildSiteRoutePreview(candidate.id)}</code>
                      </span>
                    </span>
                    <code className={styles.siteSlug} title={candidate.id}>
                      {shortAggregateSiteId(candidate.id)}
                    </code>
                  </li>
                ))}
              </ul>
            ) : null}
        </>
      )}

      {invalidSiteIds.length > 0 ? (
        <div className={styles.error} role="alert">
          {t('gateway.aggregate.notice.invalidConfig')}
        </div>
      ) : null}

      <details className={styles.subagentExposed} open>
        <summary className={styles.subagentExposedSummary}>
          <span>{t('gateway.aggregate.subagentExposedTitle')}</span>
          <span className={styles.fieldHelp}>
            {t('gateway.aggregate.subagentExposedCount', {
              count: effectiveSubagentExposedModels.length,
              limit: SUBAGENT_EXPOSED_MODEL_LIMIT,
            })}
          </span>
        </summary>
        <p className={styles.helper}>{t('gateway.aggregate.subagentExposedHint')}</p>
            {/* Tick order is priority order, and only the first `limit` ticks fit
                Codex's model hint. The banner states the budget before any tick. */}
            <p className={styles.helper}>
              {t('gateway.aggregate.subagentExposedOrderHint', {
                limit: SUBAGENT_EXPOSED_MODEL_LIMIT,
              })}
            </p>
            {subagentCatalogLoading ? (
              <div className={styles.loading}>
                <Loader2 size={14} className={styles.spin} aria-hidden="true" />
              </div>
            ) : orderedSubagentExposureCandidates.length === 0 ? (
              <p className={styles.helper}>{t('gateway.aggregate.subagentExposedEmpty')}</p>
            ) : (
              <ul className={styles.subagentExposedList}>
                {orderedSubagentExposureCandidates.map((entry) => {
                  const ticked = subagentExposedModels.includes(entry.model);
                  const effectiveIndex = effectiveSubagentExposedModels.indexOf(entry.model);
                  const promoted =
                    ticked &&
                    effectiveIndex >= 0 &&
                    effectiveIndex < SUBAGENT_EXPOSED_MODEL_LIMIT;
                  return (
                    <li key={entry.model} className={styles.subagentExposedItem}>
                      <label className={styles.subagentExposedOption}>
                        <input
                          type="checkbox"
                          checked={ticked}
                          disabled={
                            busy ||
                            (!ticked &&
                              effectiveSubagentExposedModels.length >=
                                SUBAGENT_EXPOSED_MODEL_LIMIT)
                          }
                          aria-label={entry.model}
                          onChange={(event) =>
                            handleToggleExposedModel(entry.model, event.currentTarget.checked)
                          }
                        />
                        <code className={styles.subagentExposedModel}>{entry.model}</code>
                        {/* Which names the subagent can actually pick vs. which
                            stay addressable only: the difference is invisible
                            from the Codex side, so the panel has to say it. */}
                        {effectiveIndex >= 0 ? (
                          <span
                            className={
                              promoted
                                ? styles.subagentExposedBadge
                                : styles.subagentExposedBadgeMuted
                            }
                          >
                            {promoted
                              ? t('gateway.aggregate.subagentExposedBadgePromoted')
                              : t('gateway.aggregate.subagentExposedBadgeAddressable')}
                          </span>
                        ) : null}
                        {entry.provider_name ? (
                          <span className={styles.subagentExposedProvider}>
                            {entry.provider_name}
                          </span>
                        ) : null}
                      </label>
                    </li>
                  );
                })}
              </ul>
            )}
            {/* A name the current site selection no longer declares cannot be
                published; say so instead of dropping it on save. */}
            {staleSubagentExposedModels.length > 0 ? (
              <p className={styles.error} role="alert">
                {t('gateway.aggregate.subagentExposedStale', {
                  models: staleSubagentExposedModels.join(', '),
                })}
              </p>
            ) : null}
            {subagentExposureInvalid ? (
              <p className={styles.error} role="alert">
                {subagentExposureOverflow
                  ? t('gateway.aggregate.subagentExposedLimit', {
                      limit: SUBAGENT_EXPOSED_MODEL_LIMIT,
                    })
                  : t('gateway.aggregate.subagentExposedRequired')}
              </p>
            ) : null}
      </details>

      {notice ? (
        <div
          className={notice.kind === 'error' ? styles.error : styles.success}
          role="status"
        >
          {notice.text}
        </div>
      ) : null}
    </div>
  );
};

export default GatewayAggregateSettings;
