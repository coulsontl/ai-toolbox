import React from 'react';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import {
  App,
  Button,
  Checkbox,
  Dropdown,
  Empty,
  Image,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Space,
  Spin,
  Tag,
  Typography,
  Upload,
} from 'antd';
import type { MenuProps } from 'antd';
import { ExclamationCircleOutlined } from '@ant-design/icons';
import { convertFileSrc } from '@tauri-apps/api/core';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import { copyFile } from '@tauri-apps/plugin-fs';
import {
  Copy,
  ChevronDown,
  Download,
  FileJson,
  GripVertical,
  History,
  Image as ImageIcon,
  Palette,
  Pencil,
  Plus,
  RefreshCcw,
  RotateCcw,
  Route,
  Sparkles,
  Trash2,
  Upload as UploadIcon,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import type {
  CreateImageJobInput,
  ImageChannel,
  ImageChannelModel,
  UpsertImageChannelInput,
} from '../services/imageApi';
import ImageChannelModal from '../components/ImageChannelModal';
import SizePickerModal from '../components/SizePickerModal';
import { useImage } from '../hooks/useImage';
import {
  filterHistoryJobParamsByModel,
  getImageParameterVisibility,
  parseHistoryJobParams,
} from '../utils/modelProfile';
import { normalizeImageSize } from '../utils/sizeUtils';
import styles from './ImagePage.module.less';

const { Title, Text } = Typography;

type ImageModeKey = 'text_to_image' | 'image_to_image';

interface LocalReferenceImage {
  id: string;
  fileName: string;
  mimeType: string;
  base64Data: string;
  previewUrl: string;
}

interface FormState {
  prompt: string;
  mode: ImageModeKey;
  modelId: string;
  channelId: string;
  size: string;
  quality: string;
  outputFormat: string;
  outputCompression: number | null;
  moderation: string;
}

interface WorkbenchChannelOption {
  id: string;
  name: string;
  sortOrder: number;
}

interface WorkbenchModelOption {
  id: string;
  label: string;
  supportsTextToImage: boolean;
  supportsImageToImage: boolean;
  availableChannels: WorkbenchChannelOption[];
}

interface ChannelDraft {
  id?: string | null;
  name: string;
  provider_kind: 'openai_compatible';
  base_url: string;
  api_key: string;
  generation_path?: string | null;
  edit_path?: string | null;
  timeout_seconds?: number | null;
  enabled: boolean;
  models: ImageChannelModel[];
}

interface ResultImageViewModel {
  id: string;
  job_id?: string | null;
  role: string;
  mime_type: string;
  file_name: string;
  relative_path: string;
  bytes: number;
  width?: number | null;
  height?: number | null;
  created_at: number;
  file_path: string;
  previewUrl: string;
  dimensionLabel: string | null;
}

const MODE_KEYS: ImageModeKey[] = ['text_to_image', 'image_to_image'];

const QUALITY_OPTIONS = [
  { value: 'auto', label: 'Auto' },
  { value: 'high', label: 'High' },
  { value: 'medium', label: 'Medium' },
  { value: 'low', label: 'Low' },
];

const FORMAT_OPTIONS = [
  { value: 'png', label: 'PNG' },
  { value: 'jpeg', label: 'JPEG' },
  { value: 'webp', label: 'WebP' },
];

const MODERATION_OPTIONS = [
  { value: 'low', label: 'Low' },
  { value: 'auto', label: 'Auto' },
];

const PROVIDER_KIND_OPTIONS = [
  { value: 'openai_compatible', label: 'OpenAI Compatible' },
];

const MAX_REFERENCE_COUNT = 16;

const createDefaultFormState = (): FormState => ({
  prompt: '',
  mode: 'text_to_image',
  modelId: '',
  channelId: '',
  size: 'auto',
  quality: 'auto',
  outputFormat: 'png',
  outputCompression: null,
  moderation: 'low',
});

const createEmptyChannelDraft = (): ChannelDraft => ({
  id: null,
  name: '',
  provider_kind: 'openai_compatible',
  base_url: '',
  api_key: '',
  generation_path: null,
  edit_path: null,
  timeout_seconds: 300,
  enabled: true,
  models: [],
});

const buildDropdownItems = (
  options: Array<{ value: string; label: string }>
): MenuProps['items'] => (
  options.map((option) => ({
    key: option.value,
    label: option.label,
  }))
);

const findOptionLabel = (
  options: Array<{ value: string; label: string }>,
  value: string
): string => options.find((option) => option.value === value)?.label ?? value;

const measureTextWidth = (text: string, font: string): number => {
  if (typeof document === 'undefined') {
    return text.length * 8;
  }

  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context) {
    return text.length * 8;
  }

  context.font = font;
  return context.measureText(text).width;
};

interface SortableChannelCardProps {
  channel: ImageChannel;
  active: boolean;
  onEdit: (channel: ImageChannel) => void;
  onCopy: (channel: ImageChannel) => void;
  onDelete: (channelId: string) => void;
  onSelect: (channelId: string) => void;
  children?: React.ReactNode;
}

const SortableChannelCard: React.FC<SortableChannelCardProps> = ({
  channel,
  active,
  onEdit,
  onCopy,
  onDelete,
  onSelect,
  children,
}) => {
  const { t } = useTranslation();
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: channel.id });

  const sortableStyle: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.6 : 1,
  };

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <div
        className={`${styles.channelListItem} ${active ? styles.channelListItemActive : ''}`}
        onClick={() => onSelect(channel.id)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            onSelect(channel.id);
          }
        }}
        role="button"
        tabIndex={0}
      >
        <div className={styles.channelListHead}>
          <div className={styles.channelListTitle}>
            <div className={styles.channelTitleRow}>
              <div
                {...attributes}
                {...listeners}
                className={styles.channelDragHandle}
                onClick={(event) => event.stopPropagation()}
              >
                <GripVertical size={14} />
              </div>
              <Text strong>{channel.name}</Text>
              <span className={styles.channelBaseUrl}>{channel.base_url}</span>
              <Tag color={channel.enabled ? 'success' : 'default'}>
                {channel.enabled ? t('image.more.enabled') : t('image.more.disabled')}
              </Tag>
            </div>
            <div className={styles.channelInlineMeta}>
              <span className={styles.hintText}>
                {findOptionLabel(PROVIDER_KIND_OPTIONS, channel.provider_kind)}
              </span>
              {children}
            </div>
          </div>

          <div className={styles.channelListActions}>
            <Button
              size="small"
              className={styles.toolActionIconButton}
              icon={<Pencil size={14} />}
              onClick={(event) => {
                event.stopPropagation();
                onEdit(channel);
              }}
            />
            <Button
              size="small"
              className={styles.toolActionIconButton}
              icon={<Copy size={14} />}
              onClick={(event) => {
                event.stopPropagation();
                onCopy(channel);
              }}
            />
            <Popconfirm
              title={t('image.more.confirmDelete')}
              onConfirm={() => onDelete(channel.id)}
            >
              <Button
                size="small"
                className={styles.dangerToolIconButton}
                danger
                icon={<Trash2 size={14} />}
                onClick={(event) => event.stopPropagation()}
              />
            </Popconfirm>
          </div>
        </div>
      </div>
    </div>
  );
};

