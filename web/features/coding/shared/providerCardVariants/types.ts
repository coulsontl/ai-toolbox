import type React from 'react';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';

/**
 * The data every provider card style needs, regardless of how it lays it out.
 *
 * Each CLI maps its own provider record into this shape, so the three styles
 * stay presentation-only and never reach into a CLI's storage format.
 */
export interface ProviderCardModel {
  id: string;
  name: string;
  /** `@ai-sdk/...` family, shown as a tag. Empty hides the tag. */
  sdkName?: string;
  baseUrl?: string;
  /**
   * Free-form facts, rendered in the order given.
   *
   * The three styles differ in *where* these go, not in what they carry, so the
   * caller decides the content and the style decides the placement. A `code`
   * entry renders monospaced (ids, model names); a `text` entry renders
   * secondary (labels, notes); a `tag` entry renders as a coloured Tag.
   */
  meta?: ProviderCardMetaEntry[];
}

export interface ProviderCardMetaEntry {
  /**
   * Only the kinds a style actually renders are listed. `id` and `sdk` were
   * removed on 2026-10-07: nothing produced them (the OpenCode style builds its
   * own id/SDK line from `provider.id` / `sdkName`), so they were a contract
   * with no reader.
   */
  kind: 'code' | 'text' | 'tag';
  value: string;
  /**
   * Optional label rendered immediately before the value, in secondary text:
   * `Haiku: claude-haiku-4-5`.
   *
   * A label belongs to its value, so it travels in the same entry. Modelling it
   * as a second entry looks identical in the data but renders wrong: the row's
   * inter-entry gap lands between the label and its value, and the two boxes
   * align by their tops instead of sharing a baseline — which is exactly the
   * misalignment that showed up on the Claude Code card.
   */
  label?: string;
  /** Only meaningful for `tag`: the Ant Design color. */
  color?: string;
}

/**
 * Actions rendered in the card header, in this order. Omit one to hide it.
 *
 * The styles place these differently — the OpenCode style renders them as
 * icon buttons with the toggle first, the Claude/Codex styles render the
 * primary actions as text links and fold the rest into a "more" menu — but
 * the set is the same, so a CLI wires its handlers once.
 */
export interface ProviderCardActions {
  onEdit?: () => void;
  onCopy?: () => void;
  onShare?: () => void;
  onDelete?: () => void;
  /** Disables the delete button and explains why on hover. */
  deleteDisabledReason?: string;
  /** Wraps delete in a Popconfirm; the OpenCode style relies on the caller's own confirm. */
  deleteConfirm?: boolean;
  /**
   * Sub-label under the enable switch in the "more" menu, describing the
   * current state. The Claude and Codex styles render it; without it the menu
   * shows the bare switch.
   *
   * Per-CLI because the wording is: Claude Code and Codex say "配置已启用" /
   * "配置已禁用", which is not the generic `common.provider.enabled`.
   */
  enabledStateLabel?: string;
  /**
   * Primary action, rendered as a text link ("应用" / "设为默认") in the Claude
   * and Codex styles. The OpenCode style has no header-level primary action:
   * its equivalent lives on the model row, so it ignores this.
   */
  primaryAction?: {
    label: string;
    icon?: React.ReactNode;
    onClick: () => void;
    disabled?: boolean;
    loading?: boolean;
    tooltip?: string;
    /** A disabled primary action with a tooltip still renders, greyed out. */
    locked?: boolean;
  };
  /**
   * Gateway takeover actions (代理 / 恢复直连 / 切换主渠道), rendered **before**
   * `primaryAction` so the row reads "take over" → "apply" → "edit".
   *
   * A slot rather than a data prop: the four actions are conditional on the
   * gateway's own status shape, and each caller's handlers already read that
   * status itself. The card has nothing to add — it only has to put the node in
   * the right place, which is the part that used to drift.
   *
   * Rendered by the Claude and Codex styles (the CLIs the gateway can take
   * over). The OpenCode style has no header action rail and ignores it.
   */
  gatewayActions?: React.ReactNode;
  /**
   * Extra header actions for the OpenCode style (batch delete, connectivity).
   *
   * In the Claude and Codex styles these render as icon buttons **before** the
   * "more" menu — the place the bespoke cards put their tool-specific header
   * actions.
   */
  extraActions?: React.ReactNode;
}

