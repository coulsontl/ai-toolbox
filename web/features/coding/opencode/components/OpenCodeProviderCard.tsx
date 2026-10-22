import React from 'react';
import { Button, Modal, Space, Tag, Tooltip } from 'antd';
import { CopyOutlined, DeleteOutlined, EditOutlined } from '@ant-design/icons';
import { Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import { ProviderEnabledSwitch } from '@/features/coding/shared/providerCardVariants/providerCardActions';
import type {
  ProviderCardModels,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import type { OfficialModel } from '@/services/opencodeApi';
import type { OpenCodeModelReference } from '../utils/modelReference';
import {
  buildOpenCodeModelRows,
  buildOpenCodeProviderCardModel,
} from '../utils/providerCardMapping';
import type { OpenCodeProvider } from '@/types/opencode';
import OpenCodeOfficialModels from './OpenCodeOfficialModels';

export interface OpenCodeProviderCardProps {
  providerId: string;
  provider: OpenCodeProvider;
  /** Parsed `config.model`, used to mark the row that is the global default. */
  defaultModelReference?: OpenCodeModelReference;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
  /** Hides the drag handle while the list order cannot be changed. */
  dragDisabled?: boolean;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
  connectivityStatus?: ProviderConnectivityStatusItem;
  isDisabled: boolean;
  /** Receives the **new enabled state** (antd `Switch.onChange` contract). */
  onToggleDisabled: (enabled: boolean) => void;
  onEdit: () => void;
  onCopy: () => void;
  onShare: () => void;
  onDelete: () => void;
  deleteDisabledReason?: string;
  /** Official models from `auth.json` that belong to this channel (read-only). */
  officialModels?: OfficialModel[];

  onAddModel: () => void;
  onEditModel: (modelId: string) => void;
  onCopyModel: (modelId: string) => void;
  onDeleteModel: (modelId: string) => void;
  onSetPrimaryModel: (modelId: string) => void;
  onReorderModels: (orderedModelIds: string[]) => void;
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode: () => void;
  onBatchDeleteModels: () => void;

  onTestModels: () => void;
  testModelsDisabled: boolean;
  testModelsDisabledTooltip?: string;
  onFetchModels: () => void;
  fetchDisabled: boolean;
  fetchDisabledTooltip?: string;
}

/**
 * An OpenCode provider, rendered in the OpenCode style.
 *
 * OpenCode owns a model catalog per provider and has no provider-level "apply":
 * `config.model` points at one model inside one channel, and `设为默认` on the
 * model row is what moves it. That is exactly the shape `OpenCodeStyleCard`
 * encodes, so the layout lives there and this file only maps OpenCode's storage
 * (`provider.models`, `provider.options`, `disabled_providers`) onto it.
 *
 * The read-only official catalog merged from `auth.json` is not part of the
 * editable model list, so it travels in the style's `footer` slot (the
 * documented place for catalogs `ModelListSection` cannot host).
 */
const OpenCodeProviderCard: React.FC<OpenCodeProviderCardProps> = ({
  providerId,
  provider,
  defaultModelReference,
  viewMode = 'list',
  dragDisabled = false,
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  connectivityStatus,
  isDisabled,
  onToggleDisabled,
  onEdit,
  onCopy,
  onShare,
  onDelete,
  deleteDisabledReason,
  officialModels,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onSetPrimaryModel,
  onReorderModels,
  modelSelectionMode = false,
  selectedModelIds = [],
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  onTestModels,
  testModelsDisabled,
  testModelsDisabledTooltip,
  onFetchModels,
  fetchDisabled,
  fetchDisabledTooltip,
}) => {
  const { t } = useTranslation();

  const header = buildOpenCodeProviderCardModel(providerId, provider);
  const modelRows = React.useMemo<ModelDisplayData[]>(
    () => buildOpenCodeModelRows(providerId, provider, defaultModelReference),
    [providerId, provider, defaultModelReference],
  );

  /** Hoisted so the list card and the detail sidebar render the same catalog. */
  const modelSection: ProviderCardModels = {
    models: modelRows,
    modelsDraggable: !modelSelectionMode,
    onReorderModels,
    modelSelectionMode,
    selectedModelIds,
    onToggleModelSelection,
    onToggleBatchDeleteMode,
    onBatchDeleteModels,
    onTestModels,
    testModelsDisabled,
    testModelsDisabledTooltip,
    onFetchModels,
    fetchDisabled,
    fetchDisabledTooltip,
    onAddModel,
    onEditModel,
    onCopyModel,
    onDeleteModel,
    onSetPrimaryModel,
  };

  // The old card only expressed "disabled" through its switch; the shared card
  // draws the same switch in the header, so nothing else is added here (a
  // migration should not introduce new state furniture).
  const props: ProviderCardVariantProps = {
    provider: header,
    providerState: {
      isDisabled,
      onToggleDisabled,
      draggable: !selectable && !dragDisabled,
      sortableId: providerId,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
    },
    actions: {
      onEdit,
      onCopy,
      onShare,
      onDelete,
      // Same confirmation the old card had: delete removes the channel and every
      // model reference pointing into it.
      deleteConfirm: true,
      deleteDisabledReason,
    },
    modelSection,
    footer: <OpenCodeOfficialModels models={officialModels} />,
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  const defaultModelName = modelRows.find((model) => model.isPrimary)?.name;

  /**
   * The cell's second line: the provider's npm package (the AI SDK protocol it
   * talks, e.g. `@ai-sdk/openai-compatible`), then the one model the provider
   * starts with when it owns the default.
   *
   * A cell that owns no default model otherwise has an empty line under its
   * name, which reads as missing data rather than "no default here" — the npm
   * package is a fact every provider has, so the line always states something.
   * The list card shows the same package as an `SdkTag` on its second line.
   *
   * The package goes in **without a label**: `@scope/name` names itself, and a
   * `NPM 包:` prefix would eat the room the default model needs on the one
   * truncated line (user-directed, 2026-10-19).
   */
  const cardSubtitle = [
    { value: header.sdkName },
    ...(defaultModelName
      ? [{ label: t('common.model.currentPrimary'), value: defaultModelName }]
      : []),
  ];

  const detailSections: ProviderDetailSection[] = [
    {
      key: 'connection',
      title: t('common.provider.connectionTitle'),
      fields: [
        {
          key: 'baseUrl',
          label: t('opencode.provider.baseURL'),
          value: header.baseUrl,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
        },
        { key: 'npm', label: t('opencode.provider.npm'), value: header.sdkName, copyable: true },
        {
          key: 'apiKey',
          label: t('opencode.provider.apiKey'),
          value: provider.options?.apiKey,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
          elideMiddle: true,
        },
      ],
    },
    // The catalog brings its own header, toolbar and empty state.
    {
      key: 'catalog',
      content: (
        <ProviderModelSection
          providerId={providerId}
          modelSection={modelSection}
          appearance="detail"
        />
      ),
    },
  ];

  if (officialModels && officialModels.length > 0) {
    detailSections.push({
      key: 'officialModels',
      content: <OpenCodeOfficialModels models={officialModels} variant="detail" />,
    });
  }

  /**
   * The cell's delete button cannot open a Popconfirm (the sidebar footer is
   * not a Popconfirm host), so it asks first and then runs the same handler.
   * The default-model guard on this button mirrors the list card's disabled
   * state — without it the sidebar would become a way around
   * `canDeleteOpenCodeProvider`.
   */
  const handleDeleteFromDetail = () => {
    if (deleteDisabledReason) {
      return;
    }
    Modal.confirm({
      title: t('common.provider.deleteProvider'),
      content: t('common.provider.confirmDelete', { name: header.name }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: onDelete,
    });
  };

  return (
    <ProviderCompactCard
      provider={header}
      subtitle={cardSubtitle}
      // No provider-level apply exists in OpenCode, so the row's action slot
      // holds the one per-provider control there is — the same switch the list
      // card draws in its header (same reasoning as ZCode).
      actionPlacement="subtitle"
      cellControl={
        <ProviderEnabledSwitch isDisabled={isDisabled} onToggleDisabled={onToggleDisabled} />
      }
      providerState={props.providerState}
      actions={props.actions}
      extraMenuItems={[{
        key: 'model-test',
        label: t('opencode.connectivity.button'),
        disabled: testModelsDisabled,
        onClick: onTestModels,
      }]}
      detail={{
        title: <ProviderNameLink name={header.name} baseUrl={header.baseUrl} />,
        statusTags: isDisabled ? <Tag style={{ margin: 0 }}>{t('common.provider.disabled')}</Tag> : undefined,
        sections: detailSections,
        secondaryActions: (
          <Space size={4} wrap>
            <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
              {t('common.edit')}
            </Button>
            <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
              {t('common.copy')}
            </Button>
            <Button size="small" type="text" icon={<Share2 size={14} />} onClick={onShare}>
              {t('common.share')}
            </Button>
            <Tooltip title={deleteDisabledReason}>
              <span>
                <Button
                  size="small"
                  type="text"
                  danger
                  icon={<DeleteOutlined />}
                  disabled={Boolean(deleteDisabledReason)}
                  onClick={handleDeleteFromDetail}
                >
                  {t('common.delete')}
                </Button>
              </span>
            </Tooltip>
          </Space>
        ),
      }}
    />
  );
};

export default OpenCodeProviderCard;
