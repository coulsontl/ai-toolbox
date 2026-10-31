import React from 'react';
import { Modal, Form, Input, Button, Select, message, Typography, Tag, Divider, Checkbox, InputNumber } from 'antd';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';
import { RightOutlined, DownOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  MODEL_FORM_CHECKBOX_STYLES,
  MODEL_FORM_EXTRA_STYLE,
  MODEL_FORM_LAYOUT,
  MODEL_FORM_MODAL_WIDTH,
  MODEL_FORM_PAIR_STYLE,
} from '@/features/coding/shared/modelFormLayout';
import modelFormStyles from '../../../features/coding/shared/modelFormLayout.module.less';
import JsonEditor from '@/components/common/JsonEditor';
import {
  PRESET_MODELS,
  getPresetModelsVersion,
  subscribePresetModels,
  type PresetModel,
} from '@/constants/presetModels';
import {
  PI_INPUT_TYPES,
  buildPiThinkingLevelMapFromPreset,
} from '@/utils/piModelMetadata';
import { buildOmpThinkingFromPreset } from '@/utils/ompModelMetadata';
import { hasCompleteModelLimitPair } from '@/utils/modelLimits';
import { normalizeVariantsForProviderNpm } from '@/utils/openCodeVariantCompat';

const { Text } = Typography;

/**
 * Labels this form pulls from `common.model.*`. Tools whose wording genuinely
 * differs pass a `messageOverrides` entry for the key; everything else shares
 * the common string.
 */
export type ModelFormMessageKey =
  | 'addModel' | 'editModel' | 'id' | 'idPlaceholder'
  | 'name' | 'namePlaceholder' | 'nameOptionalPlaceholder'
  | 'contextLimit' | 'contextLimitPlaceholder'
  | 'outputLimit' | 'outputLimitPlaceholder'
  | 'inputTypes' | 'inputTypesHint' | 'inputTypesPlaceholder'
  | 'capabilities' | 'capabilitiesHint' | 'reasoning'
  | 'thinkingLevel' | 'thinkingLevelHint' | 'thinkingLevelPlaceholder'
  | 'thinkingLevelMap' | 'thinkingLevelMapHint'
  | 'ompThinking' | 'ompThinkingHint'
  | 'compat' | 'compatHint' | 'extraParams' | 'extraParamsHint'
  | 'api' | 'apiHint' | 'apiPlaceholder'
  | 'cost' | 'costHint' | 'costInput' | 'costOutput' | 'costCacheRead' | 'costCacheWrite'
  | 'invalidCompat' | 'invalidExtraParams' | 'invalidThinkingLevelMap'
  | 'selectPreset' | 'otherPresets'
  | 'inputModalities' | 'inputModalitiesPlaceholder'
  | 'outputModalities' | 'outputModalitiesPlaceholder' | 'modalitiesHint'
  | 'toolCall' | 'temperatureSetting' | 'attachment' | 'attachmentModalitiesHint'
  | 'variants' | 'variantsHint' | 'invalidVariants'
  | 'modalitiesBothRequired' | 'limitsBothRequired';

export type ModelFormMessageOverrides = Partial<Record<ModelFormMessageKey, string>>;

// Context limit options with display labels
const CONTEXT_LIMIT_OPTIONS = [
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
  { value: '128000', label: '128K' },
  { value: '200000', label: '200K' },
  { value: '256000', label: '256K' },
  { value: '1000000', label: '1M' },
  { value: '2000000', label: '2M' },
];

// Output limit options with display labels
const OUTPUT_LIMIT_OPTIONS = [
  { value: '2048', label: '2K' },
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
];

// Modality options for input/output
const MODALITY_OPTIONS = [
  { value: 'text', label: 'Text' },
  { value: 'image', label: 'Image' },
  { value: 'pdf', label: 'PDF' },
  { value: 'video', label: 'Video' },
  { value: 'audio', label: 'Audio' },
];

/**
 * Form values for model form
 */
export interface ModelFormValues {
  id: string;
  name: string;
  api?: string;
  contextLimit?: number;
  outputLimit?: number;
  options?: string;
  variants?: string;
  modalities?: string;
  inputTypes?: string;
  reasoning?: boolean;
  thinkingLevelMap?: string;
  /** Single-value per-model thinking level (Hermes agent.reasoning_overrides). */
  thinkingLevel?: string;
  /** Unknown top-level model fields preserved verbatim (OpenClaw-style extra params). */
  extraParams?: Record<string, unknown>;
  /** OMP model `thinking` structure (efforts / defaultLevel) as JSON string */
  thinking?: string;
  compat?: string;
  attachment?: boolean;
  tool_call?: boolean;
  temperature?: boolean;
  costInput?: number;
  costOutput?: number;
  costCacheRead?: number;
  costCacheWrite?: number;
}

interface ModelFormModalProps {
  open: boolean;

  /** Whether this is an edit operation */
  isEdit?: boolean;
  /** Initial form values */
  initialValues?: Partial<ModelFormValues>;

  /** Existing model IDs for duplicate check (only used when !isEdit) */
  existingIds?: string[];