export interface ProviderCardState {
  /** The provider is disabled (only meaningful with `onToggleDisabled`). */
  isDisabled?: boolean;
  /**
   * Called with the **new enabled state** when the switch is flipped — the same
   * contract antd's `Switch.onChange` uses.
   *
   * It deliberately does not take zero arguments: a no-arg "please flip it"
   * callback forces every caller to recompute the next value from the current
   * one, and a caller that inverts it twice (`!provider.isDisabled` fed into a
   * handler that inverts again) writes back the value it already had — the
   * switch then looks alive but does nothing, with no type error. Passing the
   * value through keeps the mapper a pass-through, exactly like the bespoke
   * cards did.
   */
  onToggleDisabled?: (enabled: boolean) => void;
  connectivityStatus?: ProviderConnectivityStatusItem;
  /** Batch-selection mode: the drag handle is replaced by a checkbox. */
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
  /** Renders the card at reduced opacity, e.g. while a disabled provider is listed. */
  dimmed?: boolean;
  /**
   * Card-level drag handle.
   *
   * Lives here, not on `modelSection`, because it orders the **cards**, not the
   * model rows — a Claude-style card has no model section and still needs to be
   * draggable. Nesting it under `modelSection` made the handle vanish for
   * exactly those CLIs: the prop was never read, and nothing failed.
   */
  draggable?: boolean;
  sortableId?: string;
  /**
   * Chrome accent, forwarded to `CardShell`. See that component for the
   * precedence rules (batch selection > gateway primary > applied).
   */
  accent?: 'applied' | 'gatewayPrimary';
}

export interface ProviderCardModels {
  models: ModelDisplayData[];
  /**
   * Row identity for selection / reorder / drag. Defaults to `model.id`.
   *
   * Providers whose rows are keyed by more than the model id (Codex keys on
   * `model + displayName`) pass a resolver so callbacks receive their own key.
   * See `ModelListSection` for why it must not be re-derived from the display
   * object.
   */
  rowKeyOf?: (model: ModelDisplayData) => string;
  /** Model-row actions. Omit one to hide its button. */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  onSetPrimaryModel?: (modelId: string) => void;
  /** Flips a model's enabled flag; omit for CLIs that cannot disable one model. */
  onToggleModelDisabled?: (modelId: string, isDisabled: boolean) => void;
  onReorderModels?: (orderedModelIds: string[]) => void;
  modelsDraggable?: boolean;
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;
  onTestModels?: () => void;
  testModelsDisabled?: boolean;
  testModelsDisabledTooltip?: string;
  onFetchModels?: () => void;
  fetchDisabled?: boolean;
  fetchDisabledTooltip?: string;
  /** Per-row extra action (Codex's "设为自动审批模型"). */
  renderModelExtraActions?: (model: ModelDisplayData, rowKey: string) => React.ReactNode;
  /** Rendered directly under the toolbar, above the rows (Codex's auto-review line). */
  aboveList?: React.ReactNode;
  /** Extra class on the model Collapse, e.g. to scope a style override. */
  className?: string;
  /** Style applied to the content wrapper inside the model Collapse body. */
  bodyStyle?: React.CSSProperties;
}

/**
 * What every style receives.
 *
 * Deliberately one flat interface rather than one per style: the styles exist
 * to fix *layout*, and a caller that has to pick a different prop shape per
 * style would be re-implementing the divergence this module removes. A style
 * ignores what it does not render.
 */
export interface ProviderCardVariantProps {
  provider: ProviderCardModel;
  providerState?: ProviderCardState;
  actions?: ProviderCardActions;
  modelSection?: ProviderCardModels;
  /**
   * Rendered immediately **before** the provider name.
   *
   * `nameTags` covers everything after the name, but nothing could reach in
   * front of it, because the name itself comes from `provider.name` and cannot
   * carry markup. The official-account card needs that spot: its name row is
   * the heading of an account block, and such a heading reads
   * `🔗 <name> (n)` — glyph before, count after.
   */
  namePrefix?: React.ReactNode;
  /** Tags rendered beside the provider name (applied / disabled / official…). */
  nameTags?: React.ReactNode;
  /**
   * Free-form second line, in the order given.
   *
   * The Codex and Claude styles use this for whatever facts the CLI wants to
   * show — endpoint, model, masked key, notes, role bindings. The OpenCode
   * style ignores it and builds the line from `provider.id` / `sdkName` /
   * `baseUrl` instead, which is what makes that style's second line uniform.
   */
  metaEntries?: ProviderCardMetaEntry[];
  /** Actions rendered at the end of the meta line (e.g. an inline connectivity button). */
  inlineActions?: React.ReactNode;
  /** Free-form block below the meta line, before the model section. */
  footer?: React.ReactNode;
}
