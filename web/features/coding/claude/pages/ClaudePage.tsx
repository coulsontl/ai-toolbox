import React from 'react';
import { Typography, Card } from 'antd';
import { RobotOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';

const { Title, Text } = Typography;

const ClaudePage: React.FC = () => {
  const { t } = useTranslation();

  return (
    <Card>
      <div style={{ textAlign: 'center', padding: '60px 0' }}>
        <RobotOutlined style={{ fontSize: 64, color: '#722ed1', marginBottom: 24 }} />
        <Title level={3}>Claude</Title>
        <Text type="secondary">{t('placeholder.claude')}</Text>
      </div>
    </Card>
  );
};

export default ClaudePage;