  /** Whether to show options field (settings page: true, OpenCode: false) */
  showOptions?: boolean;
  /** Whether to show variants field (OpenCode only) */
  showVariants?: boolean;
  /** Whether to show modalities field (OpenCode only) */
  showModalities?: boolean;
  /**
   * OpenCode 2.0 has no `capabilities.attachment` field: media support is
   * expressed through input modalities. In that mode the checkbox is disabled
   * and the user is pointed at the modality selector (issue #430).
   */
  attachmentViaModalities?: boolean;
  /** Whether to show input types as a standalone model field (Pi only) */
  showInputTypes?: boolean;
  /** Whether to show a per-model API override field (Pi only) */
  showApi?: boolean;
  /** API options for the per-model API override field */
  apiOptions?: Array<{ value: string; label: string }>;
  /** Whether to show reasoning as a standalone model capability (Pi only) */
  showReasoning?: boolean;
  /** Whether to show Pi thinking level map JSON */
  showThinkingLevelMap?: boolean;
  /** Whether to show a single-value per-model thinking level Select (Hermes). */
  showThinkingLevel?: boolean;
  /** Options for the single-value thinking level Select. */
  thinkingLevelOptions?: Array<{ value: string; label: string }>;
  /** Whether to show the OMP model `thinking` structure JSON (efforts/defaultLevel) */
  showOmpThinking?: boolean;
  /** Whether to show Pi model compatibility JSON */
  showCompat?: boolean;
  /** Whether to show model cost fields */
  showCost?: boolean;
  /** Whether to show the extra-params JSON editor (OpenClaw-style). */
  showExtraParams?: boolean;
  /** Whether limit fields are required (settings page: true, OpenCode: false) */
  limitRequired?: boolean;
  /** Whether context and output limits must both be filled or both be empty */
  requireCompleteLimitPair?: boolean;
  /** Whether name field is required (settings page: true, OpenCode: false) */
  nameRequired?: boolean;

  /** NPM SDK type for preset models dropdown */
  npmType?: string;
  /** Modal width override */
  width?: number;

  /** Callbacks */
  onCancel: () => void;
  onSuccess: (values: ModelFormValues) => void;
  /** Custom duplicate ID error handler */
  onDuplicateId?: (id: string) => void;

  /**
   * Display name of the tool, interpolated into hints that name it
   * (`capabilitiesHint`, `inputTypesHint`).
   */
  toolName?: string;
  /**
   * Per-key label overrides, as **already-translated text** — pass the result
   * of `t(...)`, not the key.
   *
   *   messageOverrides={{ idPlaceholder: t('dsh.model.idPlaceholder') }}
   *
   * A key stored here would be invisible to `i18n:prune` (it only sees string
   * literals it can trace to a `t()` call), which then deletes it as unused.
   * Most labels need no override at all — they default to `common.model.*`.
   */
  messageOverrides?: ModelFormMessageOverrides;
}

/**
 * A reusable model form modal component
 */
