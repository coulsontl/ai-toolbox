import React from 'react';
import {
  Button,
  Collapse,
  Divider,
  Dropdown,
  Input,
  Modal,
  Select,
  Space,
  Tooltip,
} from 'antd';
import {
  DeleteOutlined,
  EllipsisOutlined,
  FileTextOutlined,
  PlusOutlined,
  QuestionCircleOutlined,
  SettingOutlined,
  UndoOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';

import type {
  OpenCodeAgentConfig,
  OpenCodeConfig,
} from '@/types/opencode';

import {
  clearInvalidOpenCodeDefaultAgent,
  getOpenCodeAgentConfigs,
  getOpenCodeCustomAgentNames,
  getOpenCodeDefaultAgentCandidates,
  OPEN_CODE_BUILT_IN_PRIMARY_AGENTS,
  OPEN_CODE_BUILT_IN_SUBAGENTS,
  OPEN_CODE_INTERNAL_AGENTS,
  isOpenCodeBuiltInAgentName,
  removeOpenCodeAgentOverride,
  setOpenCodeAgentAdvancedConfig,
  setOpenCodeAgentModel,
  setOpenCodeAgentPrompt,
  setOpenCodeAgentVariant,
} from '../utils/openCodeAgentConfig';
import OpenCodeAgentAdvancedModal from './OpenCodeAgentAdvancedModal';
import OpenCodeAgentPromptModal from './OpenCodeAgentPromptModal';
import styles from './OpenCodeAgentSettings.module.less';

interface ModelOption {
  label: string;
  value: string;
  disabled?: boolean;
}

interface ModelGroup {
  label: string;
  options: ModelOption[];
}

interface OpenCodeAgentSettingsProps {
  config: OpenCodeConfig;
  modelOptions: ModelGroup[];
  modelVariantsMap: Record<string, string[]>;
  onSave: (config: OpenCodeConfig) => Promise<void>;
}

interface NewAgentDraft {
  name: string;
  mode: NonNullable<OpenCodeAgentConfig['mode']>;
  description: string;
  model?: string;
  variant?: string;
  prompt?: string;
}

interface PromptEditorTarget {
  type: 'new' | 'existing';
  agentName: string;
}

const defaultDescriptions: Record<string, string> = {
  build: 'opencode.agentSettings.descriptions.build',
  plan: 'opencode.agentSettings.descriptions.plan',
  general: 'opencode.agentSettings.descriptions.general',
  explore: 'opencode.agentSettings.descriptions.explore',
  scout: 'opencode.agentSettings.descriptions.scout',
  title: 'opencode.agentSettings.descriptions.title',
  summary: 'opencode.agentSettings.descriptions.summary',
  compaction: 'opencode.agentSettings.descriptions.compaction',
};

const OpenCodeAgentSettings: React.FC<OpenCodeAgentSettingsProps> = ({
  config,
  modelOptions,
  modelVariantsMap,
  onSave,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = React.useState(false);
  const [advancedAgentName, setAdvancedAgentName] = React.useState<string>();
  const [promptEditorTarget, setPromptEditorTarget] = React.useState<PromptEditorTarget>();
  const [addModalOpen, setAddModalOpen] = React.useState(false);
  const [newAgent, setNewAgent] = React.useState<NewAgentDraft>({
    name: '',
    mode: 'all',
    description: '',
  });

  const agentConfigs = getOpenCodeAgentConfigs(config);
  const customAgentNames = getOpenCodeCustomAgentNames(config);
  const defaultAgentCandidates = getOpenCodeDefaultAgentCandidates(config);
  const defaultAgentOptions = React.useMemo(() => {
    const candidates = defaultAgentCandidates.map((agentName) => ({
      label: agentName,
      value: agentName,
    }));
    if (!config.default_agent || defaultAgentCandidates.includes(config.default_agent)) {
      return candidates;
    }

    return [
      {
        label: t('opencode.agentSettings.invalidDefaultAgent', { name: config.default_agent }),
        value: config.default_agent,
      },
      ...candidates,
    ];
  }, [config.default_agent, defaultAgentCandidates, t]);

  const getVariants = (model: string | undefined, currentVariant?: string) => {
    const variants = model ? [...(modelVariantsMap[model] ?? [])] : [];
    if (currentVariant && !variants.includes(currentVariant)) {
      variants.unshift(currentVariant);
    }
    return variants;
  };

  const getModelOptions = (currentModel?: string): ModelGroup[] => {
    if (!currentModel) return modelOptions;
    const exists = modelOptions.some((group) => group.options.some((option) => option.value === currentModel));
    if (exists) return modelOptions;

    return [
      {
        label: t('opencode.agentSettings.currentConfigGroup'),
        options: [{
          label: t('opencode.agentSettings.unavailableModel', { model: currentModel }),
          value: currentModel,
        }],
      },
      ...modelOptions,
    ];
  };

  const handleModelChange = async (agentName: string, model: string | undefined) => {
    const variants = model ? modelVariantsMap[model] ?? [] : [];
    try {
      await onSave(setOpenCodeAgentModel(config, agentName, model, variants));
    } catch {
      // Parent save handler already reports the error.
    }
  };

  const handleVariantChange = async (agentName: string, variant: string | undefined) => {
    try {
      await onSave(setOpenCodeAgentVariant(config, agentName, variant));
    } catch {
      // Parent save handler already reports the error.
    }
  };

  const handleDefaultAgentChange = async (defaultAgent: string | undefined) => {
    try {
      await onSave({
        ...config,
        default_agent: defaultAgent || undefined,
      });
    } catch {
      // Parent save handler already reports the error.
    }
  };

  const handleReset = (agentName: string, custom: boolean) => {
    Modal.confirm({
      title: custom
        ? t('opencode.agentSettings.deleteConfirmTitle', { name: agentName })
        : t('opencode.agentSettings.resetConfirmTitle', { name: agentName }),
      content: custom
        ? t('opencode.agentSettings.deleteConfirmContent')
        : t('opencode.agentSettings.resetConfirmContent'),
      okText: custom ? t('common.delete') : t('common.reset'),
      okButtonProps: custom ? { danger: true } : undefined,
      cancelText: t('common.cancel'),
      onOk: async () => {
        const nextConfig = removeOpenCodeAgentOverride(config, agentName);
        if (config.default_agent === agentName) {
          nextConfig.default_agent = undefined;
        }
        await onSave(nextConfig);
      },
    });
  };

  const resetNewAgentDraft = () => {
    setNewAgent({ name: '', mode: 'all', description: '' });
  };

  const handleOpenAddModal = () => {
    resetNewAgentDraft();
    setAddModalOpen(true);
  };

  const handleCloseAddModal = () => {
    setPromptEditorTarget(undefined);
    setAddModalOpen(false);
    resetNewAgentDraft();
  };

  const renderAgentRow = (agentName: string, custom = false) => {
    const agentConfig = agentConfigs[agentName] ?? {};
    const variants = getVariants(agentConfig.model, agentConfig.variant);
    const showVariant = Boolean(agentConfig.model && variants.length > 0);
    const description = agentConfig.description
      ?? (defaultDescriptions[agentName] ? t(defaultDescriptions[agentName]) : t('opencode.agentSettings.customDescription'));

    const menuItems = [
      {
        key: 'prompt',
        icon: <FileTextOutlined />,
        label: t('opencode.agentSettings.editPrompt'),
        onClick: () => setPromptEditorTarget({ type: 'existing', agentName }),
      },
      {
        key: 'advanced',
        icon: <SettingOutlined />,
        label: t('opencode.agentSettings.advanced'),
        onClick: () => setAdvancedAgentName(agentName),
      },
      {
        key: 'reset',
        icon: custom ? <DeleteOutlined /> : <UndoOutlined />,
        danger: custom,
        label: custom ? t('opencode.agentSettings.deleteAgent') : t('opencode.agentSettings.restoreDefaults'),
        onClick: () => handleReset(agentName, custom),
      },
    ];

    return (
      <div className={styles.agentRow} key={agentName}>
        <div className={styles.agentIdentity}>
          <Tooltip title={agentName}>
            <span className={styles.agentName}>{agentName}</span>
          </Tooltip>
          <Tooltip title={description}>
            <QuestionCircleOutlined
              className={styles.agentHelpIcon}
              aria-label={description}
              tabIndex={0}
            />
          </Tooltip>
          {agentConfig.disable ? (
            <Tooltip title={t('opencode.agentSettings.disabled')}>
              <span className={styles.disabledIndicator} aria-label={t('opencode.agentSettings.disabled')} />
            </Tooltip>
          ) : null}
        </div>
        <Space.Compact className={styles.modelControls}>
          <Select
            className={styles.modelSelect}
            value={agentConfig.model}
            options={getModelOptions(agentConfig.model)}
            placeholder={t('opencode.agentSettings.inheritModel')}
            allowClear
            showSearch
            optionFilterProp="label"
            aria-label={`${agentName} ${t('opencode.agentSettings.model')}`}
            onChange={(model) => void handleModelChange(agentName, model)}
            notFoundContent={t('opencode.modelSettings.noModels')}
          />
          {showVariant ? (
            <Select
              className={styles.variantSelect}
              value={agentConfig.variant}
              options={variants.map((variant) => ({ label: variant, value: variant }))}
              placeholder={t('opencode.agentSettings.variantPlaceholder')}
              allowClear
              aria-label={`${agentName} ${t('opencode.agentSettings.variant')}`}
              onChange={(variant) => void handleVariantChange(agentName, variant)}
            />
          ) : null}
          <Dropdown menu={{ items: menuItems }} trigger={['click']}>
            <Button
              className={styles.advancedButton}
              icon={<EllipsisOutlined />}
              aria-label={t('opencode.agentSettings.moreActions', { name: agentName })}
            />
          </Dropdown>
        </Space.Compact>
      </div>
    );
  };

  const handleAddAgent = async () => {
    const agentName = newAgent.name.trim();
    const description = newAgent.description.trim();
    if (
      !agentName
      || !description
      || agentConfigs[agentName]
      || isOpenCodeBuiltInAgentName(agentName)
    ) return;

    const agentConfig: OpenCodeAgentConfig = {
      mode: newAgent.mode,
      description,
      ...(newAgent.model ? { model: newAgent.model } : {}),
      ...(newAgent.model && newAgent.variant ? { variant: newAgent.variant } : {}),
      ...(newAgent.prompt?.trim() ? { prompt: newAgent.prompt } : {}),
    };

    try {
      await onSave(setOpenCodeAgentAdvancedConfig(config, agentName, agentConfig));
      handleCloseAddModal();
    } catch {
      // Parent save handler already reports the error.
    }
  };

  const currentNewAgentVariants = getVariants(newAgent.model, newAgent.variant);
  const showNewAgentVariant = Boolean(newAgent.model && currentNewAgentVariants.length > 0);
  const normalizedNewAgentName = newAgent.name.trim();
  const reservedAgentName = normalizedNewAgentName !== ''
    && isOpenCodeBuiltInAgentName(normalizedNewAgentName);
  const duplicateAgentName = normalizedNewAgentName !== ''
    && Boolean(agentConfigs[normalizedNewAgentName]);
  const cannotAddAgent = !newAgent.name.trim()
    || duplicateAgentName
    || reservedAgentName
    || !newAgent.description.trim();
  const promptEditorInitialValue = promptEditorTarget?.type === 'new'
    ? newAgent.prompt
    : promptEditorTarget
      ? agentConfigs[promptEditorTarget.agentName]?.prompt
      : undefined;

  return (
    <>
      <Collapse
        className={styles.moduleCollapse}
        bordered={false}
        expandIconPosition="end"
        activeKey={expanded ? ['agent-settings'] : []}
        onChange={(keys) => setExpanded(keys.includes('agent-settings'))}
        items={[{
          key: 'agent-settings',
          label: (
            <div className={styles.moduleHeader}>
              <div className={styles.moduleHeading}>
                <span className={styles.moduleTitle}>{t('opencode.agentSettings.title')}</span>
                <span className={styles.moduleHint}>{t('opencode.agentSettings.hint')}</span>
              </div>
            </div>
          ),
          children: (
            <div className={styles.content}>
              <div className={`${styles.agentRow} ${styles.defaultAgentRow}`}>
                <span className={styles.defaultAgentLabel}>{t('opencode.agentSettings.defaultAgent')}</span>
                <Select
                  className={styles.defaultAgentSelect}
                  value={config.default_agent}
                  placeholder={t('opencode.agentSettings.defaultAgentPlaceholder')}
                  aria-label={t('opencode.agentSettings.defaultAgent')}
                  allowClear
                  options={defaultAgentOptions}
                  onChange={(defaultAgent) => void handleDefaultAgentChange(defaultAgent)}
                />
              </div>

              <section className={styles.group}>
                <Divider className={styles.groupDivider}>{t('opencode.agentSettings.primaryAgents')}</Divider>
                <div className={styles.agentList}>
                  {OPEN_CODE_BUILT_IN_PRIMARY_AGENTS.map((agentName) => renderAgentRow(agentName))}
                </div>
              </section>

              <section className={styles.group}>
                <Divider className={styles.groupDivider}>{t('opencode.agentSettings.subagents')}</Divider>
                <div className={styles.agentList}>
                  {OPEN_CODE_BUILT_IN_SUBAGENTS.map((agentName) => renderAgentRow(agentName))}
                </div>
              </section>

              <section className={styles.group}>
                <Divider className={styles.groupDivider}>{t('opencode.agentSettings.internalAgents')}</Divider>
                <div className={styles.agentList}>
                  {OPEN_CODE_INTERNAL_AGENTS.map((agentName) => renderAgentRow(agentName))}
                </div>
              </section>

              <section className={styles.group}>
                <Divider className={styles.groupDivider}>
                  <span>{t('opencode.agentSettings.customAgents')}</span>
                  <Button
                    className={styles.groupHeaderButton}
                    type="link"
                    size="small"
                    icon={<PlusOutlined />}
                    onClick={handleOpenAddModal}
                  >
                    {t('common.add')}
                  </Button>
                </Divider>
                <div className={styles.agentList}>
                  {customAgentNames.length > 0
                    ? customAgentNames.map((agentName) => renderAgentRow(agentName, true))
                    : <div className={styles.emptyGroup}>{t('opencode.agentSettings.noCustomAgents')}</div>}
                </div>
              </section>
            </div>
          ),
        }]}
      />

      <OpenCodeAgentAdvancedModal
        open={Boolean(advancedAgentName)}
        agentName={advancedAgentName ?? ''}
        initialConfig={advancedAgentName ? agentConfigs[advancedAgentName] : undefined}
        requireDescription={Boolean(advancedAgentName && !isOpenCodeBuiltInAgentName(advancedAgentName))}
        onCancel={() => setAdvancedAgentName(undefined)}
        onSave={async (agentConfig) => {
          if (!advancedAgentName) return;
          const nextConfig = setOpenCodeAgentAdvancedConfig(config, advancedAgentName, agentConfig);
          await onSave(clearInvalidOpenCodeDefaultAgent(nextConfig));
        }}
      />

      <Modal
        title={t('opencode.agentSettings.addAgentTitle')}
        width={640}
        open={addModalOpen}
        onCancel={handleCloseAddModal}
        onOk={() => void handleAddAgent()}
        okText={t('common.add')}
        cancelText={t('common.cancel')}
        okButtonProps={{ disabled: cannotAddAgent }}
      >
        <div className={styles.modalContent}>
          <div className={styles.formRow}>
            <span className={`${styles.fieldLabel} ${styles.requiredLabel}`}>
              {t('opencode.agentSettings.agentName')}
            </span>
            <Input
              value={newAgent.name}
              status={duplicateAgentName || reservedAgentName ? 'error' : undefined}
              placeholder="spec-verify"
              onChange={(event) => setNewAgent((current) => ({ ...current, name: event.target.value }))}
            />
          </div>
          {duplicateAgentName ? (
            <div className={styles.modalHint}>{t('opencode.agentSettings.duplicateName')}</div>
          ) : null}
          {reservedAgentName ? (
            <div className={styles.modalHint}>{t('opencode.agentSettings.reservedName')}</div>
          ) : null}
          <div className={styles.formRow}>
            <span className={styles.fieldLabel}>{t('opencode.agentSettings.agentMode')}</span>
            <Select
              value={newAgent.mode}
              options={(['primary', 'subagent', 'all'] as const).map((mode) => ({
                label: t(`opencode.agentSettings.mode.${mode}`),
                value: mode,
              }))}
              onChange={(mode) => setNewAgent((current) => ({ ...current, mode }))}
            />
          </div>
          <div className={styles.formRow}>
            <span className={`${styles.fieldLabel} ${styles.requiredLabel}`}>
              {t('opencode.agentSettings.description')}
            </span>
            <Input
              value={newAgent.description}
              placeholder={t('opencode.agentSettings.descriptionPlaceholder')}
              onChange={(event) => setNewAgent((current) => ({ ...current, description: event.target.value }))}
            />
          </div>
          <div className={styles.formRow}>
            <span className={styles.fieldLabel}>{t('opencode.agentSettings.model')}</span>
            <Select
              value={newAgent.model}
              options={modelOptions}
              placeholder={t('opencode.agentSettings.inheritModel')}
              allowClear
              showSearch
              optionFilterProp="label"
              onChange={(model) => setNewAgent((current) => ({
                ...current,
                model,
                variant: undefined,
              }))}
            />
          </div>
          {showNewAgentVariant ? (
            <div className={styles.formRow}>
              <span className={styles.fieldLabel}>{t('opencode.agentSettings.variant')}</span>
              <Select
                value={newAgent.variant}
                options={currentNewAgentVariants.map((variant) => ({ label: variant, value: variant }))}
                placeholder={t('opencode.agentSettings.variantPlaceholder')}
                allowClear
                onChange={(variant) => setNewAgent((current) => ({ ...current, variant }))}
              />
            </div>
          ) : null}
          <div className={styles.formRow}>
            <span className={styles.fieldLabel}>{t('opencode.agentSettings.agentPrompt')}</span>
            <button
              className={styles.promptEntry}
              type="button"
              onClick={() => setPromptEditorTarget({
                type: 'new',
                agentName: newAgent.name.trim() || t('opencode.agentSettings.newAgent'),
              })}
            >
              <span
                className={newAgent.prompt?.trim() ? styles.promptSummary : styles.promptPlaceholder}
              >
                {newAgent.prompt?.trim() || t('opencode.agentSettings.promptNotSet')}
              </span>
              <span className={styles.promptAction}>
                {newAgent.prompt?.trim() ? t('common.edit') : t('opencode.agentSettings.addPrompt')}
              </span>
            </button>
          </div>
          <div className={styles.modalHint}>{t('opencode.agentSettings.addAgentHint')}</div>
        </div>
      </Modal>

      <OpenCodeAgentPromptModal
        open={Boolean(promptEditorTarget)}
        agentName={promptEditorTarget?.agentName || newAgent.name.trim() || t('opencode.agentSettings.newAgent')}
        initialValue={promptEditorInitialValue}
        onCancel={() => setPromptEditorTarget(undefined)}
        onSave={async (prompt) => {
          if (!promptEditorTarget) return;
          if (promptEditorTarget.type === 'new') {
            setNewAgent((current) => ({ ...current, prompt }));
            setPromptEditorTarget(undefined);
            return;
          }

          await onSave(setOpenCodeAgentPrompt(
            config,
            promptEditorTarget.agentName,
            prompt,
          ));
          setPromptEditorTarget(undefined);
        }}
      />
    </>
  );
};

export default OpenCodeAgentSettings;