const channelToDraft = (channel: ImageChannel): ChannelDraft => ({
  id: channel.id,
  name: channel.name,
  provider_kind: channel.provider_kind,
  base_url: channel.base_url,
  api_key: channel.api_key,
  generation_path: channel.generation_path ?? null,
  edit_path: channel.edit_path ?? null,
  timeout_seconds: channel.timeout_seconds ?? 300,
  enabled: channel.enabled,
  models: channel.models.map((model) => ({ ...model })),
});

const fileToBase64DataUrl = async (file: File): Promise<string> => {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        resolve(reader.result);
        return;
      }
      reject(new Error('Failed to read file as data URL'));
    };
    reader.onerror = () => reject(reader.error ?? new Error('Failed to read file'));
    reader.readAsDataURL(file);
  });
};

const filePathToDataUrl = async (filePath: string): Promise<string> => {
  const response = await fetch(convertFileSrc(filePath));
  if (!response.ok) {
    throw new Error(`Failed to load image asset: HTTP ${response.status}`);
  }
  const blob = await response.blob();
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        resolve(reader.result);
        return;
      }
      reject(new Error('Failed to convert asset to data URL'));
    };
    reader.onerror = () => reject(reader.error ?? new Error('Failed to read asset blob'));
    reader.readAsDataURL(blob);
  });
};

const formatTime = (timestamp?: number | null): string => {
  if (!timestamp) return '-';
  return new Date(timestamp).toLocaleString();
};

const formatElapsedClock = (elapsedMs: number): string => {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
  }

  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
};

const toErrorMessage = (error: unknown, fallbackMessage: string): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === 'string' && error.trim()) {
    return error;
  }

  const stringifiedError = String(error ?? '').trim();
  return stringifiedError && stringifiedError !== '[object Object]'
    ? stringifiedError
    : fallbackMessage;
};

const buildResultDimensionLabel = (
  width?: number | null,
  height?: number | null,
  fallbackSize?: string | null
): string | null => {
  if (typeof width === 'number' && width > 0 && typeof height === 'number' && height > 0) {
    return `${width}x${height}`;
  }

  if (!fallbackSize) {
    return null;
  }

  const normalizedSize = normalizeImageSize(fallbackSize);
  if (!normalizedSize || normalizedSize === 'auto') {
    return null;
  }

  return normalizedSize;
};

const buildWorkbenchModelOptions = (channels: ImageChannel[]): WorkbenchModelOption[] => {
  const modelMap = new Map<string, WorkbenchModelOption>();

  for (const channel of channels) {
    if (!channel.enabled) continue;

    for (const model of channel.models) {
      if (!model.enabled) continue;

      const existingModel = modelMap.get(model.id);
      const nextChannelOption = {
        id: channel.id,
        name: channel.name,
        sortOrder: channel.sort_order,
      };

      if (existingModel) {
        existingModel.supportsTextToImage =
          existingModel.supportsTextToImage || model.supports_text_to_image;
        existingModel.supportsImageToImage =
          existingModel.supportsImageToImage || model.supports_image_to_image;

        if (!existingModel.availableChannels.some((item) => item.id === channel.id)) {
          existingModel.availableChannels.push(nextChannelOption);
        }
        continue;
      }

      modelMap.set(model.id, {
        id: model.id,
        label: model.name?.trim() || model.id,
        supportsTextToImage: model.supports_text_to_image,
        supportsImageToImage: model.supports_image_to_image,
        availableChannels: [nextChannelOption],
      });
    }
  }

  const modelOptions = [...modelMap.values()]
    .map((item) => ({
      ...item,
      availableChannels: [...item.availableChannels].sort(
        (left, right) => left.sortOrder - right.sortOrder
      ),
    }))
    .sort((left, right) => left.label.localeCompare(right.label));

  return modelOptions;
};

const filterModelsByMode = (
  modelOptions: WorkbenchModelOption[],
  mode: ImageModeKey
): WorkbenchModelOption[] => (
  modelOptions.filter((modelOption) =>
    mode === 'text_to_image'
      ? modelOption.supportsTextToImage
      : modelOption.supportsImageToImage
  )
);