const ModelFormModal: React.FC<ModelFormModalProps> = ({
  open,
  isEdit = false,
  initialValues,
  existingIds = [],
  showOptions = true,
  showVariants = false,
  showModalities = false,
  attachmentViaModalities = false,
  showInputTypes = false,
  showApi = false,
  apiOptions = [],
  showReasoning = false,
  showThinkingLevelMap = false,
  showThinkingLevel = false,
  thinkingLevelOptions = [],
  showOmpThinking = false,
  showCompat = false,
  showCost = false,
  showExtraParams = false,
  limitRequired = true,
  requireCompleteLimitPair = false,
  nameRequired = true,
  npmType,
  width,
  onCancel,
  onSuccess,
  onDuplicateId,
  toolName,
  messageOverrides,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [jsonOptions, setJsonOptions] = React.useState<unknown>({});
  const [jsonValid, setJsonValid] = React.useState(true);
  const [jsonVariants, setJsonVariants] = React.useState<unknown>({});
  const [variantsValid, setVariantsValid] = React.useState(true);
  const [jsonThinkingLevelMap, setJsonThinkingLevelMap] = React.useState<unknown>({});
  const [thinkingLevelMapValid, setThinkingLevelMapValid] = React.useState(true);
  const [jsonOmpThinking, setJsonOmpThinking] = React.useState<unknown>({});
  const [ompThinkingValid, setOmpThinkingValid] = React.useState(true);
  const [jsonCompat, setJsonCompat] = React.useState<unknown>({});
  const [compatValid, setCompatValid] = React.useState(true);
  const [extraParamsValue, setExtraParamsValue] = React.useState<unknown>(undefined);
  const [extraParamsValid, setExtraParamsValid] = React.useState(true);
  const [inputModalities, setInputModalities] = React.useState<string[]>([]);
  const [outputModalities, setOutputModalities] = React.useState<string[]>([]);
  const [advancedExpanded, setAdvancedExpanded] = React.useState(false);
  const [presetsExpanded, setPresetsExpanded] = React.useState(false);
  const [capReasoning, setCapReasoning] = React.useState(true);
  const [capAttachment, setCapAttachment] = React.useState(false);
  const [capToolCall, setCapToolCall] = React.useState(true);
  const [capTemperature, setCapTemperature] = React.useState(true);
  const apiValue = Form.useWatch('api', form);
  const costInputValue = Form.useWatch('costInput', form);
  const costOutputValue = Form.useWatch('costOutput', form);
  const costCacheReadValue = Form.useWatch('costCacheRead', form);
  const costCacheWriteValue = Form.useWatch('costCacheWrite', form);
  const presetModelsVersion = React.useSyncExternalStore(
    subscribePresetModels,
    getPresetModelsVersion,
    getPresetModelsVersion,
  );

  const presetModels = React.useMemo(() => {
    if (!npmType) return [];
    return PRESET_MODELS[npmType] || [];
  }, [npmType, presetModelsVersion]);

  const otherPresetModels = React.useMemo(() => {
    if (!npmType) return [];
    return Object.entries(PRESET_MODELS)
      .filter(([type]) => type !== npmType)
      .flatMap(([, models]) => models);
  }, [npmType, presetModelsVersion]);

  const handlePresetSelect = (preset: PresetModel) => {
    // When editing, don't override the model ID
    if (isEdit) {
      form.setFieldsValue({
        name: preset.name,
        contextLimit: preset.contextLimit,
        outputLimit: preset.outputLimit,
      });
    } else {
      form.setFieldsValue({
        id: preset.id,
        name: preset.name,
        contextLimit: preset.contextLimit,
        outputLimit: preset.outputLimit,
      });
    }

    // Set options if present
    if (showOptions && preset.options && Object.keys(preset.options).length > 0) {
      setJsonOptions(preset.options);
      setJsonValid(true);
    }

    // Set variants if present
    if (showVariants && preset.variants && Object.keys(preset.variants).length > 0) {
      setJsonVariants(normalizeVariantsForProviderNpm(preset.variants, npmType));
      setVariantsValid(true);
      // Auto expand advanced settings if variants has content
      setAdvancedExpanded(true);
    }

    if (showThinkingLevelMap) {
      const nextThinkingLevelMap = buildPiThinkingLevelMapFromPreset(preset.variants);
      setJsonThinkingLevelMap(nextThinkingLevelMap);
      setThinkingLevelMapValid(true);
      if (Object.keys(nextThinkingLevelMap).length > 0) {
        setAdvancedExpanded(true);
      }
    }

    if (showOmpThinking) {
      const nextOmpThinking = buildOmpThinkingFromPreset(preset.variants);
      setJsonOmpThinking(nextOmpThinking ?? {});
      setOmpThinkingValid(true);
      if (nextOmpThinking && Object.keys(nextOmpThinking).length > 0) {
        setAdvancedExpanded(true);
      }
    }

    // Set modalities if present
    if (preset.modalities) {
      if (preset.modalities.input) {
        setInputModalities(showInputTypes
          ? preset.modalities.input.filter((item) => PI_INPUT_TYPES.has(item))
          : preset.modalities.input);
      }
      if (preset.modalities.output) {
        setOutputModalities(preset.modalities.output);
      }
      // Auto expand advanced settings if modalities has content
      if ((preset.modalities.input && preset.modalities.input.length > 0) ||
          (preset.modalities.output && preset.modalities.output.length > 0)) {
        setAdvancedExpanded(true);
      }
    }

    // Set capability fields from preset
    if (showModalities || showReasoning) {
      setCapReasoning(preset.reasoning !== undefined ? preset.reasoning : true);
    }
    if (showModalities) {
      setCapAttachment(preset.attachment !== undefined ? preset.attachment : false);
      setCapToolCall(preset.tool_call !== undefined ? preset.tool_call : true);
      setCapTemperature(preset.temperature !== undefined ? preset.temperature : true);
    }

    setPresetsExpanded(false);
  };


  // Check if options or variants or modalities has content
  const hasAdvancedContent = React.useMemo(() => {
    const hasOptions = typeof jsonOptions === 'object' && jsonOptions !== null &&
      Object.keys(jsonOptions).length > 0;
    const hasVariants = showVariants &&
      typeof jsonVariants === 'object' && jsonVariants !== null &&
      Object.keys(jsonVariants as object).length > 0;
    const hasModalities = showModalities &&
      (inputModalities.length > 0 || outputModalities.length > 0);
    const hasThinkingLevelMap = showThinkingLevelMap &&
      typeof jsonThinkingLevelMap === 'object' && jsonThinkingLevelMap !== null &&
      Object.keys(jsonThinkingLevelMap as object).length > 0;
    const hasOmpThinking = showOmpThinking &&
      typeof jsonOmpThinking === 'object' && jsonOmpThinking !== null &&
      Object.keys(jsonOmpThinking as object).length > 0;
    const hasCompat = showCompat &&
      typeof jsonCompat === 'object' && jsonCompat !== null &&
      Object.keys(jsonCompat as object).length > 0;
    const hasExtraParams = showExtraParams &&
      typeof extraParamsValue === 'object' && extraParamsValue !== null &&
      Object.keys(extraParamsValue as object).length > 0;
    const hasApi = showApi && typeof apiValue === 'string' && apiValue.trim() !== '';
    const hasCost = showCost && [
      costInputValue,
      costOutputValue,
      costCacheReadValue,
      costCacheWriteValue,
    ].some((value) => typeof value === 'number');
    return hasOptions || hasVariants || hasModalities || hasThinkingLevelMap || hasOmpThinking || hasCompat || hasExtraParams || hasApi || hasCost;
  }, [
    jsonOptions,
    jsonVariants,
    jsonThinkingLevelMap,
    jsonOmpThinking,
    jsonCompat,
    extraParamsValue,
    apiValue,
    costInputValue,
    costOutputValue,
    costCacheReadValue,
    costCacheWriteValue,
    inputModalities,
    outputModalities,
    showVariants,
    showModalities,
    showThinkingLevelMap,
    showCompat,
    showApi,
    showCost,
  ]);

  React.useEffect(() => {
    if (open) {
      if (initialValues) {
        form.setFieldsValue({
          id: initialValues.id,
          name: initialValues.name,
          api: initialValues.api,
          contextLimit: initialValues.contextLimit,
          outputLimit: initialValues.outputLimit,
          thinkingLevel: initialValues.thinkingLevel,
          costInput: initialValues.costInput,
          costOutput: initialValues.costOutput,
          costCacheRead: initialValues.costCacheRead,
          costCacheWrite: initialValues.costCacheWrite,
        });
        
        let shouldExpand = false;
        if (typeof initialValues.api === 'string' && initialValues.api.trim() !== '') {
          shouldExpand = true;
        }
        if (typeof initialValues.thinkingLevel === 'string' && initialValues.thinkingLevel.trim() !== '') {
          shouldExpand = true;
        }
        if ([
          initialValues.costInput,
          initialValues.costOutput,
          initialValues.costCacheRead,
          initialValues.costCacheWrite,
        ].some((value) => typeof value === 'number')) {
          shouldExpand = true;
        }
        
        // Parse options JSON
        if (initialValues.options) {
          try {
            const parsed = JSON.parse(initialValues.options);
            setJsonOptions(parsed);
            setJsonValid(true);
            // Auto expand if options has content
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonOptions({});
            setJsonValid(false);
          }
        } else {
          setJsonOptions({});
          setJsonValid(true);
        }
        
        // Parse variants JSON
        if (initialValues.variants) {
          try {
            const parsed = JSON.parse(initialValues.variants);
            setJsonVariants(parsed);
            setVariantsValid(true);
            // Auto expand if variants has content
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonVariants({});
            setVariantsValid(false);
          }
        } else {
          setJsonVariants({});
          setVariantsValid(true);
        }

        // Parse thinking level map JSON
        if (initialValues.thinkingLevelMap) {
          try {
            const parsed = JSON.parse(initialValues.thinkingLevelMap);
            setJsonThinkingLevelMap(parsed);
            setThinkingLevelMapValid(true);
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonThinkingLevelMap({});
            setThinkingLevelMapValid(false);
          }
        } else {
          setJsonThinkingLevelMap({});
          setThinkingLevelMapValid(true);
        }

        // Parse OMP model thinking JSON
        if (initialValues.thinking) {
          try {
            const parsed = JSON.parse(initialValues.thinking);
            setJsonOmpThinking(parsed);
            setOmpThinkingValid(true);
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonOmpThinking({});
            setOmpThinkingValid(false);
          }
        } else {
          setJsonOmpThinking({});
          setOmpThinkingValid(true);
        }

        // Parse compat JSON
        if (initialValues.compat) {
          try {
            const parsed = JSON.parse(initialValues.compat);
            setJsonCompat(parsed);
            setCompatValid(true);
            if (typeof parsed === 'object' && parsed !== null && Object.keys(parsed).length > 0) {
              shouldExpand = true;
            }
          } catch {
            setJsonCompat({});
            setCompatValid(false);
          }
        } else {
          setJsonCompat({});
          setCompatValid(true);
        }

        // Extra params (object, not a JSON string — OpenClaw-style unknown-field passthrough)
        if (showExtraParams) {
          const extra = initialValues.extraParams;
          if (extra && typeof extra === 'object' && Object.keys(extra).length > 0) {
            setExtraParamsValue(extra);
            setExtraParamsValid(true);
            shouldExpand = true;
          } else {
            setExtraParamsValue(undefined);
            setExtraParamsValid(true);
          }
        }

        // Parse modalities JSON
        if (initialValues.modalities) {
          try {
            const parsed = JSON.parse(initialValues.modalities);
            if (parsed && typeof parsed === 'object') {
              if (Array.isArray(parsed.input)) {
                setInputModalities(parsed.input);
              }
              if (Array.isArray(parsed.output)) {
                setOutputModalities(parsed.output);
              }
              // Auto expand if modalities has content
              if ((parsed.input && parsed.input.length > 0) || (parsed.output && parsed.output.length > 0)) {
                shouldExpand = true;
              }
            }
          } catch {
            setInputModalities([]);
            setOutputModalities([]);
          }
        } else {
          setInputModalities([]);
          setOutputModalities([]);
        }

        if (showInputTypes && initialValues.inputTypes) {
          try {
            const parsed = JSON.parse(initialValues.inputTypes);
            setInputModalities(Array.isArray(parsed) ? parsed : []);
          } catch {
            setInputModalities([]);
          }
        }
        
        setAdvancedExpanded(shouldExpand);

        // Set capability fields (default to false when editing/copying existing model without values)
        if (showModalities || showReasoning) {
          setCapReasoning(initialValues.reasoning !== undefined ? initialValues.reasoning : false);
        }
        if (showModalities) {
          setCapAttachment(initialValues.attachment !== undefined ? initialValues.attachment : false);
          setCapToolCall(initialValues.tool_call !== undefined ? initialValues.tool_call : false);
          setCapTemperature(initialValues.temperature !== undefined ? initialValues.temperature : false);
        }
      } else {
        form.resetFields();
        setJsonOptions({});
        setJsonValid(true);
        setJsonVariants({});
        setVariantsValid(true);
        setJsonThinkingLevelMap({});
        setThinkingLevelMapValid(true);
        setJsonCompat({});
        setCompatValid(true);
        setExtraParamsValue(undefined);
        setExtraParamsValid(true);
        setInputModalities([]);
        setOutputModalities([]);
        setAdvancedExpanded(true);
        setCapReasoning(true);
        setCapAttachment(false);
        setCapToolCall(true);
        setCapTemperature(true);
      }
    }
  }, [open, initialValues, form]);

  const handleJsonChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonOptions(value);
    }
    setJsonValid(isValid);
  };

  const handleExtraParamsChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setExtraParamsValue(value);
    }
    setExtraParamsValid(isValid);
  };

  const handleVariantsChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonVariants(value);
    }
    setVariantsValid(isValid);
  };

  const handleThinkingLevelMapChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonThinkingLevelMap(value);
    }
    setThinkingLevelMapValid(isValid);
  };

  const handleOmpThinkingChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonOmpThinking(value);
    }
    setOmpThinkingValid(isValid);
  };

  const handleCompatChange = (value: unknown, isValid: boolean) => {
    if (isValid) {
      setJsonCompat(value);
    }
    setCompatValid(isValid);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      
      // Validate JSON if showing options
      if (showOptions && !jsonValid) {
        message.error(t('settings.model.invalidJson'));
        return;
      }
      
      // Validate variants JSON if showing variants
      if (showVariants && !variantsValid) {
        message.error(t('common.model.invalidVariants'));
        return;
      }

      if (showThinkingLevelMap && !thinkingLevelMapValid) {
        message.error(getKey('invalidThinkingLevelMap'));
        return;
      }

      if (showOmpThinking && !ompThinkingValid) {
        message.error(getKey('invalidThinkingLevelMap'));
        return;
      }

      if (showCompat && !compatValid) {
        message.error(getKey('invalidCompat'));
        return;
      }

      if (showExtraParams && !extraParamsValid) {
        message.error(getKey('invalidExtraParams'));
        return;
      }

      // Validate modalities: either both selected or both empty
      if (showModalities) {
        const hasInput = inputModalities.length > 0;
        const hasOutput = outputModalities.length > 0;
        if (hasInput !== hasOutput) {
          message.error(t('common.model.modalitiesBothRequired'));
          return;
        }
      }

      if (requireCompleteLimitPair) {
        if (!hasCompleteModelLimitPair(values.contextLimit, values.outputLimit)) {
          message.error(t('common.model.limitsBothRequired'));
          return;
        }
      }

      setLoading(true);

      // Check for duplicate ID when creating
      if (!isEdit && existingIds.includes(values.id)) {
        if (onDuplicateId) {
          onDuplicateId(values.id);
        }
        setLoading(false);
        return;
      }

      const result: ModelFormValues = {
        id: values.id,
        name: values.name,
        api: values.api,
        contextLimit: values.contextLimit,
        outputLimit: values.outputLimit,
      };

      if (showOptions) {
        result.options = JSON.stringify(jsonOptions);
      }
      
      if (showVariants) {
        result.variants = JSON.stringify(jsonVariants);
      }

      if (showThinkingLevelMap) {
        result.thinkingLevelMap = JSON.stringify(jsonThinkingLevelMap);
      }

      if (showThinkingLevel) {
        result.thinkingLevel = values.thinkingLevel;
      }

      if (showOmpThinking) {
        result.thinking = JSON.stringify(jsonOmpThinking);
      }

      if (showCompat) {
        result.compat = JSON.stringify(jsonCompat);
      }

      if (showExtraParams && extraParamsValid
        && typeof extraParamsValue === 'object' && extraParamsValue !== null
        && Object.keys(extraParamsValue as object).length > 0) {
        result.extraParams = extraParamsValue as Record<string, unknown>;
      }

      if (showModalities && inputModalities.length > 0 && outputModalities.length > 0) {
        result.modalities = JSON.stringify({
          input: inputModalities,
          output: outputModalities,
        });
      }

      if (showInputTypes && inputModalities.length > 0) {
        result.inputTypes = JSON.stringify(inputModalities);
      }

      if (showModalities || showReasoning) {
        result.reasoning = capReasoning;
      }

      if (showCost) {
        result.costInput = values.costInput;
        result.costOutput = values.costOutput;
        result.costCacheRead = values.costCacheRead;
        result.costCacheWrite = values.costCacheWrite;
      }

      if (showModalities) {
        // V2 has no attachment flag: the input modality list is the source of
        // truth, so the field is neither shown nor submitted. The V1 checkbox
        // state is `capAttachment`, seeded from presets/initial values above.
        if (!attachmentViaModalities) {
          result.attachment = capAttachment;
        }
        result.tool_call = capToolCall;
        result.temperature = capTemperature;
      }

      onSuccess(result);
      form.resetFields();
    } catch (error: unknown) {
      console.error('Model form validation error:', error);
      // Form validation errors are already shown by Form
    } finally {
      setLoading(false);
    }
  };

  /**
   * Resolves a label: a caller override is already-translated text, otherwise
   * fall back to the shared `common.model.*` key. Returning the override
   * verbatim (rather than a key) keeps the strings visible to the i18n
   * checker — an override holding a key would be invisible to static analysis
   * and get pruned as unused.
   */
  const getKey = (key: ModelFormMessageKey, params?: Record<string, unknown>) =>
    messageOverrides?.[key] ?? t(`common.model.${key}`, params);

  const limitRules = limitRequired
    ? [
        { required: true, message: getKey('contextLimitPlaceholder') },
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ]
    : [
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ];

  const outputLimitRules = limitRequired
    ? [
        { required: true, message: getKey('outputLimitPlaceholder') },
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ]
    : [
        {
          validator: (_: unknown, value: unknown) => {
            if (value && !/^\d+$/.test(String(value))) {
              return Promise.reject(t('settings.model.invalidNumber'));
            }
            return Promise.resolve();
          },
        },
      ];

  return (
    <Modal
      title={isEdit ? getKey('editModel') : getKey('addModel')}
      open={open}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" loading={loading} onClick={handleSubmit}>
          {t('common.save')}
        </Button>,
      ]}
      // Fields now span the dialog, so the width is what decides whether the
      // JSON editors are readable (see `shared/modelFormLayout.ts`).
      width={width ?? MODEL_FORM_MODAL_WIDTH}
    >
      <Form
        form={form}
        layout={MODEL_FORM_LAYOUT}
        // Labels were the same size and weight as the values inside the inputs,
        // so nothing separated "what this field is" from "what is in it".
        classNames={{ label: modelFormStyles.label }}
        styles={{ extra: MODEL_FORM_EXTRA_STYLE }}
        style={{ marginTop: 24 }}
      >
        <Form.Item
          label={getKey('id')}
          required
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Form.Item
              name="id"
              noStyle
              rules={[{ required: true, message: getKey('idPlaceholder') }]}
            >
              <Input
                placeholder={getKey('idPlaceholder')}
                disabled={isEdit}
                style={{ flex: 1 }}
              />
            </Form.Item>
            {npmType && presetModels.length > 0 && (
              <a
                style={{
                  flexShrink: 0,
                  fontSize: 12,
                  fontWeight: 500,
                  color: 'var(--ant-color-text-secondary)',
                  cursor: 'pointer',
                  userSelect: 'none',
                  whiteSpace: 'nowrap',
                }}
                onClick={() => setPresetsExpanded(!presetsExpanded)}
              >
                {t('common.model.selectPreset')}
                {presetsExpanded ? ' ▴' : ' ▾'}
              </a>
            )}
          </div>
        </Form.Item>

        {presetsExpanded && presetModels.length > 0 && (
          <Form.Item style={{ marginTop: -8 }}>
            {/* Capped and scrollable: the catalogue is over 100 entries, and
                letting it run the full height pushed every other field below the
                fold. */}
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, maxHeight: 168, overflowY: 'auto', paddingRight: 4 }}>
              {presetModels.map((preset) => (
                <Tag
                  key={preset.id}
                  style={{
                    cursor: 'pointer',
                    transition: 'all 0.2s',
                  }}
                  onClick={() => handlePresetSelect(preset)}
                >
                  {preset.name}
                </Tag>
              ))}
            </div>
            {otherPresetModels.length > 0 && (
              <>
                <Divider style={{ margin: '12px 0', fontSize: 12, color: 'var(--color-text-tertiary)' }}>
                  {t('common.model.otherPresets')}
                </Divider>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                  {otherPresetModels.map((preset) => (
                    <Tag
                      key={preset.id}
                      style={{
                        cursor: 'pointer',
                        transition: 'all 0.2s',
                      }}
                      onClick={() => handlePresetSelect(preset)}
                    >
                      {preset.name}
                    </Tag>
                  ))}
                </div>
              </>
            )}
          </Form.Item>
        )}

        <Form.Item
          label={getKey('name')}
          name="name"
          rules={nameRequired ? [{ required: true, message: getKey('namePlaceholder') }] : []}
        >
          <Input placeholder={nameRequired ? getKey('namePlaceholder') : getKey('nameOptionalPlaceholder')} />
        </Form.Item>

        {/* The two limits are one fact split in two, so they share a row: stacked,
            each six-digit input sat alone on a full-width line and the dialog grew
            past the viewport. Both `Form.Item`s stay direct children of the grid —
            wrapping one in another item would break its label association. */}
        <div style={MODEL_FORM_PAIR_STYLE}>
          <Form.Item
            label={getKey('contextLimit')}
            name="contextLimit"
            rules={limitRules}
            getValueFromEvent={(val) => {
              const num = parseInt(val, 10);
              return isNaN(num) ? undefined : num;
            }}
          >
            <ImeSafeAutoComplete
              options={CONTEXT_LIMIT_OPTIONS}
              placeholder={getKey('contextLimitPlaceholder')}
              style={{ width: '100%' }}
              filterOption={(inputValue, option) =>
                (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
                option?.value.includes(inputValue)) ?? false
              }
            />
          </Form.Item>

          <Form.Item
            label={getKey('outputLimit')}
            name="outputLimit"
            rules={outputLimitRules}
            getValueFromEvent={(val) => {
              const num = parseInt(val, 10);
              return isNaN(num) ? undefined : num;
            }}
          >
            <ImeSafeAutoComplete
              options={OUTPUT_LIMIT_OPTIONS}
              placeholder={getKey('outputLimitPlaceholder')}
              style={{ width: '100%' }}
              filterOption={(inputValue, option) =>
                (option?.label.toLowerCase().includes(inputValue.toLowerCase()) ||
                option?.value.includes(inputValue)) ?? false
              }
            />
          </Form.Item>
        </div>

        {showInputTypes && (
          <Form.Item
            label={getKey('inputTypes')}
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('inputTypesHint', { tool: toolName ?? '' })}</Text>}
          >
            <Select
              mode="multiple"
              allowClear
              placeholder={getKey('inputTypesPlaceholder')}
              options={MODALITY_OPTIONS.filter((option) => option.value === 'text' || option.value === 'image')}
              value={inputModalities}
              onChange={setInputModalities}
            />
          </Form.Item>
        )}

        {showReasoning && !showModalities && (
          <Form.Item
            label={getKey('capabilities')}
            extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('capabilitiesHint', { tool: toolName ?? '' })}</Text>}
          >
            <Checkbox styles={MODEL_FORM_CHECKBOX_STYLES} checked={capReasoning} onChange={(e) => setCapReasoning(e.target.checked)}>
              {getKey('reasoning')}
            </Checkbox>
          </Form.Item>
        )}

        {(showOptions || showVariants || showModalities || showThinkingLevelMap || showThinkingLevel || showOmpThinking || showCompat || showExtraParams || showApi || showCost) && (
          <>
            <div style={{ marginBottom: advancedExpanded ? 16 : 0 }}>
              <Button
                type="link"
                onClick={() => setAdvancedExpanded(!advancedExpanded)}
                style={{ padding: 0, height: 'auto' }}
              >
                {advancedExpanded ? <DownOutlined /> : <RightOutlined />}
                <span style={{ marginLeft: 4 }}>
                  {t('common.advancedSettings')}
                  {hasAdvancedContent && !advancedExpanded && (
                    <span style={{ marginLeft: 4, color: '#1890ff' }}>*</span>
                  )}
                </span>
              </Button>
            </div>
            {advancedExpanded && (
              <>
                {showModalities && (
                  <>
                    {/* Input and output are one fact — the shape of the payload
                        the model accepts and returns — so they share a row the
                        same way the two limits do. Stacked they were two
                        full-width select boxes and half a screen of chrome for
                        two short tag lists (user-directed, 2026-11-01:
                        「也和上下文一样，改成左右排列，它们是一组的」).

                        The hint belongs to the pair, not to 输出 alone: it
                        describes both fields, so it sits under the row instead
                        of under only the right-hand box. */}
                    <div style={MODEL_FORM_PAIR_STYLE}>
                      <Form.Item
                        label={t('common.model.inputModalities')}
                      >
                        <Select
                          mode="multiple"
                          allowClear
                          placeholder={t('common.model.inputModalitiesPlaceholder')}
                          options={MODALITY_OPTIONS}
                          value={inputModalities}
                          onChange={setInputModalities}
                        />
                      </Form.Item>
                      <Form.Item
                        label={t('common.model.outputModalities')}
                      >
                        <Select
                          mode="multiple"
                          allowClear
                          placeholder={t('common.model.outputModalitiesPlaceholder')}
                          options={MODALITY_OPTIONS}
                          value={outputModalities}
                          onChange={setOutputModalities}
                        />
                      </Form.Item>
                    </div>
                    <Form.Item style={{ marginTop: -12 }}>
                      <Text type="secondary" style={{ fontSize: 12 }}>{t('common.model.modalitiesHint')}</Text>
                    </Form.Item>
                    <Form.Item
                      label={t('common.model.capabilities')}
                      extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('common.model.capabilitiesHint')}</Text>}
                    >
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 16 }}>
                        <Checkbox styles={MODEL_FORM_CHECKBOX_STYLES} checked={capReasoning} onChange={(e) => setCapReasoning(e.target.checked)}>
                          {t('common.model.reasoning')}
                        </Checkbox>
                        <Checkbox styles={MODEL_FORM_CHECKBOX_STYLES} checked={capToolCall} onChange={(e) => setCapToolCall(e.target.checked)}>
                          {t('common.model.toolCall')}
                        </Checkbox>
                        <Checkbox styles={MODEL_FORM_CHECKBOX_STYLES} checked={capTemperature} onChange={(e) => setCapTemperature(e.target.checked)}>
                          {t('common.model.temperatureSetting')}
                        </Checkbox>
                        <Checkbox
                          styles={MODEL_FORM_CHECKBOX_STYLES}
                          checked={attachmentViaModalities
                            ? inputModalities.some((item) => item === 'image' || item === 'pdf')
                            : capAttachment}
                          disabled={attachmentViaModalities}
                          onChange={(event) => setCapAttachment(event.target.checked)}
                        >
                          {t('common.model.attachment')}
                        </Checkbox>
                      </div>
                      {attachmentViaModalities && (
                        <Text type="secondary" style={{ fontSize: 12 }}>
                          {messageOverrides?.attachmentModalitiesHint
                            ?? t('common.model.attachmentModalitiesHint')}
                        </Text>
                      )}
                    </Form.Item>
                  </>
                )}

                {showVariants && (
                  <Form.Item
                    label={t('common.model.variants')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{t('common.model.variantsHint')}</Text>}
                  >
                    <JsonEditor
                      value={typeof jsonVariants === 'object' && jsonVariants !== null && Object.keys(jsonVariants as object).length === 0 ? undefined : jsonVariants}
                      onChange={handleVariantsChange}
                      mode="text"
                      height={200}
                      resizable
                      placeholder={`{
    "minimal": { "thinkingLevel": "minimal" },
    "low": { "thinkingLevel": "low" },
    "medium": { "thinkingLevel": "medium" },
    "high": { "thinkingLevel": "high" }
}`}
                    />
                  </Form.Item>
                )}

                {showThinkingLevel && (
                  <Form.Item
                    label={getKey('thinkingLevel')}
                    name="thinkingLevel"
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('thinkingLevelHint')}</Text>}
                  >
                    <Select
                      allowClear
                      showSearch
                      optionFilterProp="label"
                      placeholder={getKey('thinkingLevelPlaceholder')}
                      options={thinkingLevelOptions}
                    />
                  </Form.Item>
                )}

                {showThinkingLevelMap && (
                  <Form.Item
                    label={getKey('thinkingLevelMap')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('thinkingLevelMapHint')}</Text>}
                  >
                    <JsonEditor
                      value={typeof jsonThinkingLevelMap === 'object' && jsonThinkingLevelMap !== null && Object.keys(jsonThinkingLevelMap as object).length === 0 ? undefined : jsonThinkingLevelMap}
                      onChange={handleThinkingLevelMapChange}
                      mode="text"
                      height={180}
                      resizable
                      placeholder={`{
    "minimal": null,
    "low": null,
    "medium": null,
    "high": "high",
    "xhigh": "xhigh",
    "max": "max"
}`}
                    />
                  </Form.Item>
                )}

                {showOmpThinking && (
                  <Form.Item
                    label={getKey('ompThinking')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('ompThinkingHint')}</Text>}
                  >
                    <JsonEditor
                      value={typeof jsonOmpThinking === 'object' && jsonOmpThinking !== null && Object.keys(jsonOmpThinking as object).length === 0 ? undefined : jsonOmpThinking}
                      onChange={handleOmpThinkingChange}
                      mode="text"
                      height={180}
                      resizable
                      placeholder={`{
    "mode": "effort",
    "efforts": ["low", "medium", "high"],
    "defaultLevel": "high"
}`}
                    />
                  </Form.Item>
                )}

                {showCompat && (
                  <Form.Item
                    label={getKey('compat')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('compatHint')}</Text>}
                  >
                    <JsonEditor
                      value={typeof jsonCompat === 'object' && jsonCompat !== null && Object.keys(jsonCompat as object).length === 0 ? undefined : jsonCompat}
                      onChange={handleCompatChange}
                      mode="text"
                      height={180}
                      resizable
                      placeholder={`{
    "supportsDeveloperRole": false,
    "supportsReasoningEffort": false
}`}
                    />
                  </Form.Item>
                )}

                {showExtraParams && (
                  <Form.Item
                    label={getKey('extraParams')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('extraParamsHint')}</Text>}
                  >
                    <JsonEditor
                      value={typeof extraParamsValue === 'object' && extraParamsValue !== null && Object.keys(extraParamsValue as object).length === 0 ? undefined : extraParamsValue}
                      onChange={handleExtraParamsChange}
                      mode="text"
                      height={150}
                      minHeight={100}
                      maxHeight={300}
                      resizable
                      placeholder={`{
    "input": ["text", "image"],
    "compat": { "streaming": true }
}`}
                    />
                  </Form.Item>
                )}

                {showApi && (
                  <Form.Item
                    label={getKey('api')}
                    name="api"
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('apiHint')}</Text>}
                  >
                    <Select
                      allowClear
                      showSearch
                      placeholder={getKey('apiPlaceholder')}
                      options={apiOptions}
                    />
                  </Form.Item>
                )}

                {showCost && (
                  <Form.Item
                    label={getKey('cost')}
                    extra={<Text type="secondary" style={{ fontSize: 12 }}>{getKey('costHint')}</Text>}
                  >
                    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: 12 }}>
                      <Form.Item name="costInput" label={getKey('costInput')} noStyle>
                        <InputNumber min={0} placeholder="0" style={{ width: '100%' }} addonBefore={getKey('costInput')} />
                      </Form.Item>
                      <Form.Item name="costOutput" label={getKey('costOutput')} noStyle>
                        <InputNumber min={0} placeholder="0" style={{ width: '100%' }} addonBefore={getKey('costOutput')} />
                      </Form.Item>
                      <Form.Item name="costCacheRead" label={getKey('costCacheRead')} noStyle>
                        <InputNumber min={0} placeholder="0" style={{ width: '100%' }} addonBefore={getKey('costCacheRead')} />
                      </Form.Item>
                      <Form.Item name="costCacheWrite" label={getKey('costCacheWrite')} noStyle>
                        <InputNumber min={0} placeholder="0" style={{ width: '100%' }} addonBefore={getKey('costCacheWrite')} />
                      </Form.Item>
                    </div>
                  </Form.Item>
                )}

                {showOptions && (
                  <Form.Item label={t('settings.model.options')}>
                    <JsonEditor
                      value={typeof jsonOptions === 'object' && jsonOptions !== null && Object.keys(jsonOptions).length === 0 ? undefined : jsonOptions}
                      onChange={handleJsonChange}
                      mode="text"
                      height={200}
                      resizable
                      placeholder={`{
    "store": false
}`}
                    />
                  </Form.Item>
                )}
              </>
            )}
          </>
        )}
      </Form>
    </Modal>
  );
};

export default ModelFormModal;
