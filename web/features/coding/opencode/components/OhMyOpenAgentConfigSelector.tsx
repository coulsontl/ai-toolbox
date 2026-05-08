import React from 'react';
import { Select, Spin, Empty, Button, Space, message } from 'antd';
import { SyncOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { OhMyOpenAgentConfig } from '@/types/ohMyOpenAgent';
import { listOhMyOpenAgentConfigs, applyOhMyOpenAgentConfig } from '@/services/ohMyOpenAgentApi';
import { useRefreshStore } from '@/stores';

interface OhMyOpenAgentConfigSelectorProps {
  disabled?: boolean;
  onConfigSelected?: (configId: string) => void;
}

const OhMyOpenAgentConfigSelector: React.FC<OhMyOpenAgentConfigSelectorProps> = ({
  disabled = false,
  onConfigSelected,
}) => {
  const { t } = useTranslation();
  const { omoConfigRefreshKey, incrementOmoConfigRefresh } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configs, setConfigs] = React.useState<OhMyOpenAgentConfig[]>([]);
  const [selectedConfigId, setSelectedConfigId] = React.useState<string>('');

  // Load configs on mount and when refresh key changes
  React.useEffect(() => {
    loadConfigs();
  }, [omoConfigRefreshKey]);

  const loadConfigs = async () => {
    setLoading(true);
    try {
      const data = await listOhMyOpenAgentConfigs();
      setConfigs(data);
      const applied = data.find((c) => c.isApplied);
      if (applied) {
        setSelectedConfigId(applied.id);
      } else {
        setSelectedConfigId('');
      }
    } catch (error) {
      console.error('Failed to load configs:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleChange = async (configId: string | undefined) => {
    if (!configId) {
      setSelectedConfigId('');
      return;
    }

    try {
      await applyOhMyOpenAgentConfig(configId);
      setSelectedConfigId(configId);
      message.success(t('opencode.ohMyOpenCode.applySuccess'));
      loadConfigs();
      // 触发其他组件（如 Settings）刷新
      incrementOmoConfigRefresh();
      if (onConfigSelected) {
        onConfigSelected(configId);
      }
    } catch {
      message.error(t('common.error'));
    }
  };

  const options = configs.map((config) => ({
    label: config.isApplied 
      ? `${config.name} ✓` 
      : config.name,
    value: config.id,
  }));

  if (loading) {
    return <Spin size="small" />;
  }

  if (configs.length === 0) {
    return (
      <Empty 
        description={t('opencode.ohMyOpenCode.noConfigs')} 
        style={{ margin: '8px 0' }}
      >
        <Button
          type="link"
          size="small"
          icon={<SyncOutlined />}
          onClick={loadConfigs}
        >
          {t('opencode.ohMyOpenCode.refresh')}
        </Button>
      </Empty>
    );
  }

  return (
    <Space.Compact style={{ width: '100%' }}>
      <Select
        value={selectedConfigId || undefined}
        onChange={handleChange}
        placeholder={t('opencode.ohMyOpenCode.selectConfig')}
        options={options}
        style={{ flex: 1 }}
        allowClear
        disabled={disabled}
      />
      <Button
        icon={<SyncOutlined />}
        onClick={loadConfigs}
        loading={loading}
        disabled={disabled}
      />
    </Space.Compact>
  );
};

export default OhMyOpenAgentConfigSelector;
