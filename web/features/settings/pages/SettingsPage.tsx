import React from 'react';
import { Tabs, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import GeneralSettingsPage from './GeneralSettingsPage';
import ProviderSettingsPage from './ProviderSettingsPage';

const { Title } = Typography;

const SettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = React.useState('general');

  const tabItems = [
    {
      key: 'general',
      label: t('settings.tabs.general'),
      children: <GeneralSettingsPage />,
    },
    {
      key: 'provider',
      label: t('settings.tabs.provider'),
      children: <ProviderSettingsPage />,
    },
  ];

  return (
    <div>
      <Title level={4} style={{ marginBottom: 16 }}>
        {t('settings.title')}
      </Title>
      <Tabs activeKey={activeTab} onChange={setActiveTab} items={tabItems} />
    </div>
  );
};

export default SettingsPage;
