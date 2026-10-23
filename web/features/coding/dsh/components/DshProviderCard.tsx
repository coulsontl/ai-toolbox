import React from 'react';
import { Button, Space, Tag, Tooltip } from 'antd';
import { CopyOutlined, DeleteOutlined, EditOutlined, ShareAltOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import type {
  ProviderCardModels,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';
import type { DshRuntimeProviderView } from '@/types/dsh';
import { dshApiToSdkName } from '../utils/dshFetchedModels';
import { getDshModelRecords, getStringField } from '../utils/providerModels';
import { isCatalogOwnedRoute } from '../utils/providerConfig';

/** The backend only reports whether a credential ref exists, not its value. */
const MASKED_CREDENTIAL = '••••••••';

interface DshProviderCardProps {
  provider: DshRuntimeProviderView;
  /** The model `agent-default-model` starts sessions with, when this route owns it. */
  defaultModelId?: string;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
  connectivityStatus?: ProviderConnectivityStatusItem;
  /** Set while search or a non-`custom` sort makes the visible order not the stored one. */
  dragDisabled?: boolean;

  onEdit?: () => void;
  onCopy?: () => void;
  onShare?: () => void;
  onDelete?: () => void;
  deleteDisabledReason?: string;

  /** Model catalog actions. Omit a handler to hide its button. */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  onSetPrimaryModel?: (modelId: string) => void;
  onReorderModels?: (modelIds: string[]) => void;
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;

  /** Whole-catalog probes: connectivity and upstream model discovery. */
  onTestModels?: () => void;
  testModelsDisabled?: boolean;
  testModelsDisabledTooltip?: string;
  onFetchModels?: () => void;
  fetchDisabled?: boolean;
  fetchDisabledTooltip?: string;
}

/**
 * A dsh route, rendered in the OpenCode style.
 *
 * dsh keeps a per-route catalog under `llm-pi-ai.providers` and has no
 * provider-level "apply": `agent-default-model` names one model inside one
 * route, and `设为默认` on a model row moves that pointer. That is the OpenCode
 * shape.
 *
 * A built-in / catalog-backed route (`isBuiltin`, or `modelSource === 'builtin'`)
 * serves the adapter's bundled models and is not removable: it carries the
 * `内置 · 适配器默认模型` tag where the old card put it, and gets no delete
 * action at all rather than a dead button.
 */
const DshProviderCard: React.FC<DshProviderCardProps> = ({
  provider,
  defaultModelId,
  viewMode = 'list',
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  connectivityStatus,
  dragDisabled = false,
  onEdit,
  onCopy,
  onShare,
  onDelete,
  deleteDisabledReason,
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
  testModelsDisabled = false,
  testModelsDisabledTooltip,
  onFetchModels,
  fetchDisabled = false,
  fetchDisabledTooltip,
}) => {
  const { t } = useTranslation();
  const providerConfig = provider.provider ?? {};
  const baseUrl = getStringField(providerConfig, 'baseURL');
  const api = getStringField(providerConfig, 'api') || provider.api || '';
  // A catalog-backed route gets its name and protocol from the adapter, so the
  // second line says which env var holds the credential instead of pretending
  // there is a channel config to describe. The predicate is shared with the
  // editor modal and the page's save path — a card that keeps its own copy is
  // how "copy this route" once produced a route with no name or protocol.
  const isBuiltInChannel = isCatalogOwnedRoute(provider, false);

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () => getDshModelRecords(provider).map((entry) => ({
      id: entry.id,
      name: getStringField(entry.model, 'name') || entry.id,
      contextLimit: typeof entry.model.contextWindow === 'number' ? entry.model.contextWindow : undefined,
      outputLimit: typeof entry.model.maxTokens === 'number' ? entry.model.maxTokens : undefined,
      // dsh stores the pointer in `agent-default-model`, not on the route.
      isPrimary: Boolean(defaultModelId) && defaultModelId === entry.id,
    })),
    [provider, defaultModelId],
  );

  const modelSection: ProviderCardModels = {
    models: modelRows,
    // Only a route with its own config row has a stored position; a bundled
    // catalog has no `models` list to rewrite, so dragging it would write rows
    // into a file that never held them.
    modelsDraggable: !modelSelectionMode && !isBuiltInChannel,
    onReorderModels: isBuiltInChannel ? undefined : onReorderModels,
    onSetPrimaryModel,
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
  };

  const nameTags = (
    <>
      {provider.modelSource === 'builtin' && (
        <Tag style={{ margin: 0 }}>
          {t('dsh.model.modelSourceBuiltin', { defaultValue: '内置 · 适配器默认模型' })}
        </Tag>
      )}
      {/* The ref name is a fact for every route, including a catalog-backed
          one: it is how a user checks which credential an override shares
          before deleting either of them. */}
      {provider.apiKeyEnv && (
        <Tag style={{ margin: 0 }}>{provider.apiKeyEnv}</Tag>
      )}
    </>
  );

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.providerKey,
      name: provider.displayName,
      // `dshApiToSdkName` maps anything unknown to `@ai-sdk/openai-compatible`,
      // so calling it on an empty `api` would state a protocol the route never
      // declared. A route with no `api` says where its credential lives
      // instead — the fact a user actually needs before wiring a gateway.
      sdkName: api ? dshApiToSdkName(api) : (provider.apiKeyEnv || t('dsh.provider.builtinHint', { defaultValue: '内置供应商' })),
      baseUrl,
    },
    providerState: {
      draggable: !isBuiltInChannel && !selectable && !dragDisabled,
      sortableId: provider.providerKey,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
    },
    actions: {
      // A catalog-backed route is still editable: that is how a user pins an
      // explicit `models` list on top of the bundled catalog. Only *deletion*
      // is withheld (the page passes no handler), because the route is part of
      // the installed adapter rather than a user-created entry.
      onEdit,
      onCopy,
      onShare,
      onDelete,
      deleteConfirm: false,
      deleteDisabledReason,
    },
    nameTags,
    modelSection,
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  const defaultModelName = defaultModelId
    ? modelRows.find((row) => row.id === defaultModelId)?.name ?? defaultModelId
    : undefined;

  // The grid cell only carries "more"; the sidebar carries the rest so it can
  // do everything the list row can. A catalog-backed route passes no edit /
  // copy / share / delete handler at all, so those buttons are skipped rather
  // than drawn dead.
  const secondaryActions = (
    <Space size={4} wrap>
      {onEdit && (
        <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
          {t('common.edit')}
        </Button>
      )}
      {onCopy && (
        <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
          {t('common.copy')}
        </Button>
      )}
      {onShare && (
        <Button size="small" type="text" icon={<ShareAltOutlined />} onClick={onShare}>
          {t('common.share')}
        </Button>
      )}
      {onDelete && (
        <Tooltip title={deleteDisabledReason}>
          <span>
            <Button
              size="small"
              type="text"
              danger
              icon={<DeleteOutlined />}
              disabled={Boolean(deleteDisabledReason)}
              onClick={() => {
                if (!deleteDisabledReason) {
                  onDelete();
                }
              }}
            >
              {t('common.delete')}
            </Button>
          </span>
        </Tooltip>
      )}
    </Space>
  );

  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('dsh.provider.baseUrl', { defaultValue: 'Base URL' }),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      {
        key: 'apiType',
        label: t('dsh.provider.apiType', { defaultValue: 'API 类型' }),
        value: api,
        emptyText: t('common.provider.notConfigured'),
      },
      // Only a custom route owns a credential ref; a catalog route's auth is
      // resolved inside the adapter.
      ...(isBuiltInChannel ? [] : [{
        key: 'credential',
        label: t('dsh.provider.providerApiKey', { defaultValue: 'API Key' }),
        value: provider.credentialExists ? MASKED_CREDENTIAL : '',
        emptyText: t('common.provider.notConfigured'),
      }]),
    ],
  }, {
    key: 'catalog',
    content: (
      <ProviderModelSection
        providerId={provider.providerKey}
        modelSection={modelSection}
        appearance="detail"
      />
    ),
  }];

  return (
    <ProviderCompactCard
      provider={props.provider}
      subtitle={[
        { value: props.provider.sdkName ?? 'dsh' },
        ...(defaultModelName
          ? [{ label: t('common.model.currentPrimary'), value: defaultModelName }]
          : []),
      ]}
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      nameTags={nameTags}
      extraMenuItems={onTestModels ? [{
        key: 'model-test',
        label: t('dsh.connectivity.button', { defaultValue: '连通性测试' }),
        disabled: testModelsDisabled,
        onClick: onTestModels,
      }] : undefined}
      detail={{
        title: <ProviderNameLink name={props.provider.name} baseUrl={baseUrl} />,
        statusTags: nameTags,
        sections: detailSections,
        secondaryActions,
      }}
    />
  );
};

export default DshProviderCard;
