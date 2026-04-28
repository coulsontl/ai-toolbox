import React from 'react';
import { App, Button, Dropdown, Input, InputNumber, Modal, Switch, Typography } from 'antd';
import type { MenuProps } from 'antd';
import { Plus, Trash2, ChevronDown } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { ImageChannelModel, UpsertImageChannelInput } from '../services/imageApi';
import styles from './ImageChannelModal.module.less';

const { Text } = Typography;

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

interface ImageChannelModalProps {
  open: boolean;
  draft: ChannelDraft | null;
  saving: boolean;
  onClose: () => void;
  onChange: (nextDraft: ChannelDraft) => void;
  onSubmit: () => Promise<void>;
}

const PROVIDER_KIND_OPTIONS = [
  { value: 'openai_compatible', label: 'OpenAI Compatible' },
];

const createEmptyModel = (): ImageChannelModel => ({
  id: '',
  name: '',
  supports_text_to_image: true,
  supports_image_to_image: true,
  enabled: true,
});

const buildDropdownItems = (
  options: Array<{ value: string; label: string }>
): MenuProps['items'] => options.map((option) => ({ key: option.value, label: option.label }));

const findOptionLabel = (
  options: Array<{ value: string; label: string }>,
  value: string
): string => options.find((option) => option.value === value)?.label ?? value;

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

