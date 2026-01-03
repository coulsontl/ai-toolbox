import React from 'react';
import { Modal, Form, Input, Switch, Row, Col } from 'antd';
import { useTranslation } from 'react-i18next';
import { useSettingsStore, type S3ConfigFE } from '@/stores';

interface S3SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const S3SettingsModal: React.FC<S3SettingsModalProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const { s3, setS3 } = useSettingsStore();

  React.useEffect(() => {
    if (open) {
      form.setFieldsValue(s3);
    }
  }, [open, s3, form]);

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      setS3(values as Partial<S3ConfigFE>);
      onClose();
    } catch {
      // Validation failed
    }
  };

  return (
    <Modal
      title={t('settings.s3.title')}
      open={open}
      onOk={handleSave}
      onCancel={onClose}
      width={520}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form form={form} layout="vertical" size="small">
        <Row gutter={16}>
          <Col span={12}>
            <Form.Item label={t('settings.s3.accessKey')} name="accessKey">
              <Input.Password visibilityToggle />
            </Form.Item>
          </Col>
          <Col span={12}>
            <Form.Item label={t('settings.s3.secretKey')} name="secretKey">
              <Input.Password visibilityToggle />
            </Form.Item>
          </Col>
        </Row>
        <Row gutter={16}>
          <Col span={12}>
            <Form.Item label={t('settings.s3.bucket')} name="bucket">
              <Input />
            </Form.Item>
          </Col>
          <Col span={12}>
            <Form.Item label={t('settings.s3.region')} name="region">
              <Input placeholder="us-east-1" />
            </Form.Item>
          </Col>
        </Row>
        <Form.Item label={t('settings.s3.prefix')} name="prefix">
          <Input placeholder="upload/images" />
        </Form.Item>
        <Form.Item label={t('settings.s3.endpointUrl')} name="endpointUrl">
          <Input placeholder="https://s3.example.com" />
        </Form.Item>
        <Row gutter={16}>
          <Col span={12}>
            <Form.Item
              label={t('settings.s3.forcePathStyle')}
              name="forcePathStyle"
              valuePropName="checked"
            >
              <Switch />
            </Form.Item>
          </Col>
          <Col span={12}>
            <Form.Item label={t('settings.s3.publicDomain')} name="publicDomain">
              <Input placeholder="https://cdn.example.com" />
            </Form.Item>
          </Col>
        </Row>
      </Form>
    </Modal>
  );
};

export default S3SettingsModal;
