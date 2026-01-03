import React from 'react';
import { Typography, Card } from 'antd';
import { FileTextOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';

const { Title, Text } = Typography;

const NotesPage: React.FC = () => {
  const { t } = useTranslation();

  return (
    <Card>
      <div style={{ textAlign: 'center', padding: '60px 0' }}>
        <FileTextOutlined style={{ fontSize: 64, color: '#1890ff', marginBottom: 24 }} />
        <Title level={3}>{t('notes.title')}</Title>
        <Text type="secondary">{t('placeholder.comingSoon')}</Text>
      </div>
    </Card>
  );
};

export default NotesPage;