const ImageChannelModal: React.FC<ImageChannelModalProps> = ({
  open,
  draft,
  saving,
  onClose,
  onChange,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();

  const renderProviderTrigger = (label: string) => (
    <button type="button" className={styles.providerTrigger}>
      <span className={styles.providerTriggerValue}>{label}</span>
      <ChevronDown size={14} className={styles.providerTriggerIcon} />
    </button>
  );

  const updateModel = React.useCallback(
    (modelIndex: number, updater: (currentModel: ImageChannelModel) => ImageChannelModel) => {
      if (!draft) return;
      onChange({
        ...draft,
        models: draft.models.map((model, index) => (index === modelIndex ? updater(model) : model)),
      });
    },
    [draft, onChange]
  );

  if (!draft) return null;

  return (
    <Modal
      open={open}
      onCancel={onClose}
      onOk={() => {
        void onSubmit().catch((error) => {
          message.error(toErrorMessage(error, t('common.error')));
        });
      }}
      okButtonProps={{ className: styles.primaryActionButton }}
      cancelButtonProps={{ className: styles.secondaryActionButton }}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      confirmLoading={saving}
      title={draft.id ? t('image.more.editChannel') : t('image.more.addChannel')}
      className={styles.modal}
      width={980}
      destroyOnHidden
    >
      <div className={styles.content}>
        <section className={styles.sectionCard}>
          <div className={styles.sectionTitle}>
            <Text strong>{t('image.more.connectionTitle')}</Text>
            <span className={styles.sectionHint}>{t('image.more.connectionHint')}</span>
          </div>

          <div className={styles.formGrid}>
            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.name')}</div>
              <Input
                value={draft.name}
                onChange={(event) => onChange({ ...draft, name: event.target.value })}
              />
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.provider')}</div>
              <Dropdown
                trigger={['click']}
                overlayClassName={styles.providerDropdownOverlay}
                menu={{
                  items: buildDropdownItems(PROVIDER_KIND_OPTIONS),
                  selectable: true,
                  selectedKeys: [draft.provider_kind],
                  onClick: ({ key }) =>
                    onChange({
                      ...draft,
                      provider_kind: key as UpsertImageChannelInput['provider_kind'],
                    }),
                }}
              >
                {renderProviderTrigger(
                  findOptionLabel(PROVIDER_KIND_OPTIONS, draft.provider_kind)
                )}
              </Dropdown>
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.enabled')}</div>
              <div className={styles.switchControl}>
                <Switch
                  size="small"
                  checked={draft.enabled}
                  onChange={(checked) => onChange({ ...draft, enabled: checked })}
                />
              </div>
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.baseUrl')}</div>
              <Input
                value={draft.base_url}
                onChange={(event) => onChange({ ...draft, base_url: event.target.value })}
              />
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.apiKey')}</div>
              <Input.Password
                value={draft.api_key}
                onChange={(event) => onChange({ ...draft, api_key: event.target.value })}
              />
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.timeoutSeconds')}</div>
              <InputNumber
                min={1}
                max={1800}
                controls={false}
                value={draft.timeout_seconds ?? 300}
                onChange={(value) =>
                  onChange({
                    ...draft,
                    timeout_seconds: typeof value === 'number' ? value : 300,
                  })
                }
              />
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.generationPath')}</div>
              <Input
                value={draft.generation_path ?? ''}
                placeholder={t('image.more.placeholders.generationPath')}
                onChange={(event) => onChange({ ...draft, generation_path: event.target.value })}
              />
            </div>

            <div className={styles.fieldRow}>
              <div className={styles.fieldLabel}>{t('image.more.fields.editPath')}</div>
              <Input
                value={draft.edit_path ?? ''}
                placeholder={t('image.more.placeholders.editPath')}
                onChange={(event) => onChange({ ...draft, edit_path: event.target.value })}
              />
            </div>
          </div>
        </section>

        <section className={styles.sectionCard}>
          <div className={styles.sectionHeader}>
            <div className={styles.sectionTitle}>
              <Text strong>{t('image.more.fields.models')}</Text>
              <span className={styles.sectionHint}>{t('image.more.modelsHint')}</span>
            </div>
            <Button
              size="small"
              className={styles.secondaryActionButtonCompact}
              icon={<Plus size={12} />}
              onClick={() => onChange({ ...draft, models: [...draft.models, createEmptyModel()] })}
            >
              {t('image.more.actions.addModel')}
            </Button>
          </div>

          <div className={styles.modelList}>
            {draft.models.map((model, index) => (
              <div key={`${draft.id ?? 'draft'}-model-${index}`} className={styles.modelCard}>
                <div className={styles.modelGrid}>
                  <div className={styles.modelField}>
                    <span className={styles.modelFieldLabel}>{t('image.more.fields.modelId')}</span>
                    <Input
                      value={model.id}
                      onChange={(event) =>
                        updateModel(index, (currentModel) => ({
                          ...currentModel,
                          id: event.target.value,
                        }))
                      }
                    />
                  </div>

                  <div className={styles.modelField}>
                    <span className={styles.modelFieldLabel}>{t('image.more.fields.modelName')}</span>
                    <Input
                      value={model.name ?? ''}
                      onChange={(event) =>
                        updateModel(index, (currentModel) => ({
                          ...currentModel,
                          name: event.target.value,
                        }))
                      }
                    />
                  </div>

                  <div className={styles.modelSwitchField}>
                    <span className={styles.modelFieldLabel}>{t('image.more.fields.supportsText')}</span>
                    <div className={styles.switchControl}>
                      <Switch
                        size="small"
                        checked={model.supports_text_to_image}
                        onChange={(checked) =>
                          updateModel(index, (currentModel) => ({
                            ...currentModel,
                            supports_text_to_image: checked,
                          }))
                        }
                      />
                    </div>
                  </div>

                  <div className={styles.modelSwitchField}>
                    <span className={styles.modelFieldLabel}>{t('image.more.fields.supportsImage')}</span>
                    <div className={styles.switchControl}>
                      <Switch
                        size="small"
                        checked={model.supports_image_to_image}
                        onChange={(checked) =>
                          updateModel(index, (currentModel) => ({
                            ...currentModel,
                            supports_image_to_image: checked,
                          }))
                        }
                      />
                    </div>
                  </div>

                  <div className={styles.modelSwitchField}>
                    <span className={styles.modelFieldLabel}>{t('image.more.fields.modelEnabled')}</span>
                    <div className={styles.switchControl}>
                      <Switch
                        size="small"
                        checked={model.enabled}
                        onChange={(checked) =>
                          updateModel(index, (currentModel) => ({
                            ...currentModel,
                            enabled: checked,
                          }))
                        }
                      />
                    </div>
                  </div>
                </div>

                <div className={styles.modelActions}>
                  <Button
                    size="small"
                    className={styles.dangerActionButtonCompact}
                    danger
                    icon={<Trash2 size={12} />}
                    onClick={() =>
                      onChange({
                        ...draft,
                        models: draft.models.filter((_, modelIndex) => modelIndex !== index),
                      })
                    }
                  >
                    {t('common.delete')}
                  </Button>
                </div>
              </div>
            ))}

            {draft.models.length === 0 && (
              <div className={styles.emptyHint}>{t('image.more.emptyModels')}</div>
            )}
          </div>
        </section>
      </div>
    </Modal>
  );
};

export default ImageChannelModal;
