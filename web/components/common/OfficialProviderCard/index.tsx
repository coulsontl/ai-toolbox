import type React from 'react';
import { Card, Collapse, Tag, Typography, Space, Switch, Tooltip } from 'antd';
import { SafetyOutlined, LockOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { OfficialModel } from '@/services/opencodeApi';

const { Text } = Typography;

interface OfficialModelItemProps {
  model: OfficialModel;
  i18nPrefix?: string;
}

/**
 * Get status tag color based on status value
 */
const getStatusTagColor = (status: string): string => {
  switch (status) {
    case 'alpha':
      return 'purple';
    case 'beta':
      return 'blue';
    case 'deprecated':
      return 'red';
    default:
      return 'default';
  }
};

/**
 * Display a single official model (read-only)
 */
const OfficialModelItem: React.FC<OfficialModelItemProps> = ({ model, i18nPrefix = 'opencode' }) => {
  const { t } = useTranslation();

  return (
    <div
      style={{
        padding: '8px 12px',
        backgroundColor: 'var(--color-bg-container)',
        borderRadius: '6px',
        border: '1px dashed #d4b106',
        marginBottom: 4,
      }}
      title={t(`${i18nPrefix}.official.modelReadOnlyHint`)}
    >
      <Space size={8} wrap style={{ flex: 1, minWidth: 0 }}>
        <Text style={{ fontSize: 13 }}>{model.name || model.id}</Text>
        <Text type="secondary" style={{ fontSize: 11 }}>
          ID: {model.id}
        </Text>
        {model.isFree && (
          <>
            <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
            <Tag color="green" style={{ fontSize: 11, margin: 0 }}>
              {t(`${i18nPrefix}.official.freeModel`)}
            </Tag>
          </>
        )}
        {model.status && (
          <>
            <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
            <Tag color={getStatusTagColor(model.status)} style={{ fontSize: 11, margin: 0 }}>
              {model.status}
            </Tag>
          </>
        )}
        {(model.context !== undefined && model.context !== null) || (model.output !== undefined && model.output !== null) ? (
          <>
            <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
            <Text type="secondary" style={{ fontSize: 11 }}>
              {[
                model.context !== undefined && model.context !== null ? `${t(`${i18nPrefix}.official.contextLimit`)}: ${model.context.toLocaleString()}` : null,
                model.output !== undefined && model.output !== null ? `${t(`${i18nPrefix}.official.outputLimit`)}: ${model.output.toLocaleString()}` : null,
              ].filter(Boolean).join(' | ')}
            </Text>
          </>
        ) : null}
      </Space>
      <LockOutlined style={{ fontSize: 12, color: '#d4b106', marginLeft: 8 }} />
    </div>
  );
};

interface OfficialProviderCardProps {
  id: string;
  name: string;
  models: OfficialModel[];
  i18nPrefix?: string;
  /** Whether this provider is disabled (controlled by disabled_providers). */
  isDisabled?: boolean;
  /** Toggle callback for disabled state. */
  onToggleDisabled?: () => void;
}

/**
 * A read-only card component for displaying official auth providers
 * These providers cannot be edited, deleted, or copied
 */
const OfficialProviderCard: React.FC<OfficialProviderCardProps> = ({
  id,
  name,
  models,
  i18nPrefix = 'opencode',
  isDisabled,
  onToggleDisabled,
}) => {
  const { t } = useTranslation();

  const renderModelList = () => {
    if (models.length === 0) {
      return (
        <div style={{ padding: '16px', textAlign: 'center', color: '#999' }}>
          {t(`${i18nPrefix}.official.noModels`)}
        </div>
      );
    }

    return (
      <Space orientation="vertical" style={{ width: '100%' }} size={0}>
        {models.map((model) => (
          <OfficialModelItem key={model.id} model={model} i18nPrefix={i18nPrefix} />
        ))}
      </Space>
    );
  };

  return (
    <Card
      style={{
        marginBottom: 16,
        border: '1px solid #d4b106',
      }}
      styles={{
        body: { padding: 16 },
      }}
    >
      <div>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 8 }}>
          <div>
            <Space size={8}>
              <SafetyOutlined style={{ color: '#d4b106', fontSize: 14 }} />
              <Text strong style={{ fontSize: 14 }}>
                {name}
              </Text>
            </Space>
            <div style={{ marginTop: 4 }}>
              <Text type="secondary" style={{ fontSize: 12 }}>
                ID: {id}
              </Text>
            </div>
          </div>
          <Space size={8}>
            <Tag color="warning" icon={<LockOutlined />} style={{ fontSize: 11 }}>
              {t(`${i18nPrefix}.official.authProvider`)}
            </Tag>
            {onToggleDisabled && (
              <Tooltip
                title={
                  isDisabled
                    ? t(`${i18nPrefix}.provider.disabled`)
                    : t(`${i18nPrefix}.provider.enabled`)
                }
              >
                <Switch
                  size="small"
                  checked={!isDisabled}
                  onChange={() => {
                    onToggleDisabled();
                  }}
                />
              </Tooltip>
            )}
          </Space>
        </div>

        <Collapse
          defaultActiveKey={[]}
          ghost
          style={{ marginTop: 8 }}
          items={[
            {
              key: id,
              label: (
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', width: '100%' }}>
                  <Text strong style={{ fontSize: 13 }}>
                    {t(`${i18nPrefix}.model.title`)} ({models.length})
                  </Text>
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    {t(`${i18nPrefix}.official.readOnly`)}
                  </Text>
                </div>
              ),
              children: renderModelList(),
            },
          ]}
        />
      </div>
    </Card>
  );
};

export default OfficialProviderCard;
