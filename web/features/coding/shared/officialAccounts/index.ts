/**
 * The official-account section, shared by every CLI that has an official login.
 *
 * A CLI reuses it by mapping its own account records onto
 * `OfficialAccountRowView` and choosing a host: `embedded` when it has an
 * official provider row to sit inside (Codex, Kimi), `standalone` when the
 * official login is not a provider at all (ZCode).
 */
export {
  default as OfficialAccountsSection,
  OfficialAccountCount,
  OfficialAccountHeadingIcon,
  OfficialAccountHint,
} from './OfficialAccountsSection';
export type {
  OfficialAccountAction,
  OfficialAccountPendingAction,
  OfficialAccountRowView,
  OfficialAccountsSectionProps,
  OfficialAccountsSectionVariant,
} from './types';