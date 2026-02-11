/**
 * SSH Connection Modal
 *
 * Modal for creating/editing SSH connection presets
 */

import React, { useEffect } from 'react';
import { Modal, Form, Input, InputNumber, Radio, Space } from 'antd';
import { useTranslation } from 'react-i18next';
import type { SSHConnection } from '@/types/sshsync';

interface SSHConnectionModalProps {
  open: boolean;
  onClose: () => void;
  onSave: (connection: SSHConnection) => void;
  connection: SSHConnection | null;
}

export const SSHConnectionModal: React.FC<SSHConnectionModalProps> = ({
  open,
  onClose,
  onSave,
  connection,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();

  const isEditing = connection && connection.id !== '';

  useEffect(() => {
    if (open && connection) {
      form.setFieldsValue({
        name: connection.name,
        host: connection.host,
        port: connection.port || 22,
        username: connection.username,
        authMethod: connection.authMethod || 'key',
        password: connection.password,
        privateKeyPath: connection.privateKeyPath,
        passphrase: connection.passphrase,
      });
    } else if (open) {
      form.resetFields();
      form.setFieldsValue({
        port: 22,
        authMethod: 'key',
      });
    }
  }, [open, connection, form]);

  const handleOk = async () => {
    try {
      const values = await form.validateFields();
      const id = connection?.id || `ssh-${Date.now()}`;
      onSave({
        id,
        name: values.name,
        host: values.host,
        port: values.port || 22,
        username: values.username,
        authMethod: values.authMethod,
        password: values.authMethod === 'password' ? values.password || '' : '',
        privateKeyPath: values.authMethod === 'key' ? values.privateKeyPath || '' : '',
        privateKeyContent: connection?.privateKeyContent || '',
        passphrase: values.authMethod === 'key' ? values.passphrase || '' : '',
        sortOrder: connection?.sortOrder || 0,
      });
      onClose();
    } catch {
      // validation failed
    }
  };

  const authMethod = Form.useWatch('authMethod', form);

  return (
    <Modal
      title={isEditing ? t('settings.ssh.editConnection') : t('settings.ssh.newConnection')}
      open={open}
      onOk={handleOk}
      onCancel={onClose}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      destroyOnClose
    >
      <Form form={form} layout="horizontal" labelCol={{ span: 6 }} wrapperCol={{ span: 18 }}>
        <Form.Item
          name="name"
          label={t('settings.ssh.connectionName')}
          rules={[{ required: true, message: t('settings.ssh.connectionNameRequired') }]}
        >
          <Input placeholder={t('settings.ssh.connectionNamePlaceholder')} />
        </Form.Item>

        <Form.Item
          name="host"
          label={t('settings.ssh.host')}
          rules={[{ required: true, message: t('settings.ssh.hostRequired') }]}
        >
          <Input placeholder="192.168.1.100" />
        </Form.Item>

        <Form.Item name="port" label={t('settings.ssh.port')}>
          <InputNumber min={1} max={65535} style={{ width: '100%' }} />
        </Form.Item>

        <Form.Item
          name="username"
          label={t('settings.ssh.username')}
          rules={[{ required: true, message: t('settings.ssh.usernameRequired') }]}
        >
          <Input placeholder="root" />
        </Form.Item>

        <Form.Item name="authMethod" label={t('settings.ssh.authMethod')}>
          <Radio.Group>
            <Space direction="horizontal">
              <Radio value="key">{t('settings.ssh.authKey')}</Radio>
              <Radio value="password">{t('settings.ssh.authPassword')}</Radio>
            </Space>
          </Radio.Group>
        </Form.Item>

        {authMethod === 'key' && (
          <>
            <Form.Item name="privateKeyPath" label={t('settings.ssh.privateKey')}>
              <Input.TextArea
                placeholder={t('settings.ssh.privateKeyPlaceholder')}
                autoSize={{ minRows: 1, maxRows: 6 }}
              />
            </Form.Item>
            <Form.Item name="passphrase" label={t('settings.ssh.passphrase')}>
              <Input.Password placeholder={t('settings.ssh.passphrasePlaceholder')} />
            </Form.Item>
          </>
        )}

        {authMethod === 'password' && (
          <Form.Item
            name="password"
            label={t('settings.ssh.password')}
            rules={[{ required: true, message: t('settings.ssh.passwordRequired') }]}
          >
            <Input.Password />
          </Form.Item>
        )}
      </Form>
    </Modal>
  );
};
