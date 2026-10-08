import React from 'react';
import { App, Button, Space, Typography } from 'antd';
import { ImportOutlined } from '@ant-design/icons';
import { DndContext, closestCenter, type DragEndEvent } from '@dnd-kit/core';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { useTranslation } from 'react-i18next';
import ModelFormModal, { type ModelFormValues } from '@/components/common/ModelFormModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import ProviderConnectivityTestModal, {
  buildOmoNativeProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import {
  PROVIDER_SORT_MODES_BASIC,
  filterProviderItems,
  sortProviderItems,
  useProviderBatchSelection,
  useProviderListSort,
} from '@/features/coding/shared/providerList';
import {
  buildFavoriteProviderStorageKey,
  extractFavoriteProviderRawId,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  type OmoNativeFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import {
  upsertFavoriteProvider,
  type OpenCodeFavoriteProvider,
} from '@/services/opencodeApi';
import { buildOmoNativeFavoriteProviderConfig } from '../utils/omoNativeFavoriteProvider';
import {
  deleteOmoNativeProvider,
  reorderOmoNativeProviders,
  saveOmoNativeProvider,
} from '@/services/omoNativeApi';
import { refreshTrayMenu } from '@/services/appApi';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import type { OmoNativeProvider } from '@/types/omoNative';
import {
  OMO_NATIVE_API_OPTIONS,
  asRecord,
  buildFetchedOmoNativeModel,
  getNumberField,
  getOmoNativeModelEntries,
  getStringField,
  isRecordEmpty,
  omoNativeApiToNpm,
  parseInputTypes,
  parseJsonRecord,
  stringifyInputTypes,
  stringifyRecordField,
  type OmoNativeModelEntry,
} from '../utils/omoNativeProviders';
import { findPresetModelById } from '@/constants/presetModels';
import OmoNativeProviderCard from './OmoNativeProviderCard';
import OmoNativeProviderFormModal from './OmoNativeProviderFormModal';

const { Text } = Typography;

/** 可选字符串字段：有值写入，留空则删除该键（回落引擎默认）。 */
const setOptionalStringField = (
  target: Record<string, unknown>,
  key: string,
  value: unknown,
) => {
  if (typeof value === 'string' && value.trim()) {
    target[key] = value.trim();
  } else {
    delete target[key];
  }
};

/**
 * Native 的自定义 provider 存在引擎的 `models.json`（`providers` 键）里，密钥存在
 * `auth.json`。UI 与 OMP/Pi 同级：共享 `ProviderListSection` 外壳 + 共享
 * `OpenCodeStyleCard` 卡片 + 共享表单弹窗 + 共享连通性测试。
 *
 * `models.json` 没有对外发布的 schema，字段形状反推自上游 `convertProvider`：
 * provider 级 `name` / `baseUrl` / `api` / `headers`，模型级 `id` / `name` /
 * `reasoning` / `input` / `contextWindow` / `maxTokens`。写入按 provider key 局部更新，
 * 未知字段原样保留。
 */

interface ProviderModalState {
  provider?: OmoNativeProvider;
  copy?: boolean;
}

interface OmoNativeProvidersSectionProps {
  /**
   * provider 列表由**页面**持有：同一个列表「模型设置」区块的下拉也要用，
   * 两处各查一次会白跑一遍文件读取与内建名单拼装。
   */
  providers: OmoNativeProvider[];
  setProviders: React.Dispatch<React.SetStateAction<OmoNativeProvider[]>>;
  /** 从后端重读 provider 列表（各处的落盘动作都调它）。 */
  loadProviders: () => Promise<void>;
  /**
   * 「模型设置」里选中的默认渠道 key（`settings.json` 的 `defaultProvider`）。
   *
   * 它**不能删**：删掉之后引擎的 `defaultProvider` 就指向一个不存在的 provider。
   * 与 Pi 同规则——删除按钮保留但置灰，并排除出批量选择。
   */
  defaultProviderKey?: string;
}

const OmoNativeProvidersSection: React.FC<OmoNativeProvidersSectionProps> = ({
  providers,
  setProviders,
  loadProviders,
  defaultProviderKey,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();

  const [providerModal, setProviderModal] = React.useState<ProviderModalState | null>(null);
  const [modelModal, setModelModal] = React.useState<{
    provider: OmoNativeProvider;
    modelId?: string;
    /** Seed row for "add", used by copy. See `handleCopyModel`. */
    prefill?: ModelFormValues;
  } | null>(null);
  const [fetchModelsProvider, setFetchModelsProvider] = React.useState<OmoNativeProvider | null>(null);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<
    Record<string, ProviderConnectivityStatusItem>
  >({});
  const [batchTesting, setBatchTesting] = React.useState(false);
  /** 单个 provider 的连通性测试弹窗（共享 `ProviderConnectivityTestModal`）。 */
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(
    null,
  );
  const [providerKeyword, setProviderKeyword] = React.useState('');
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  /** 处于「模型批量删除」模式的 provider key（同一时刻只有一个）。 */
  const [modelBatchDeleteProviderKey, setModelBatchDeleteProviderKey] = React.useState<string | null>(
    null,
  );
  /** 各 provider 已勾选的模型 id。 */
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<
    Record<string, string[]>
  >({});

  // 内建 provider 由引擎自带，没有 models.json 条目可改；只有自定义的能编辑/删除。
  const customProviders = React.useMemo(
    () => providers.filter((provider) => provider.custom),
    [providers],
  );

  const { sortMode, setSortMode, lastUsedAt } = useProviderListSort('omo_native');
  const visibleProviders = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(customProviders, providerKeyword, (provider) => [
          provider.key,
          getStringField(provider.config, 'name'),
          getStringField(provider.config, 'baseUrl'),
          ...getOmoNativeModelEntries(provider.config).map((entry) => entry.id),
        ]),
        sortMode,
        { name: (provider) => getStringField(provider.config, 'name') || provider.key },
        (provider) => lastUsedAt(provider.key),
      ),
    [customProviders, providerKeyword, sortMode, lastUsedAt],
  );

  const openProviderModal = (provider?: OmoNativeProvider, options?: { copy?: boolean }) => {
    setProviderModal({ provider, copy: options?.copy === true });
  };

  // 搜索与非 `custom` 排序都会重排展示列表，此时拖拽写回的会是一个
  // 和用户眼前顺序不符的「自定义顺序」——只在 `custom` 模式下允许拖。
  const providerDragDisabled = sortMode !== 'custom' || providerKeyword.trim() !== '';

  const handleProviderDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const currentKeys = visibleProviders.map((provider) => provider.key);
    const oldIndex = currentKeys.indexOf(String(active.id));
    const newIndex = currentKeys.indexOf(String(over.id));
    if (oldIndex < 0 || newIndex < 0) return;

    const nextKeys = arrayMove(currentKeys, oldIndex, newIndex);
    // 乐观更新：先按新顺序重排本地列表，落盘后再从后端重读一次，
    // 失败也会收敛回已存的顺序。
    const byKey = new Map(visibleProviders.map((provider) => [provider.key, provider]));
    setProviders((previous) => [
      ...previous.filter((provider) => !provider.custom),
      ...nextKeys
        .map((key) => byKey.get(key))
        .filter((provider): provider is OmoNativeProvider => Boolean(provider)),
    ]);

    try {
      await reorderOmoNativeProviders(nextKeys);
    } catch (error) {
      console.error('Failed to reorder OmO Native providers:', error);
      message.error(t('common.error'));
    }
    await loadProviders();
  };

  /** 删除前先把 provider 存进收藏库，这样「导入我使用过的供应商」能把它恢复回来。 */
  const backUpProviderToFavorites = React.useCallback(async (provider: OmoNativeProvider) => {
    await upsertFavoriteProvider(
      buildFavoriteProviderStorageKey('omo_native', provider.key),
      buildOmoNativeFavoriteProviderConfig(provider),
    );
  }, []);

  const handleDeleteProvider = async (provider: OmoNativeProvider) => {
    try {
      await backUpProviderToFavorites(provider);
      await deleteOmoNativeProvider(provider.key, true);
      message.success(t('common.success'));
      await loadProviders();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to delete OmO Native provider:', error);
      message.error(t('common.error'));
    }
  };

  const handleBatchDeleteProviders = React.useCallback(
    async (keys: string[]): Promise<boolean> => {
      const targets = providers.filter((provider) => keys.includes(provider.key));
      if (targets.length === 0) return false;
      try {
        for (const provider of targets) {
          await backUpProviderToFavorites(provider);
          await deleteOmoNativeProvider(provider.key, true);
        }
        message.success(t('common.success'));
        await loadProviders();
        await refreshTrayMenu();
        return true;
      } catch (error) {
        console.error('Failed to batch delete OmO Native providers:', error);
        message.error(t('common.error'));
        return false;
      }
    },
    [providers, backUpProviderToFavorites, loadProviders, message, t],
  );

  /** 从收藏库导入：同名 key 已存在时跳过，避免覆盖用户当前的配置。 */
  const handleImportFavoriteProviders = async (favorites: OpenCodeFavoriteProvider[]) => {
    const existingKeys = new Set(customProviders.map((provider) => provider.key));
    let imported = 0;
    for (const favorite of favorites) {
      const payload = getFavoriteProviderPayload<OmoNativeFavoriteProviderPayload>(favorite);
      if (!payload) continue;
      const key = extractFavoriteProviderRawId('omo_native', favorite.providerId);
      if (!key || existingKeys.has(key)) continue;
      try {
        await saveOmoNativeProvider({ key, config: payload.config });
        imported += 1;
      } catch (error) {
        console.error('Failed to import OmO Native favorite provider:', error);
      }
    }
    if (imported > 0) {
      message.success(t('common.success'));
      setImportModalOpen(false);
    }
    await loadProviders();
  };

  /** 模型改动一律「读整份 provider 配置 → 改 models → 按 key 写回」，未知字段不动。 */
  const persistModels = async (
    provider: OmoNativeProvider,
    nextModels: Record<string, unknown>[],
  ) => {
    const nextConfig = { ...asRecord(provider.config) };
    if (nextModels.length > 0) {
      nextConfig.models = nextModels;
    } else {
      delete nextConfig.models;
    }
    await saveOmoNativeProvider({ key: provider.key, config: nextConfig });
    await loadProviders();
  };

  const handleSaveModel = async (values: ModelFormValues) => {
    if (!modelModal) return;
    const provider = modelModal.provider;
    const entries = getOmoNativeModelEntries(provider.config);
    // 未知字段原样保留：编辑一条模型不能顺手删掉引擎写的其它键。
    const existing = modelModal.modelId
      ? entries.find((entry) => entry.id === modelModal.modelId)?.model
      : undefined;
    const nextModel: Record<string, unknown> = { ...asRecord(existing), id: values.id };
    // 清空即删除：`name` / `api` 是可选覆盖项，留空应回落到引擎默认
    // （`name` 缺省 = id；`api` 缺省 = provider 的 `api`），不能保留旧值。
    setOptionalStringField(nextModel, 'name', values.name);
    setOptionalStringField(nextModel, 'api', values.api);
    if (typeof values.reasoning === 'boolean') nextModel.reasoning = values.reasoning;
    // `input` 在 `models.json` 里是**数组**（`["text","image"]`），而弹窗交回的是
    // JSON 字符串——必须 parse，不能直接赋值，否则写出引擎读不到的字符串。
    const inputTypes = parseInputTypes(values.inputTypes);
    if (inputTypes.length > 0) {
      nextModel.input = inputTypes;
    } else {
      delete nextModel.input;
    }
    if (typeof values.contextLimit === 'number') nextModel.contextWindow = values.contextLimit;
    if (typeof values.outputLimit === 'number') nextModel.maxTokens = values.outputLimit;
    // 引擎（senpi）的思考级别字段是 `thinkingLevelMap`，**没有** OMP 的 `thinking`
    // 结构——写 `thinking` 等于写一个没人读的键。
    const thinkingLevelMap = parseJsonRecord(values.thinkingLevelMap);
    if (!isRecordEmpty(thinkingLevelMap)) {
      nextModel.thinkingLevelMap = thinkingLevelMap;
    } else {
      delete nextModel.thinkingLevelMap;
    }
    const compat = parseJsonRecord(values.compat);
    if (!isRecordEmpty(compat)) {
      nextModel.compat = compat;
    } else {
      delete nextModel.compat;
    }
    // cost 的四个字段按「有值才写」处理：只填部分时不补 0，缺的键保持缺失
    // （「键缺失 = 让引擎用默认」与「显式 0 = 免费」语义不同）。
    const nextCost = asRecord(nextModel.cost);
    const costFields: Array<[string, number | undefined]> = [
      ['input', values.costInput],
      ['output', values.costOutput],
      ['cacheRead', values.costCacheRead],
      ['cacheWrite', values.costCacheWrite],
    ];
    costFields.forEach(([key, value]) => {
      if (typeof value === 'number' && Number.isFinite(value)) {
        nextCost[key] = value;
      } else {
        delete nextCost[key];
      }
    });
    if (!isRecordEmpty(nextCost)) {
      nextModel.cost = nextCost;
    } else {
      delete nextModel.cost;
    }

    const isEdit = Boolean(modelModal.modelId);
    if (!isEdit && entries.some((entry) => entry.id === values.id)) {
      message.error(t('omoNative.providers.modelIdExists'));
      return;
    }

    const nextEntries: OmoNativeModelEntry[] = entries.map((entry) =>
      isEdit && entry.id === modelModal.modelId ? { id: values.id, model: nextModel } : entry,
    );
    if (!isEdit) {
      nextEntries.push({ id: values.id, model: nextModel });
    }

    try {
      await persistModels(
        provider,
        nextEntries.map((entry) => entry.model),
      );
      message.success(t('common.success'));
      setModelModal(null);
    } catch (error) {
      console.error('Failed to save OmO Native model:', error);
      message.error(t('common.error'));
    }
  };

  /**
   * 复制一条模型：走**新增**流程并预填副本。
   *
   * `models.json` 按模型 `id` 索引目录，所以不能照抄 Codex 那种「同 id 加名字后缀」
   * ——那会造出重复 id。正确做法是打开新增弹窗让用户自己起新 id（与 ZCode 同）。
   */
  const handleCopyModel = (provider: OmoNativeProvider, modelId: string) => {
    const entry = getOmoNativeModelEntries(provider.config).find((item) => item.id === modelId);
    if (!entry) return;
    setModelModal({
      provider,
      modelId: undefined,
      prefill: {
        id: `${entry.id}-copy`,
        name: `${getStringField(entry.model, 'name') || entry.id} copy`,
        api: getStringField(entry.model, 'api') || undefined,
        reasoning:
          typeof entry.model.reasoning === 'boolean' ? entry.model.reasoning : undefined,
        inputTypes: stringifyInputTypes(entry.model.input),
        contextLimit: getNumberField(entry.model, 'contextWindow'),
        outputLimit: getNumberField(entry.model, 'maxTokens'),
        thinkingLevelMap: stringifyRecordField(entry.model.thinkingLevelMap),
        compat: stringifyRecordField(entry.model.compat),
        costInput: getNumberField(asRecord(entry.model.cost), 'input'),
        costOutput: getNumberField(asRecord(entry.model.cost), 'output'),
        costCacheRead: getNumberField(asRecord(entry.model.cost), 'cacheRead'),
        costCacheWrite: getNumberField(asRecord(entry.model.cost), 'cacheWrite'),
      },
    });
  };

  const handleDeleteModel = async (provider: OmoNativeProvider, modelId: string) => {
    const nextModels = getOmoNativeModelEntries(provider.config)
      .filter((entry) => entry.id !== modelId)
      .map((entry) => entry.model);
    try {
      await persistModels(provider, nextModels);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete OmO Native model:', error);
      message.error(t('common.error'));
    }
  };

  const handleReorderModels = async (provider: OmoNativeProvider, orderedIds: string[]) => {
    const entries = getOmoNativeModelEntries(provider.config);
    const byId = new Map(entries.map((entry) => [entry.id, entry]));
    const ordered = orderedIds
      .map((id) => byId.get(id))
      .filter((entry): entry is OmoNativeModelEntry => Boolean(entry));
    // 不在新顺序里的条目保持原相对位置，追加在后面。
    for (const entry of entries) {
      if (!ordered.includes(entry)) ordered.push(entry);
    }
    try {
      await persistModels(
        provider,
        ordered.map((entry) => entry.model),
      );
    } catch (error) {
      console.error('Failed to reorder OmO Native models:', error);
      message.error(t('common.error'));
    }
  };

    /** 进入/退出某 provider 的模型批量删除模式（同时只有一个 provider 处于该模式）。 */
  const handleToggleModelBatchDeleteMode = (provider: OmoNativeProvider) => {
    setSelectedModelIdsByProvider({});
    setModelBatchDeleteProviderKey((current) => (current === provider.key ? null : provider.key));
  };

  const handleToggleModelSelection = (provider: OmoNativeProvider, modelId: string, selected: boolean) => {
    setSelectedModelIdsByProvider((previous) => {
      const current = previous[provider.key] ?? [];
      const next = selected
        ? Array.from(new Set([...current, modelId]))
        : current.filter((id) => id !== modelId);
      if (next.length === 0) {
        const nextState = { ...previous };
        delete nextState[provider.key];
        return nextState;
      }
      return { ...previous, [provider.key]: next };
    });
  };

  const handleBatchDeleteModels = async (provider: OmoNativeProvider) => {
    const selectedIds = selectedModelIdsByProvider[provider.key] ?? [];
    if (selectedIds.length === 0) return;
    const removed = new Set(selectedIds);
    const nextModels = getOmoNativeModelEntries(provider.config)
      .filter((entry) => !removed.has(entry.id))
      .map((entry) => entry.model);
    try {
      await persistModels(provider, nextModels);
      message.success(t('common.success'));
      setSelectedModelIdsByProvider((previous) => {
        if (!(provider.key in previous)) return previous;
        const nextState = { ...previous };
        delete nextState[provider.key];
        return nextState;
      });
      setModelBatchDeleteProviderKey((current) => (current === provider.key ? null : current));
    } catch (error) {
      console.error('Failed to batch delete OmO Native models:', error);
      message.error(t('common.error'));
    }
  };

  const handleFetchModelsSuccess = async (result: FetchModelsApplyResult) => {
    if (!fetchModelsProvider) return;
    const provider = fetchModelsProvider;
    const providerApi = getStringField(provider.config, 'api');
    const entries = getOmoNativeModelEntries(provider.config);
    // `removedModelIds`：用户在弹窗里勾了「移除已不存在的模型」才非空。
    const removed = new Set(result.removedModelIds);
    const kept = entries.filter((entry) => !removed.has(entry.id));
    const existingIds = new Set(kept.map((entry) => entry.id));
    for (const model of result.selectedModels) {
      if (existingIds.has(model.id)) continue;
      // 用 model id 去预设表匹配，命中则自动补全上下文/输出/能力/思考级别
      // （SOP §4.2.5）。只写裸 id 的话用户点「应用」拿到的是一串空壳。
      const matchedPreset = findPresetModelById(model.id, omoNativeApiToNpm(providerApi));
      kept.push({ id: model.id, model: buildFetchedOmoNativeModel(model, matchedPreset) });
    }
    // `orderedModelIds`：**有意不采用**。本页的模型行支持手动拖拽排序
    // （`onReorderModels`），照弹窗顺序重排会覆盖用户自己排好的顺序；
    // 新增项追加在末尾。SOP §4.2.5 允许有手动排序的 CLI 这样权衡。
    try {
      await persistModels(
        provider,
        kept.map((entry) => entry.model),
      );
      message.success(t('common.success'));
      setFetchModelsProvider(null);
    } catch (error) {
      console.error('Failed to apply fetched models:', error);
      message.error(t('common.error'));
    }
  };

  const handleBatchTestProviders = async () => {
    // 单个测试与批量测试共用同一个 builder：此前批量那条独立拼装，
    // 既漏传 apiKey 又把语法标成 `omp`（应为 `omo`），实测必然 401。
    const eligible = visibleProviders
      .map((provider) => {
        const modelIds = getOmoNativeModelEntries(provider.config).map((entry) => entry.id);
        if (!getStringField(provider.config, 'baseUrl') || modelIds.length === 0) return null;
        return buildProviderConnectivityBatchTarget(
          buildOmoNativeProviderConnectivityInfo(provider),
          {
            requireBaseUrl: true,
            requireApiKey: false,
            errorMessages: {
              missingBaseUrl: t('common.baseUrlMissing'),
              missingApiKey: t('common.apiKeyMissing'),
              missingModel: t('common.modelMissing'),
            },
          },
        );
      })
      .filter((target): target is NonNullable<typeof target> => target !== null);

    if (eligible.length === 0) {
      message.warning(t('omoNative.providers.noTestableProvider'));
      return;
    }

    setConnectivityStatuses(
      Object.fromEntries(eligible.map((target) => [target.providerId, { status: 'running' as const }])),
    );
    setBatchTesting(true);
    try {
      await runProviderConnectivityBatch(eligible, (providerId, status) => {
        const nextStatus: ProviderConnectivityStatusItem =
          status.status === 'success'
            ? {
                ...status,
                tooltipMessage:
                  status.totalMs !== undefined
                    ? t('common.connectivityBatchSuccessWithTiming', {
                        model: status.modelId || t('common.notSet'),
                        totalMs: status.totalMs,
                      })
                    : t('common.connectivityBatchSuccess', {
                        model: status.modelId || t('common.notSet'),
                      }),
              }
            : status;
        setConnectivityStatuses((previous) => ({ ...previous, [providerId]: nextStatus }));
      });
    } catch (error) {
      console.error('Failed to batch test OmO Native providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTesting(false);
    }
  };

  /**
   * 单个 provider 的「模型测试」：打开共享的连通性测试弹窗。
   *
   * ⚠️ **不要退回 inline 状态徽标**。共享 `ProviderConnectivityTestModal` 是
   * 其余 10 个页面的标准做法（逐个模型列出结果、可移除失败项），本页此前是
   * 全仓仅有的两个例外之一（另一个是 ZCode），用户报「模型测试没弹窗」
   * （2026-10-07 修）。卡片头部的**批量**测试仍走 inline 徽标——那与 Codex 一致。
   */
  const handleTestProviderModels = (provider: OmoNativeProvider) => {
    setConnectivityInfo(buildOmoNativeProviderConnectivityInfo(provider));
    setConnectivityModalOpen(true);
  };

  /** 弹窗里勾选「移除失败的模型」后，从 `models.json` 删掉这些条目。 */
  const handleRemoveConnectivityModels = React.useCallback(
    async (modelIdsToRemove: string[]) => {
      const provider = providers.find((item) => item.key === connectivityInfo?.providerId);
      if (!provider || modelIdsToRemove.length === 0) return;
      const removed = new Set(modelIdsToRemove);
      const nextModels = getOmoNativeModelEntries(provider.config)
        .filter((entry) => !removed.has(entry.id))
        .map((entry) => entry.model);
      await persistModels(provider, nextModels);
    },
    [providers, connectivityInfo?.providerId],
  );

  // 默认渠道不进批量选择（与 Pi 一致）：只把卡片上的删除按钮置灰还不够，
  // 「全选 → 批量删除」同样能把它删掉，那是同一条规则的绕过路径。
  const batchSelectableIds = React.useMemo(
    () => visibleProviders
      .filter((provider) => provider.key !== defaultProviderKey)
      .map((provider) => provider.key),
    [visibleProviders, defaultProviderKey],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });

  const builtinCount = providers.length - customProviders.length;

  return (
    <>
      <ProviderListSection
        sectionId="omo-native-providers"
        collapsed={providerListCollapsed}
        onCollapsedChange={setProviderListCollapsed}
        providerCount={customProviders.length}
        visibleCount={visibleProviders.length}
        batch={providerBatch}
        batchSelectableIds={batchSelectableIds}
        keyword={providerKeyword}
        onKeywordChange={setProviderKeyword}
        sortMode={sortMode}
        sortModes={PROVIDER_SORT_MODES_BASIC}
        onSortModeChange={setSortMode}
        onBatchTest={handleBatchTestProviders}
        batchTesting={batchTesting}
        onAddProvider={() => openProviderModal()}
        headerExtra={
          builtinCount > 0 ? (
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t('omoNative.providers.builtinCount', { count: builtinCount })}
            </Text>
          ) : undefined
        }
        hint={
          <>
            {/* 页面级提示放这里（与 ZCode / Codex / ClaudeCode 同位置），
                不放在页头——`ProviderListSection` 负责它的样式。
                「内建 provider 不在 models.json 里重复定义」那句已删：内建渠道现在
                有自己的区块（且只列已配置的），列表下方又有「另有 N 个引擎内建」
                的计数，这句既重复又不再准确。 */}
            <div>{t('omoNative.pageHint')}</div>
            <div>{t('omoNative.providers.sectionHint')}</div>
          </>
        }
        footer={
          <Space wrap>
            <Button
              type="dashed"
              icon={<ImportOutlined />}
              onClick={() => setImportModalOpen(true)}
            >
              {t('opencode.provider.importFavorite')}
            </Button>
          </Space>
        }
      >
        {/* 卡片级拖拽：顺序存在 `models.json` 里 `providers` 的**键序**上
            （`reorder_omo_native_providers` 重写键序；读取端不再按 key 排序）。
            模型行的拖拽由 `ModelListSection` 自带的 context 处理，与这里无关。 */}
        <DndContext
          // 批量选择时卡片渲染的是复选框而不是把手，这里再关一次传感器，
          // 避免任何残留的拖拽手势。
          sensors={providerDragDisabled || providerBatch.selectionMode ? [] : undefined}
          collisionDetection={closestCenter}
          modifiers={[restrictToVerticalAxis]}
          onDragEnd={(event) => void handleProviderDragEnd(event)}
        >
          <SortableContext
            items={visibleProviders.map((provider) => provider.key)}
            strategy={verticalListSortingStrategy}
          >
            <div>
              {visibleProviders.map((provider) => (
            <OmoNativeProviderCard
              key={provider.key}
              provider={provider}
              onEdit={() => openProviderModal(provider)}
              onCopy={() => openProviderModal(provider, { copy: true })}
              onDelete={() => void handleDeleteProvider(provider)}
              deleteDisabledReason={
                provider.key === defaultProviderKey
                  ? t('omoNative.provider.deleteDisabledDefault')
                  : undefined
              }
              selectable={
                providerBatch.selectionMode && providerBatch.isSelectable(provider.key)
              }
              selected={providerBatch.selectedIds.has(provider.key)}
              onSelectChange={(selected) => providerBatch.toggleSelect(provider.key, selected)}
              connectivityStatus={connectivityStatuses[provider.key]}
              onAddModel={() => setModelModal({ provider })}
              onEditModel={(modelId) => setModelModal({ provider, modelId })}
              onCopyModel={(modelId) => handleCopyModel(provider, modelId)}
              onDeleteModel={(modelId) => void handleDeleteModel(provider, modelId)}
              onReorderModels={(orderedIds) => void handleReorderModels(provider, orderedIds)}
              onTestModels={() => handleTestProviderModels(provider)}
              testModelsDisabled={
                getOmoNativeModelEntries(provider.config).length === 0
              }
              testModelsDisabledTooltip={t('common.modelMissing')}
              onFetchModels={() => setFetchModelsProvider(provider)}
              dragDisabled={providerDragDisabled}
              modelSelectionMode={modelBatchDeleteProviderKey === provider.key}
              selectedModelIds={selectedModelIdsByProvider[provider.key] ?? []}
              onToggleModelSelection={(modelId, selected) =>
                handleToggleModelSelection(provider, modelId, selected)
              }
              onToggleBatchDeleteMode={() => handleToggleModelBatchDeleteMode(provider)}
              onBatchDeleteModels={() => void handleBatchDeleteModels(provider)}
            />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      </ProviderListSection>

      <ProviderConnectivityTestModal
        open={connectivityModalOpen}
        connectivityInfo={connectivityInfo}
        onRemoveModels={handleRemoveConnectivityModels}
        onCancel={() => setConnectivityModalOpen(false)}
      />

      <OmoNativeProviderFormModal
        open={Boolean(providerModal)}
        provider={providerModal?.provider}
        isCopy={providerModal?.copy === true}
        onCancel={() => setProviderModal(null)}
        onSaved={async () => {
          setProviderModal(null);
          await loadProviders();
          await refreshTrayMenu();
        }}
      />

      <ModelFormModal
        open={!!modelModal}
        width={700}
        isEdit={!!modelModal?.modelId}
        initialValues={
          modelModal?.prefill ??
          (modelModal?.modelId
            ? (() => {
                const entry = getOmoNativeModelEntries(modelModal.provider.config).find(
                  (item) => item.id === modelModal.modelId,
                );
                if (!entry) return undefined;
                return {
                  id: entry.id,
                  name: getStringField(entry.model, 'name'),
                  api: getStringField(entry.model, 'api'),
                  reasoning:
                    typeof entry.model.reasoning === 'boolean' ? entry.model.reasoning : undefined,
                  inputTypes: stringifyInputTypes(entry.model.input),
                  contextLimit: getNumberField(entry.model, 'contextWindow'),
                  outputLimit: getNumberField(entry.model, 'maxTokens'),
                  thinkingLevelMap: stringifyRecordField(entry.model.thinkingLevelMap),
                  compat: stringifyRecordField(entry.model.compat),
                  costInput: getNumberField(asRecord(entry.model.cost), 'input'),
                  costOutput: getNumberField(asRecord(entry.model.cost), 'output'),
                  costCacheRead: getNumberField(asRecord(entry.model.cost), 'cacheRead'),
                  costCacheWrite: getNumberField(asRecord(entry.model.cost), 'cacheWrite'),
                };
              })()
            : undefined)
        }
        existingIds={
          modelModal && !modelModal.modelId
            ? getOmoNativeModelEntries(modelModal.provider.config).map((entry) => entry.id)
            : []
        }
        showOptions={false}
        showVariants={false}
        showModalities={false}
        showInputTypes
        showApi
        apiOptions={OMO_NATIVE_API_OPTIONS}
        showReasoning
        // 引擎的思考级别字段是 `thinkingLevelMap`（上游 docs/models.md），
        // 不是 OMP 的 `thinking: { efforts, defaultLevel }`。
        showThinkingLevelMap
        // 与 Pi 同引擎同 schema：`compat` 与 `cost` 都是上游文档里的模型级字段。
        showCompat
        showCost
        limitRequired={false}
        nameRequired={false}
        npmType={omoNativeApiToNpm(
          getStringField(modelModal?.provider.config ?? {}, 'api'),
        )}
        toolName="OmO"
        onCancel={() => setModelModal(null)}
        onSuccess={handleSaveModel}
      />

      {fetchModelsProvider && (
        <FetchModelsModal
          open={!!fetchModelsProvider}
          providerId={fetchModelsProvider.key}
          providerName={
            getStringField(fetchModelsProvider.config, 'name') || fetchModelsProvider.key
          }
          baseUrl={getStringField(fetchModelsProvider.config, 'baseUrl')}
          // 密钥必须传：不传的话请求不带 Authorization，直接 401（2026-10-07 用户报的）。
          // 值来自列表接口回填的明文（`auth.json` 优先，其次 `models.json` 的 `apiKey`）。
          apiKey={fetchModelsProvider.apiKey}
          headers={asRecord(fetchModelsProvider.config.headers) as Record<string, string>}
          sdkType={omoNativeApiToNpm(getStringField(fetchModelsProvider.config, 'api'))}
          configValueMode="omo"
          existingModelIds={getOmoNativeModelEntries(fetchModelsProvider.config).map(
            (entry) => entry.id,
          )}
          onCancel={() => setFetchModelsProvider(null)}
          onSuccess={handleFetchModelsSuccess}
        />
      )}

      <ImportProviderModal
        open={importModalOpen}
        onClose={() => setImportModalOpen(false)}
        onImport={(imported) => void handleImportFavoriteProviders(imported)}
        existingProviderIds={customProviders.map((provider) =>
          buildFavoriteProviderStorageKey('omo_native', provider.key),
        )}
        providerFilter={(provider) => isFavoriteProviderForSource('omo_native', provider)}
      />
    </>
  );
};

export default OmoNativeProvidersSection;
