import React from 'react';
import { Modal, Button, Typography, message } from 'antd';
import { useTranslation } from 'react-i18next';
import JSON5 from 'json5';
import JsonEditor from '@/components/common/JsonEditor';

const { Text } = Typography;

export interface ImportedConfigData {
  agents?: Record<string, Record<string, unknown>>;
  categories?: Record<string, Record<string, unknown>>;
  otherFields?: Record<string, unknown>;
}

const isPlainObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const deepMergeObjects = (
  base?: Record<string, unknown>,
  override?: Record<string, unknown>,
): Record<string, unknown> | undefined => {
  if (!base) return override;
  if (!override) return base;

  const result: Record<string, unknown> = { ...base };

  Object.entries(override).forEach(([key, overrideValue]) => {
    const baseValue = result[key];
    if (isPlainObject(baseValue) && isPlainObject(overrideValue)) {
      result[key] = deepMergeObjects(baseValue, overrideValue);
      return;
    }
    result[key] = overrideValue;
  });

  return result;
};

const resolveSlimImportedAgents = (
  config: Record<string, unknown>,
): Record<string, Record<string, unknown>> | undefined => {
  const rootAgents = isPlainObject(config.agents)
    ? config.agents as Record<string, Record<string, unknown>>
    : undefined;

  const activePresetName = typeof config.preset === 'string' ? config.preset.trim() : '';
  const presets = isPlainObject(config.presets) ? config.presets : undefined;
  const presetAgents = activePresetName && presets && isPlainObject(presets[activePresetName])
    ? presets[activePresetName] as Record<string, Record<string, unknown>>
    : undefined;

  if (!presetAgents) {
    return rootAgents;
  }

  return deepMergeObjects(presetAgents, rootAgents) as Record<string, Record<string, unknown>>;
};

interface ImportJsonConfigModalProps {
  open: boolean;
  onCancel: () => void;
  onImport: (data: ImportedConfigData, mode: 'core' | 'full') => void;
  /** 'omo' for Oh My OpenAgent, 'omos' for Oh My OpenCode Slim */
  variant: 'omo' | 'omos';
}

const ImportJsonConfigModal: React.FC<ImportJsonConfigModalProps> = ({
  open,
  onCancel,
  onImport,
  variant,
}) => {
  const { t } = useTranslation();
  const jsonRawRef = React.useRef<string>('');
  const [parsed, setParsed] = React.useState<{
    agents?: Record<string, Record<string, unknown>>;
    categories?: Record<string, Record<string, unknown>>;
    otherFields?: Record<string, unknown>;
  } | null>(null);

  // Reset state when modal opens/closes
  React.useEffect(() => {
    if (!open) {
      jsonRawRef.current = '';
      setParsed(null);
    }
  }, [open]);

  const handleParse = () => {
    const raw = jsonRawRef.current.trim();
    if (!raw) {
      message.warning(t('opencode.ohMyOpenCode.importFromJsonEmpty'));
      return;
    }

    let obj: unknown;
    try {
      obj = JSON5.parse(raw);
    } catch {
      message.error(t('opencode.ohMyOpenCode.importFromJsonInvalidFormat'));
      return;
    }

    if (typeof obj !== 'object' || obj === null || Array.isArray(obj)) {
      message.error(t('opencode.ohMyOpenCode.importFromJsonInvalidFormat'));
      return;
    }

    const config = obj as Record<string, unknown>;

    const agents = variant === 'omos'
      ? resolveSlimImportedAgents(config)
      : (isPlainObject(config.agents)
        ? config.agents as Record<string, Record<string, unknown>>
        : undefined);

    const categories = isPlainObject(config.categories)
      ? config.categories as Record<string, Record<string, unknown>>
      : undefined;

    // Collect other fields (everything except agents, categories, $schema)
    const otherFields: Record<string, unknown> = {};
    Object.entries(config).forEach(([key, value]) => {
      if (
        key !== 'agents' &&
        key !== 'categories' &&
        key !== '$schema' &&
        !(variant === 'omos' && (key === 'preset' || key === 'presets'))
      ) {
        otherFields[key] = value;
      }
    });

    if (!agents && !categories) {
      message.warning(t('opencode.ohMyOpenCode.importFromJsonEmpty'));
      return;
    }

    setParsed({
      agents,
      categories,
      otherFields: Object.keys(otherFields).length > 0 ? otherFields : undefined,
    });
  };

  const handleImport = (mode: 'core' | 'full') => {
    if (!parsed) return;
    onImport(
      {
        agents: parsed.agents,
        categories: parsed.categories,
        otherFields: mode === 'full' ? parsed.otherFields : undefined,
      },
      mode,
    );
  };

  const agentCount = parsed?.agents ? Object.keys(parsed.agents).length : 0;
  const categoryCount = parsed?.categories ? Object.keys(parsed.categories).length : 0;
  const otherFieldCount = parsed?.otherFields ? Object.keys(parsed.otherFields).length : 0;
  const jsonPlaceholder = variant === 'omo'
    ? `{
  "agents": {
    "Coder": { "model": "..." },
    "Architect": { "model": "..." }
  },
  "categories": {
    "coding": { "model": "..." }
  }
}`
    : `{
  "preset": "openai",
  "presets": {
    "openai": {
      "orchestrator": { "model": "..." },
      "oracle": { "model": "..." }
    }
  }
}`;

  return (
    <Modal
      title={t('opencode.ohMyOpenCode.importFromJsonTitle')}
      open={open}
      onCancel={onCancel}
      width={700}
      footer={
        parsed ? (
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
            <Button onClick={onCancel}>
              {t('common.cancel')}
            </Button>
            <Button
              type="default"
              onClick={() => handleImport('core')}
            >
              {variant === 'omo'
                ? t('opencode.ohMyOpenCode.importFromJsonModeCore')
                : t('opencode.ohMyOpenCode.importFromJsonModeCoreSlim')}
            </Button>
            <Button
              type="primary"
              onClick={() => handleImport('full')}
            >
              {t('opencode.ohMyOpenCode.importFromJsonModeFull')}
            </Button>
          </div>
        ) : (
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
            <Button onClick={onCancel}>
              {t('common.cancel')}
            </Button>
            <Button type="primary" onClick={handleParse}>
              {t('opencode.ohMyOpenCode.importFromJsonParse')}
            </Button>
          </div>
        )
      }
    >
      <Text type="secondary" style={{ display: 'block', fontSize: 12, marginBottom: 12 }}>
        {variant === 'omo'
          ? t('opencode.ohMyOpenCode.importFromJsonHint')
          : t('opencode.ohMyOpenCode.importFromJsonHintSlim')}
      </Text>

      <JsonEditor
        onChange={(value) => {
          if (value === null || value === undefined) {
            jsonRawRef.current = '';
          } else if (typeof value === 'string') {
            jsonRawRef.current = value;
          } else {
            jsonRawRef.current = JSON.stringify(value, null, 2);
          }
          // Reset parsed state when content changes
          if (parsed) setParsed(null);
        }}
        height={350}
        minHeight={200}
        maxHeight={500}
        resizable
        mode="text"
        placeholder={jsonPlaceholder}
      />

      {parsed && (
        <div style={{ marginTop: 12, padding: '8px 12px', background: 'var(--color-bg-hover)', borderRadius: 8, fontSize: 12 }}>
          <Text type="secondary">
            {t('opencode.ohMyOpenCode.importFromJsonParsed', {
              agentCount,
              categoryCount,
              otherFieldCount,
            })}
          </Text>
        </div>
      )}
    </Modal>
  );
};

export default ImportJsonConfigModal;
