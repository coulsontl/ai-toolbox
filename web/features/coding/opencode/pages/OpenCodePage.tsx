import React from 'react';
import { Typography, Card } from 'antd';
import { CodeOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';

const { Title, Text } = Typography;

const OpenCodePage: React.FC = () => {
  const { t } = useTranslation();

  return (
    <Card>
      <div style={{ textAlign: 'center', padding: '60px 0' }}>
        <CodeOutlined style={{ fontSize: 64, color: '#52c41a', marginBottom: 24 }} />
        <Title level={3}>OpenCode</Title>
        <Text type="secondary">{t('placeholder.opencode')}</Text>
      </div>
    </Card>
  );
};

export default OpenCodePage;