const ImagePage: React.FC = () => {
  const { t } = useTranslation();
  const { message, modal } = App.useApp();
  const {
    channels,
    jobs,
    latestJob,
    loading,
    submitting,
    channelSaving,
    activeView,
    editingChannelId,
    loadWorkspace,
    refreshJobs,
    saveChannel,
    removeChannel,
    removeJob,
    reorderChannels,
    submitJob,
    setActiveView,
    setEditingChannelId,
  } = useImage();

  const [formState, setFormState] = React.useState<FormState>(createDefaultFormState);
  const [references, setReferences] = React.useState<LocalReferenceImage[]>([]);
  const [sizePickerOpen, setSizePickerOpen] = React.useState(false);
  const [channelDraft, setChannelDraft] = React.useState<ChannelDraft>(createEmptyChannelDraft);
  const [channelDraftSourceId, setChannelDraftSourceId] = React.useState<string | null>(null);
  const [channelModalOpen, setChannelModalOpen] = React.useState(false);
  const [requestDetailJobId, setRequestDetailJobId] = React.useState<string | null>(null);
  const [generationStartedAt, setGenerationStartedAt] = React.useState<number | null>(null);
  const [generationElapsedMs, setGenerationElapsedMs] = React.useState(0);
  const previousActiveViewRef = React.useRef(activeView);

  const channelListSensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8,
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const modelOptions = React.useMemo(
    () => buildWorkbenchModelOptions(channels),
    [channels]
  );

  const availableModelOptions = React.useMemo(
    () => filterModelsByMode(modelOptions, formState.mode),
    [formState.mode, modelOptions]
  );

  const selectedModelOption = React.useMemo(
    () => availableModelOptions.find((item) => item.id === formState.modelId) ?? null,
    [availableModelOptions, formState.modelId]
  );

  const modelTriggerWidth = React.useMemo(() => {
    const fallbackLabel = t('image.workbench.selectModel');
    const longestModelLabel = availableModelOptions.reduce(
      (currentLongestLabel, option) => (
        option.label.length > currentLongestLabel.length ? option.label : currentLongestLabel
      ),
      fallbackLabel
    );
    const measuredLabelWidth = measureTextWidth(
      longestModelLabel,
      '12px ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace'
    );
    const triggerChromeWidth = 38;
    const horizontalPaddingWidth = 24;
    const boundedWidth = Math.min(
      Math.max(Math.ceil(measuredLabelWidth + triggerChromeWidth + horizontalPaddingWidth), 148),
      420
    );
    return `${boundedWidth}px`;
  }, [availableModelOptions, t]);

  const availableChannelOptions = React.useMemo(
    () => selectedModelOption?.availableChannels ?? [],
    [selectedModelOption]
  );

  const selectedChannel = React.useMemo(
    () => channels.find((channel) => channel.id === formState.channelId) ?? null,
    [channels, formState.channelId]
  );

  const editingChannel = React.useMemo(
    () => channels.find((channel) => channel.id === editingChannelId) ?? null,
    [channels, editingChannelId]
  );

  const requestDetailJob = React.useMemo(
    () => jobs.find((job) => job.id === requestDetailJobId) ?? null,
    [jobs, requestDetailJobId]
  );

  const resultImages = React.useMemo(
    (): ResultImageViewModel[] => {
      const fallbackSize = latestJob ? parseHistoryJobParams(latestJob.params_json)?.size ?? null : null;

      return (latestJob?.output_assets ?? []).map((asset) => ({
        ...asset,
        previewUrl: convertFileSrc(asset.file_path),
        dimensionLabel: buildResultDimensionLabel(asset.width, asset.height, fallbackSize),
      }));
    },
    [latestJob]
  );

  const isImageToImage = formState.mode === 'image_to_image';
  const isCompressionDisabled = formState.outputFormat === 'png';
  const hasAvailableModels = availableModelOptions.length > 0;
  const hasAvailableChannels = availableChannelOptions.length > 0;
  const selectedParameterVisibility = React.useMemo(
    () => getImageParameterVisibility(formState.modelId, selectedModelOption?.label ?? null),
    [formState.modelId, selectedModelOption?.label]
  );

  React.useEffect(() => {
    if (!hasAvailableModels) {
      setFormState((currentFormState) => ({
        ...currentFormState,
        modelId: '',
        channelId: '',
      }));
      return;
    }

    if (availableModelOptions.some((item) => item.id === formState.modelId)) {
      return;
    }

    setFormState((currentFormState) => ({
      ...currentFormState,
      modelId: availableModelOptions[0]?.id ?? '',
    }));
  }, [availableModelOptions, formState.modelId, hasAvailableModels]);

  React.useEffect(() => {
    if (!hasAvailableChannels) {
      setFormState((currentFormState) => ({
        ...currentFormState,
        channelId: '',
      }));
      return;
    }

    if (availableChannelOptions.some((item) => item.id === formState.channelId)) {
      return;
    }

    setFormState((currentFormState) => ({
      ...currentFormState,
      channelId: availableChannelOptions[0]?.id ?? '',
    }));
  }, [availableChannelOptions, formState.channelId, hasAvailableChannels]);

  React.useEffect(() => {
    if (!editingChannel) {
      if (channelDraftSourceId === null) {
        return;
      }

      if (channels.length === 0) {
        setChannelDraft(createEmptyChannelDraft());
        setChannelDraftSourceId(null);
      }
      return;
    }

    if (channelDraftSourceId === editingChannel.id && channelDraft.id === editingChannel.id) {
      return;
    }

    setChannelDraft(channelToDraft(editingChannel));
    setChannelDraftSourceId(editingChannel.id);
  }, [channelDraft.id, channelDraftSourceId, channels.length, editingChannel]);

  React.useEffect(() => {
    const previousActiveView = previousActiveViewRef.current;
    previousActiveViewRef.current = activeView;

    if (activeView === 'workbench' && previousActiveView !== 'workbench') {
      void loadWorkspace();
    }
  }, [activeView, loadWorkspace]);

  React.useEffect(() => {
    if (!generationStartedAt) {
      setGenerationElapsedMs(0);
      return;
    }

    setGenerationElapsedMs(Date.now() - generationStartedAt);
    const timerId = window.setInterval(() => {
      setGenerationElapsedMs(Date.now() - generationStartedAt);
    }, 1000);

    return () => {
      window.clearInterval(timerId);
    };
  }, [generationStartedAt]);

  const parseRequestSnapshotJson = React.useCallback((rawValue?: string | null) => {
    const trimmedValue = rawValue?.trim();
    if (!trimmedValue) {
      return {};
    }

    try {
      return JSON.parse(trimmedValue);
    } catch {
      return { raw: trimmedValue };
    }
  }, []);

  const renderParamDropdownTrigger = React.useCallback(
    (valueLabel: string, className: string, style?: React.CSSProperties) => (
      <button
        type="button"
        className={`${styles.sizeTrigger} ${styles.paramSelectTrigger} ${className}`}
        style={style}
      >
        <span className={styles.paramSelectValue}>{valueLabel}</span>
        <ChevronDown size={14} className={styles.paramSelectIcon} />
      </button>
    ),
    []
  );

  const formatHistoryJobParams = React.useCallback((
    jobParamsJson: string,
    modelId: string,
    modelName?: string | null
  ) => {
    const parsedParams = parseHistoryJobParams(jobParamsJson);
    if (!parsedParams) {
      return '';
    }

    const visibleParams = filterHistoryJobParamsByModel(
      parsedParams,
      modelId,
      modelName
    );

    const summaryParts = [
      visibleParams.size
        ? `${t('image.fields.size')}: ${normalizeImageSize(visibleParams.size) || visibleParams.size}`
        : null,
      visibleParams.quality
        ? `${t('image.fields.quality')}: ${findOptionLabel(QUALITY_OPTIONS, visibleParams.quality)}`
        : null,
      visibleParams.output_format
        ? `${t('image.fields.outputFormat')}: ${findOptionLabel(FORMAT_OPTIONS, visibleParams.output_format)}`
        : null,
      visibleParams.moderation
        ? `${t('image.fields.moderation')}: ${findOptionLabel(MODERATION_OPTIONS, visibleParams.moderation)}`
        : null,
      typeof visibleParams.output_compression === 'number'
        ? `${t('image.fields.outputCompression')}: ${visibleParams.output_compression}`
        : null,
    ].filter((value): value is string => Boolean(value));

    return summaryParts.join(' · ');
  }, [t]);

  const handleAddFiles = React.useCallback(async (files: File[]) => {
    const acceptedFiles = files.filter((file) => file.type.startsWith('image/'));
    const nextReferences = await Promise.all(
      acceptedFiles.map(async (file) => {
        const base64Data = await fileToBase64DataUrl(file);
        return {
          id: `${file.name}-${file.size}-${file.lastModified}-${Math.random().toString(36).slice(2, 8)}`,
          fileName: file.name,
          mimeType: file.type || 'image/png',
          base64Data,
          previewUrl: base64Data,
        } satisfies LocalReferenceImage;
      })
    );
    setReferences((currentReferences) => (
      [...currentReferences, ...nextReferences].slice(0, MAX_REFERENCE_COUNT)
    ));
  }, []);

  const handleGenerate = async () => {
    if (!formState.prompt.trim()) {
      message.error(t('image.errors.promptRequired'));
      return;
    }

    if (!formState.modelId) {
      message.error(t('image.errors.modelRequired'));
      return;
    }

    if (!formState.channelId) {
      message.error(t('image.errors.channelRequired'));
      return;
    }

    if (isImageToImage && references.length === 0) {
      message.error(t('image.errors.referenceRequired'));
      return;
    }

    const input: CreateImageJobInput = {
      mode: formState.mode,
      prompt: formState.prompt.trim(),
      channel_id: formState.channelId,
      model_id: formState.modelId,
      params: {
        size: formState.size,
        quality: formState.quality,
        output_format: formState.outputFormat,
        output_compression:
          selectedParameterVisibility.outputCompression && !isCompressionDisabled
            ? formState.outputCompression
            : null,
        moderation: selectedParameterVisibility.moderation
          ? formState.moderation
          : null,
      },
      references: isImageToImage
        ? references.map((reference) => ({
            file_name: reference.fileName,
            mime_type: reference.mimeType,
            base64_data: reference.base64Data,
          }))
        : [],
    };

    setGenerationStartedAt(Date.now());
    try {
      const job = await submitJob(input);
      if (job.status === 'error') {
        message.error(job.error_message || t('image.errors.generateFailed'));
        return;
      }

      message.success(t('image.messages.generated'));
    } catch (error) {
      message.error(error instanceof Error ? error.message : t('image.errors.generateFailed'));
    } finally {
      setGenerationStartedAt(null);
    }
  };

  const handleReset = () => {
    setFormState((currentFormState) => ({
      ...createDefaultFormState(),
      mode: currentFormState.mode,
    }));
    setReferences([]);
  };

  const handleDownloadAsset = async (filePath: string, fileName: string) => {
    const targetPath = await saveDialog({
      defaultPath: fileName,
      title: t('image.download.selectPath'),
    });
    if (!targetPath || Array.isArray(targetPath)) return;
    await copyFile(filePath, targetPath);
    message.success(t('image.messages.downloaded'));
  };

  const handleSelectHistoryJob = (jobId: string) => {
    const targetJob = jobs.find((job) => job.id === jobId);
    if (!targetJob) return;

    setFormState((currentFormState) => ({
      ...currentFormState,
      prompt: targetJob.prompt,
      mode: targetJob.mode,
      modelId: targetJob.model_id,
      channelId: targetJob.channel_id,
    }));
    setActiveView('workbench');
  };

  const handleCreateChannel = () => {
    setEditingChannelId(null);
    setChannelDraft(createEmptyChannelDraft());
    setChannelDraftSourceId(null);
    setChannelModalOpen(true);
  };

  const handleSaveChannel = async () => {
    const normalizedModels = channelDraft.models.map((model) => ({
      ...model,
      id: model.id.trim(),
      name: model.name?.trim() || '',
    }));

    const input: UpsertImageChannelInput = {
      id: channelDraft.id,
      name: channelDraft.name.trim(),
      provider_kind: channelDraft.provider_kind,
      base_url: channelDraft.base_url.trim(),
      api_key: channelDraft.api_key.trim(),
      generation_path: channelDraft.generation_path?.trim() || null,
      edit_path: channelDraft.edit_path?.trim() || null,
      timeout_seconds: channelDraft.timeout_seconds ?? 300,
      enabled: channelDraft.enabled,
      models: normalizedModels,
    };

    try {
      const savedChannel = await saveChannel(input);
      await loadWorkspace();
      setChannelDraft(channelToDraft(savedChannel));
      setChannelDraftSourceId(savedChannel.id);
      setChannelModalOpen(false);
      message.success(t('image.more.messages.saved'));
    } catch (error) {
      message.error(toErrorMessage(error, t('common.error')));
    }
  };

  const handleOpenEditChannel = (channel: ImageChannel) => {
    setEditingChannelId(channel.id);
    setChannelDraft(channelToDraft(channel));
    setChannelDraftSourceId(channel.id);
    setChannelModalOpen(true);
  };

  const handleCopyChannel = async (channel: ImageChannel) => {
    const copiedModels = channel.models.map((model) => ({ ...model }));
    setChannelDraft({
      ...channelToDraft(channel),
      id: null,
      name: `${channel.name} Copy`,
      models: copiedModels,
    });
    setChannelDraftSourceId(null);
    setChannelModalOpen(true);
  };

  const handleChannelDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const oldIndex = channels.findIndex((channel) => channel.id === active.id);
    const newIndex = channels.findIndex((channel) => channel.id === over.id);

    if (oldIndex < 0 || newIndex < 0) return;

    const reorderedChannels = arrayMove(channels, oldIndex, newIndex);
    try {
      await reorderChannels(reorderedChannels.map((channel) => channel.id));
    } catch (error) {
      message.error(toErrorMessage(error, t('common.error')));
    }
  };

  const handleDeleteChannel = async (channelId: string) => {
    try {
      await removeChannel(channelId);
      message.success(t('image.more.messages.deleted'));
    } catch (error) {
      message.error(toErrorMessage(error, t('common.error')));
    }
  };

  const handleDeleteHistoryJob = (jobId: string) => {
    let shouldDeleteLocalAssets = false;

    modal.confirm({
      title: t('image.history.deleteConfirmTitle'),
      content: (
        <div className={styles.deleteConfirmContent}>
          <div>{t('image.history.deleteConfirmHint')}</div>
          <Checkbox
            onChange={(event) => {
              shouldDeleteLocalAssets = event.target.checked;
            }}
          >
            {t('image.history.deleteLocalAssets')}
          </Checkbox>
        </div>
      ),
      icon: <ExclamationCircleOutlined />,
      okText: t('common.delete'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        try {
          await removeJob(jobId, shouldDeleteLocalAssets);
          if (requestDetailJobId === jobId) {
            setRequestDetailJobId(null);
          }
          message.success(t('image.history.deleteSuccess'));
        } catch (error) {
          message.error(toErrorMessage(error, t('common.error')));
        }
      },
    });
  };

  const isGenerating = submitting || generationStartedAt !== null;

  const latestStatusColor =
    isGenerating
      ? 'processing'
      : latestJob?.status === 'done'
      ? 'success'
      : latestJob?.status === 'error'
      ? 'error'
      : 'processing';

  const latestStatusKey = isGenerating ? 'running' : latestJob?.status || 'idle';

  const workbenchView = (
    <div className={styles.contentGrid}>
      <section className={styles.sectionCard}>
        <div className={styles.sectionHeader}>
          <div className={styles.sectionTitle}>
            <Text strong>{t('image.workbench.createTitle')}</Text>
            <span className={styles.sectionHint}>{t('image.workbench.createHint')}</span>
          </div>
          <div className={styles.modeGroup}>
            {MODE_KEYS.map((modeKey) => (
              <button
                key={modeKey}
                type="button"
                className={`${styles.modeButton} ${formState.mode === modeKey ? styles.modeButtonActive : ''}`}
                onClick={() => setFormState((currentFormState) => ({ ...currentFormState, mode: modeKey }))}
              >
                {t(`image.modes.${modeKey}`)}
              </button>
            ))}
          </div>
        </div>

        <div className={styles.formGrid}>
          <div className={styles.fieldRow}>
            <div className={styles.fieldLabel}>{t('image.fields.parameters')}</div>
            <div className={styles.paramsPanel}>
              <div className={styles.paramRowPrimary}>
                <div className={styles.paramField}>
                  <span className={styles.paramLabel}>{t('image.fields.model')}</span>
                  {hasAvailableModels ? (
                    <Dropdown
                      trigger={['click']}
                      overlayClassName={styles.paramDropdownOverlay}
                      menu={{
                        items: buildDropdownItems(
                          availableModelOptions.map((item) => ({ value: item.id, label: item.label }))
                        ),
                        selectable: true,
                        selectedKeys: formState.modelId ? [formState.modelId] : [],
                        onClick: ({ key }) =>
                          setFormState((currentFormState) => ({
                            ...currentFormState,
                            modelId: key,
                          })),
                      }}
                    >
                      {renderParamDropdownTrigger(
                        selectedModelOption?.label || t('image.workbench.selectModel'),
                        styles.paramControl,
                        { width: modelTriggerWidth }
                      )}
                    </Dropdown>
                  ) : (
                    <div className={styles.inlineEmptyText}>{t('image.workbench.noModelAvailable')}</div>
                  )}
                </div>

                <div className={styles.paramField}>
                  <span className={styles.paramLabel}>{t('image.fields.channel')}</span>
                  {hasAvailableChannels ? (
                    availableChannelOptions.length > 1 ? (
                      <Dropdown
                        trigger={['click']}
                        overlayClassName={styles.paramDropdownOverlay}
                        menu={{
                          items: buildDropdownItems(
                            availableChannelOptions.map((item) => ({ value: item.id, label: item.name }))
                          ),
                          selectable: true,
                          selectedKeys: formState.channelId ? [formState.channelId] : [],
                          onClick: ({ key }) =>
                            setFormState((currentFormState) => ({
                              ...currentFormState,
                              channelId: key,
                            })),
                        }}
                      >
                        {renderParamDropdownTrigger(
                          selectedChannel?.name || t('image.workbench.selectChannel'),
                          `${styles.paramControl} ${styles.paramControlWide}`
                        )}
                      </Dropdown>
                    ) : (
                      <button type="button" className={`${styles.sizeTrigger} ${styles.paramSizeButton}`} disabled>
                        {availableChannelOptions[0]?.name || '-'}
                      </button>
                    )
                  ) : (
                    <div className={styles.inlineEmptyText}>{t('image.workbench.noChannelAvailable')}</div>
                  )}
                </div>

                <div className={styles.paramField}>
                  <span className={styles.paramLabel}>{t('image.fields.size')}</span>
                  <button
                    type="button"
                    className={`${styles.sizeTrigger} ${styles.paramSizeButton}`}
                    onClick={() => setSizePickerOpen(true)}
                  >
                    {normalizeImageSize(formState.size) || 'auto'}
                  </button>
                </div>

                <div className={styles.paramField}>
                  <span className={styles.paramLabel}>{t('image.fields.quality')}</span>
                  <Dropdown
                    trigger={['click']}
                    overlayClassName={styles.paramDropdownOverlay}
                    menu={{
                      items: buildDropdownItems(QUALITY_OPTIONS),
                      selectable: true,
                      selectedKeys: [formState.quality],
                      onClick: ({ key }) =>
                        setFormState((currentFormState) => ({
                          ...currentFormState,
                          quality: key,
                        })),
                    }}
                  >
                    {renderParamDropdownTrigger(
                      findOptionLabel(QUALITY_OPTIONS, formState.quality),
                      `${styles.paramControl} ${styles.paramControlWide}`
                    )}
                  </Dropdown>
                </div>

                <div className={styles.paramField}>
                  <span className={styles.paramLabel}>{t('image.fields.outputFormat')}</span>
                  <Dropdown
                    trigger={['click']}
                    overlayClassName={styles.paramDropdownOverlay}
                    menu={{
                      items: buildDropdownItems(FORMAT_OPTIONS),
                      selectable: true,
                      selectedKeys: [formState.outputFormat],
                      onClick: ({ key }) =>
                        setFormState((currentFormState) => ({
                          ...currentFormState,
                          outputFormat: key,
                          outputCompression: key === 'png' ? null : currentFormState.outputCompression,
                        })),
                    }}
                  >
                    {renderParamDropdownTrigger(
                      findOptionLabel(FORMAT_OPTIONS, formState.outputFormat),
                      `${styles.paramControl} ${styles.paramControlWide}`
                    )}
                  </Dropdown>
                </div>

                {selectedParameterVisibility.moderation && (
                  <div className={styles.paramField}>
                    <span className={styles.paramLabel}>{t('image.fields.moderation')}</span>
                    <Dropdown
                      trigger={['click']}
                      overlayClassName={styles.paramDropdownOverlay}
                      menu={{
                        items: buildDropdownItems(MODERATION_OPTIONS),
                        selectable: true,
                        selectedKeys: [formState.moderation],
                        onClick: ({ key }) =>
                          setFormState((currentFormState) => ({
                            ...currentFormState,
                            moderation: key,
                          })),
                      }}
                    >
                      {renderParamDropdownTrigger(
                        findOptionLabel(MODERATION_OPTIONS, formState.moderation),
                        `${styles.paramControl} ${styles.paramControlMedium}`
                      )}
                    </Dropdown>
                  </div>
                )}

                {selectedParameterVisibility.outputCompression && (
                  <div className={styles.paramField}>
                    <span className={styles.paramLabel}>{t('image.fields.outputCompression')}</span>
                    <InputNumber
                      className={`${styles.paramControl} ${styles.paramNumberControl} ${styles.paramControlNarrow}`}
                      size="small"
                      min={0}
                      max={100}
                      controls={false}
                      value={formState.outputCompression}
                      disabled={isCompressionDisabled}
                      placeholder={t('image.placeholders.outputCompression')}
                      onChange={(value) =>
                        setFormState((currentFormState) => ({
                          ...currentFormState,
                          outputCompression: typeof value === 'number' ? value : null,
                        }))
                      }
                    />
                  </div>
                )}
              </div>

              {selectedParameterVisibility.outputCompression && (
                <div className={styles.paramHint}>
                  {isCompressionDisabled
                    ? t('image.hints.outputCompressionDisabled')
                    : t('image.hints.outputCompression')}
                </div>
              )}
            </div>
          </div>

          <div className={styles.fieldRow}>
            <div className={styles.fieldLabel}>{t('image.fields.prompt')}</div>
            <div>
              <Input.TextArea
                className={styles.textarea}
                placeholder={t('image.placeholders.prompt')}
                value={formState.prompt}
                onChange={(event) =>
                  setFormState((currentFormState) => ({
                    ...currentFormState,
                    prompt: event.target.value,
                  }))
                }
              />
              <div className={styles.hintText}>{t('image.hints.prompt')}</div>
            </div>
          </div>

          {isImageToImage && (
            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.fields.references')}</div>
              <div>
                <Space direction="vertical" size={12} style={{ width: '100%' }}>
                  <div className={styles.referenceHeader}>
                    <span className={styles.referenceCount}>
                      {t('image.referenceCount', { count: references.length, max: MAX_REFERENCE_COUNT })}
                    </span>
                  </div>
                  <Upload.Dragger
                    multiple
                    showUploadList={false}
                    beforeUpload={(file) => {
                      void handleAddFiles([file]);
                      return false;
                    }}
                  >
                    <p className="ant-upload-drag-icon">
                      <UploadIcon size={18} />
                    </p>
                    <p className="ant-upload-text">{t('image.upload.title')}</p>
                    <p className="ant-upload-hint">{t('image.upload.hint')}</p>
                  </Upload.Dragger>

                  <div className={styles.referenceList}>
                    {references.map((reference) => (
                      <div key={reference.id} className={styles.referenceItem}>
                        <Image
                          src={reference.previewUrl}
                          alt=""
                          className={styles.referenceThumb}
                          preview={{ mask: t('common.preview') }}
                        />
                        <div className={styles.referenceActions}>
                          <Button
                            size="small"
                            className={styles.dangerToolActionButton}
                            danger
                            onClick={() =>
                              setReferences((currentReferences) =>
                                currentReferences.filter((item) => item.id !== reference.id)
                              )
                            }
                          >
                            {t('common.delete')}
                          </Button>
                        </div>
                      </div>
                    ))}
                  </div>

                  <div className={styles.hintText}>{t('image.hints.references')}</div>
                </Space>
              </div>
            </div>
          )}

          <div className={styles.fieldRow}>
            <div className={styles.fieldLabel}>{t('image.fields.actions')}</div>
            <Space wrap>
              <Button
                type="primary"
                className={styles.primaryActionButton}
                icon={<Sparkles size={14} />}
                onClick={() => void handleGenerate()}
                loading={submitting}
                disabled={!hasAvailableModels || !hasAvailableChannels}
              >
                {t('image.actions.generate')}
              </Button>
              <Button
                className={styles.secondaryActionButton}
                icon={<RefreshCcw size={14} />}
                onClick={handleReset}
              >
                {t('image.actions.reset')}
              </Button>
            </Space>
          </div>
        </div>
      </section>

      <section className={styles.sectionCard}>
        <div className={styles.sectionHeader}>
          <div className={styles.sectionTitle}>
            <Text strong>{t('image.workbench.resultTitle')}</Text>
            <span className={styles.sectionHint}>{t('image.workbench.resultHint')}</span>
          </div>
          <Tag color={latestStatusColor}>{t(`image.status.${latestStatusKey}`)}</Tag>
        </div>

        {isGenerating ? (
          <div className={styles.resultLoading}>
            <div className={styles.resultLoadingMain}>
              <Spin size="large" />
              <div className={styles.resultLoadingText}>
                <Text>{t('image.workbench.resultGeneratingTitle')}</Text>
                <span className={styles.hintText}>{t('image.workbench.resultGeneratingHint')}</span>
              </div>
            </div>
            <div className={styles.resultLoadingTimer}>
              {t('image.workbench.resultGeneratingElapsed', {
                elapsed: formatElapsedClock(generationElapsedMs),
              })}
            </div>
          </div>
        ) : resultImages.length > 0 ? (
          <div className={styles.resultPreview}>
            {resultImages.map((asset, index) => (
              <div key={asset.id} className={styles.resultImageCard}>
                <div className={styles.resultImageMedia}>
                  <Image
                    src={asset.previewUrl}
                    alt=""
                    className={styles.resultImage}
                    preview={{ mask: t('common.preview') }}
                  />
                  <span className={styles.resultBadge}>
                    {index + 1}
                  </span>
                  {asset.dimensionLabel && (
                    <span className={styles.resultSizeBadge}>
                      {asset.dimensionLabel}
                    </span>
                  )}
                </div>
                <div className={styles.resultMeta}>
                  <Space size={4}>
                    <Button
                      size="small"
                      className={styles.secondaryActionButtonCompact}
                      onClick={async () => {
                        try {
                          const base64Data = await filePathToDataUrl(asset.file_path);
                          setReferences((currentReferences) => [
                            ...currentReferences,
                            {
                              id: `reuse-${asset.id}-${Date.now()}`,
                              fileName: asset.file_name,
                              mimeType: asset.mime_type,
                              base64Data,
                              previewUrl: asset.previewUrl,
                            },
                          ].slice(0, MAX_REFERENCE_COUNT));
                          setFormState((currentFormState) => ({
                            ...currentFormState,
                            mode: 'image_to_image',
                          }));
                          message.success(t('image.messages.reusedAsReference'));
                        } catch (error) {
                          message.error(
                            error instanceof Error ? error.message : t('image.errors.reuseFailed')
                          );
                        }
                      }}
                    >
                      {t('image.actions.reuse')}
                    </Button>
                    <Button
                      size="small"
                      className={styles.secondaryActionButtonCompact}
                      onClick={() => void handleDownloadAsset(asset.file_path, asset.file_name)}
                    >
                      {t('image.actions.download')}
                    </Button>
                  </Space>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className={styles.resultEmpty}>
            <Text>{t('image.workbench.resultEmptyTitle')}</Text>
            <span className={styles.hintText}>{t('image.workbench.resultEmptyHint')}</span>
          </div>
        )}
      </section>
    </div>
  );

  const moreView = (
    <section className={styles.sectionCard}>
      <div className={styles.sectionHeader}>
        <div className={styles.sectionTitle}>
          <Text strong>{t('image.more.title')}</Text>
        </div>
        <Button
          type="primary"
          className={styles.primaryActionButton}
          icon={<Plus size={14} />}
          onClick={() => void handleCreateChannel()}
          loading={channelSaving}
        >
          {t('image.more.actions.addChannel')}
        </Button>
      </div>

      <div className={styles.channelList}>
        {channels.length > 0 && (
          <DndContext
            sensors={channelListSensors}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis]}
            onDragEnd={handleChannelDragEnd}
          >
            <SortableContext
              items={channels.map((channel) => channel.id)}
              strategy={verticalListSortingStrategy}
            >
              <div className={styles.channelList}>
                {channels.map((channel) => {
                  const visibleModels = channel.models.filter((model) => model.enabled);

                  return (
                    <SortableChannelCard
                      key={channel.id}
                      channel={channel}
                      active={editingChannelId === channel.id}
                      onEdit={handleOpenEditChannel}
                      onCopy={(currentChannel) => {
                        void handleCopyChannel(currentChannel);
                      }}
                      onDelete={(channelId) => {
                        void handleDeleteChannel(channelId);
                      }}
                      onSelect={setEditingChannelId}
                    >
                      {visibleModels.length > 0 ? (
                        <div className={styles.channelModelNames}>
                          {visibleModels.map((model) => (
                            <Tag key={`${channel.id}-${model.id}`} className={styles.channelModelTag}>
                              {model.name?.trim() || model.id}
                            </Tag>
                          ))}
                        </div>
                      ) : (
                        <span className={styles.hintText}>{t('image.more.emptyModels')}</span>
                      )}
                    </SortableChannelCard>
                  );
                })}
              </div>
            </SortableContext>
          </DndContext>
        )}

        {channels.length === 0 && (
          <Empty description={t('image.more.empty')} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        )}
      </div>
    </section>
  );

  const historyView = (
    <section className={styles.sectionCard}>
      <div className={styles.sectionHeader}>
        <div className={styles.sectionTitle}>
          <Text strong>{t('image.history.title')}</Text>
          <span className={styles.sectionHint}>{t('image.history.hint')}</span>
        </div>
        <Space>
          <Button
            className={styles.secondaryActionButton}
            icon={<RefreshCcw size={14} />}
            onClick={() => void refreshJobs()}
            loading={loading}
          >
            {t('common.refresh')}
          </Button>
          <Button
            type="primary"
            className={styles.primaryActionButton}
            icon={<Palette size={14} />}
            onClick={() => setActiveView('workbench')}
          >
            {t('image.actions.backToWorkbench')}
          </Button>
        </Space>
      </div>

      <div className={styles.historyList}>
        {jobs.map((job) => {
          const historyParamsSummary =
            job.status === 'done'
              ? formatHistoryJobParams(
                  job.params_json,
                  job.model_id,
                  job.model_name_snapshot
                )
              : '';

          return (
            <div key={job.id} className={styles.historyItem}>
              {job.output_assets[0] && (
                <div className={styles.historyPreview}>
                  <Image
                    src={convertFileSrc(job.output_assets[0].file_path)}
                    alt=""
                    className={styles.historyPreviewImage}
                    preview={{ mask: t('common.preview') }}
                  />
                </div>
              )}
              <div className={styles.historyTopRow}>
                <div>
                  <div className={styles.historyPrompt}>{job.prompt}</div>
                  <div className={styles.historyMeta}>
                    <span>{job.model_name_snapshot}</span>
                    <span>{job.channel_name_snapshot}</span>
                    <span>{t(`image.modes.${job.mode}`)}</span>
                    <span>{formatTime(job.created_at)}</span>
                    <span>{job.elapsed_ms ? `${job.elapsed_ms} ms` : '-'}</span>
                  </div>
                  {historyParamsSummary && (
                    <div className={styles.historyParams}>
                      {historyParamsSummary}
                    </div>
                  )}
                </div>
                <div className={styles.historyHeadSide}>
                  <Tag
                    color={job.status === 'done' ? 'success' : job.status === 'error' ? 'error' : 'processing'}
                    className={styles.historyStatusTag}
                  >
                    {t(`image.status.${job.status}`)}
                  </Tag>
                  <div className={styles.historyHeadActions}>
                    <Button
                      size="small"
                      className={styles.toolActionIconButton}
                      icon={<FileJson size={14} />}
                      title={t('image.actions.viewDetail')}
                      onClick={() => setRequestDetailJobId(job.id)}
                    />
                    <Button
                      size="small"
                      className={styles.toolActionIconButton}
                      icon={<RotateCcw size={14} />}
                      title={t('image.actions.reuse')}
                      onClick={() => handleSelectHistoryJob(job.id)}
                    />
                    {job.output_assets[0] && (
                      <Button
                        size="small"
                        className={styles.toolActionIconButton}
                        icon={<Download size={14} />}
                        title={t('image.actions.download')}
                        onClick={() =>
                          void handleDownloadAsset(job.output_assets[0].file_path, job.output_assets[0].file_name)
                        }
                      />
                    )}
                    <Button
                      size="small"
                      className={styles.dangerToolIconButton}
                      danger
                      icon={<Trash2 size={14} />}
                      title={t('common.delete')}
                      onClick={() => handleDeleteHistoryJob(job.id)}
                    />
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {jobs.length === 0 && <Empty description={t('image.history.empty')} />}
    </section>
  );

  return (
    <div className={styles.imagePage}>
      <div className={styles.pageHeader}>
        <div className={styles.headerMeta}>
          <div className={styles.headerTitleRow}>
            <ImageIcon className={styles.headerIcon} />
            <Title level={4} style={{ margin: 0 }}>
              {t('image.title')}
            </Title>
            {selectedModelOption && <Tag color="blue">{selectedModelOption.label}</Tag>}
          </div>
          <div className={styles.headerHintRow}>
            <div className={styles.headerHint}>{t('image.pageHint')}</div>
            <Button
              className={`${styles.toolActionButton} ${styles.headerRefreshButton}`}
              size="small"
              icon={<RefreshCcw size={14} />}
              onClick={() => void loadWorkspace()}
              loading={loading}
            >
              {t('common.refresh')}
            </Button>
          </div>
        </div>

        <div className={styles.viewTabs}>
          <button
            type="button"
            className={`${styles.viewTab} ${activeView === 'workbench' ? styles.viewTabActive : ''}`}
            onClick={() => setActiveView('workbench')}
          >
            <Space size={6}>
              <Palette size={14} />
              <span>{t('image.views.workbench')}</span>
            </Space>
          </button>
          <button
            type="button"
            className={`${styles.viewTab} ${activeView === 'history' ? styles.viewTabActive : ''}`}
            onClick={() => setActiveView('history')}
          >
            <Space size={6}>
              <History size={14} />
              <span>{t('image.views.history')}</span>
            </Space>
          </button>
          <button
            type="button"
            className={`${styles.viewTab} ${activeView === 'more' ? styles.viewTabActive : ''}`}
            onClick={() => setActiveView('more')}
          >
            <Space size={6}>
              <Route size={14} />
              <span>{t('image.views.more')}</span>
            </Space>
          </button>
        </div>
      </div>

      {activeView === 'workbench' && workbenchView}
      {activeView === 'history' && historyView}
      {activeView === 'more' && moreView}

      <Modal
        open={Boolean(requestDetailJob)}
        title={t('image.history.requestDetailTitle')}
        onCancel={() => setRequestDetailJobId(null)}
        footer={null}
        width={860}
        className={styles.requestDetailModal}
        destroyOnHidden
      >
        {requestDetailJob && (
          <div className={styles.requestDetailContent}>
            <section className={styles.requestDetailSection}>
              <div className={styles.requestDetailLabel}>{t('image.history.errorMessage')}</div>
              <pre className={styles.requestDetailCode}>
                {requestDetailJob.error_message?.trim() || t('image.history.requestSnapshotEmpty')}
              </pre>
            </section>

            <section className={styles.requestDetailSection}>
              <div className={styles.requestDetailLabel}>{t('image.history.requestUrl')}</div>
              <pre className={styles.requestDetailCode}>
                {requestDetailJob.request_url?.trim() || t('image.history.requestSnapshotEmpty')}
              </pre>
            </section>

            <section className={styles.requestDetailSection}>
              <div className={styles.requestDetailLabel}>{t('image.history.requestHeaders')}</div>
              <div className={styles.requestDetailEditorWrap}>
                <JsonEditor
                  value={parseRequestSnapshotJson(requestDetailJob.request_headers_json)}
                  readOnly={true}
                  mode="text"
                  height={180}
                  resizable={false}
                  showMainMenuBar={false}
                  showStatusBar={false}
                  placeholder="{}"
                />
              </div>
            </section>

            <section className={styles.requestDetailSection}>
              <div className={styles.requestDetailLabel}>{t('image.history.requestBody')}</div>
              <div className={styles.requestDetailEditorWrap}>
                <JsonEditor
                  value={parseRequestSnapshotJson(requestDetailJob.request_body_json)}
                  readOnly={true}
                  mode="text"
                  height={240}
                  resizable={false}
                  showMainMenuBar={false}
                  showStatusBar={false}
                  placeholder="{}"
                />
              </div>
            </section>
          </div>
        )}
      </Modal>

      <SizePickerModal
        currentSize={formState.size}
        open={sizePickerOpen}
        onClose={() => setSizePickerOpen(false)}
        onSelect={(size) => setFormState((currentFormState) => ({ ...currentFormState, size }))}
      />

      <ImageChannelModal
        open={channelModalOpen}
        draft={channelDraft}
        saving={channelSaving}
        onClose={() => setChannelModalOpen(false)}
        onChange={setChannelDraft}
        onSubmit={handleSaveChannel}
      />
    </div>
  );
};

export default ImagePage;
