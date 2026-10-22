import type { TFunction } from 'i18next';

/**
 * Why a provider row cannot be batch-selected — one sentence per cause, so a
 * disabled checkbox can say *why* instead of leaving the slot blank.
 *
 * The wording lives in `common.batch.selectDisabled*`: the causes are the same
 * across coding tabs (default provider, local bridge record, official channel,
 * provider that still owns official accounts, engine-owned read-only entry), and
 * a per-CLI copy of the same sentence is how the three spellings of "default"
 * happened in the first place.
 *
 * Only the *classification* stays per CLI, because only the CLI knows which
 * flag means "default" or "local bridge" in its own record shape.
 */
export type ProviderSelectionBlockReason =
  | 'default'
  | 'local'
  | 'official'
  | 'officialAccount'
  | 'readOnly'
  | 'noRecord'
  | 'unnamed';

const REASON_KEYS: Record<ProviderSelectionBlockReason, string> = {
  default: 'common.batch.selectDisabledDefault',
  local: 'common.batch.selectDisabledLocal',
  official: 'common.batch.selectDisabledOfficial',
  officialAccount: 'common.batch.selectDisabledOfficialAccount',
  readOnly: 'common.batch.selectDisabledReadOnly',
  noRecord: 'common.batch.selectDisabledNoRecord',
  unnamed: 'common.batch.selectDisabledUnnamed',
};

/** The translated sentence for a block reason. */
export const providerSelectionDisabledReason = (
  t: TFunction,
  reason: ProviderSelectionBlockReason | undefined,
): string | undefined => (reason ? t(REASON_KEYS[reason]) : undefined);
