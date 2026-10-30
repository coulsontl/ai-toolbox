/**
 * SSH File Mapping Edit Modal
 *
 * Modal for adding/editing SSH file mappings
 */

import React, { useEffect } from 'react';
import { Modal, Form, Input, Select, Switch, Space, Typography, Divider } from 'antd';
import { useTranslation } from 'react-i18next';
import { sshAddFileMapping, sshUpdateFileMapping } from '@/services/sshSyncApi';
import {
  invalidCleanupPaths,
  normalizeCleanupPaths,
  supportsCleanupPaths,
} from '@/features/settings/utils/fileMappingCleanup';
import { DEFAULT_SSH_DIRECTORY_EXCLUDES } from '@/types/sshsync';
import type { SSHFileMapping } from '@/types/sshsync';

const { Text } = Typography;

const normalizeDirectoryExcludes = (values: unknown): string[] => {
  const items = Array.isArray(values) ? values : [];
  const seen = new Set<string>();
  const normalized: string[] = [];

  for (const item of items) {
    if (typeof item !== 'string') {
      continue;
    }
    const name = item.trim().replace(/^[\\/]+|[\\/]+$/g, '').trim();
    if (!name || name.includes('/') || name.includes('\\') || seen.has(name)) {
      continue;
    }
    seen.add(name);
    normalized.push(name);
  }

  return normalized;
};

interface SSHFileMappingModalProps {
  open: boolean;
  onClose: () => void;
  mapping: SSHFileMapping | null;
}

export const SSHFileMappingModal: React.FC<SSHFileMappingModalProps> = ({ open, onClose, mapping }) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();

  const isEdit = mapping !== null;

  const handleDirectoryModeChange = (checked: boolean) => {
    if (!checked) {
      return;
    }

    const currentExcludes = form.getFieldValue('directoryExcludes');
    if (!Array.isArray(currentExcludes) || currentExcludes.length === 0) {
      form.setFieldValue('directoryExcludes', [...DEFAULT_SSH_DIRECTORY_EXCLUDES]);
    }
  };

  useEffect(() => {
    if (open) {
      if (mapping && mapping.id) {
        form.setFieldsValue({
          ...mapping,
          directoryExcludes: mapping.directoryExcludes ?? [...DEFAULT_SSH_DIRECTORY_EXCLUDES],
          cleanupPaths: mapping.cleanupPaths ?? [],
        });
      } else {
        form.resetFields();
        form.setFieldsValue({
          module: mapping?.module || 'opencode',
          enabled: true,
          isPattern: false,
          isDirectory: false,
          directoryExcludes: mapping?.directoryExcludes ?? [...DEFAULT_SSH_DIRECTORY_EXCLUDES],
          cleanupPaths: mapping?.cleanupPaths ?? [],
        });
      }
    }
  }, [open, mapping, form]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      const id = mapping?.id || `custom-${Date.now()}`;

      const newMapping: SSHFileMapping = {
        ...values,
        id,
        directoryExcludes: values.isDirectory
          ? normalizeDirectoryExcludes(values.directoryExcludes)
          : [],
        cleanupPaths: supportsCleanupPaths({
          isDirectory: values.isDirectory,
          isPattern: values.isPattern,
          targetPath: values.remotePath,
          sourcePath: values.localPath,
        })
          ? normalizeCleanupPaths(values.cleanupPaths)
          : [],
      };

      if (isEdit && mapping?.id) {
        await sshUpdateFileMapping(newMapping);
      } else {
        await sshAddFileMapping(newMapping);
      }

      onClose();
    } catch (error) {
      console.error('Failed to save SSH mapping:', error);
    }
  };

  return (
    <Modal
      title={isEdit && mapping?.id ? t('settings.ssh.editMapping') : t('settings.ssh.addMapping')}
      open={open}
      onOk={handleSubmit}
      onCancel={onClose}
      width={600}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form form={form} layout="horizontal" labelCol={{ span: 6 }} wrapperCol={{ span: 18 }}>
        <Form.Item
          name="name"
          label={t('settings.ssh.mappingName')}
          rules={[{ required: true, message: t('settings.ssh.mappingNameRequired') }]}
        >
          <Input placeholder={t('settings.ssh.mappingNamePlaceholder')} />
        </Form.Item>

        <Form.Item
          name="module"
          label={t('settings.ssh.module')}
          rules={[{ required: true }]}
        >
          <Select>
            <Select.Option value="opencode">OpenCode</Select.Option>
            <Select.Option value="claude">Claude Code</Select.Option>
            <Select.Option value="claude_desktop">Claude Desktop</Select.Option>
            <Select.Option value="codex">Codex</Select.Option>
            <Select.Option value="grok">Grok</Select.Option>
            <Select.Option value="kimi">Kimi</Select.Option>
            <Select.Option value="openclaw">OpenClaw</Select.Option>
            <Select.Option value="geminicli">Gemini</Select.Option>
            <Select.Option value="antigravity">Antigravity CLI</Select.Option>
            <Select.Option value="zcode">ZCode</Select.Option>
            <Select.Option value="pi">Pi</Select.Option>
            <Select.Option value="oh_my_pi">omp</Select.Option>
            <Select.Option value="omo_native">omo</Select.Option>
            <Select.Option value="hermes">Hermes</Select.Option>
            <Select.Option value="dsh">dsh</Select.Option>
            <Select.Option value="workbuddy">WorkBuddy</Select.Option>
            <Select.Option value="codebuddy">CodeBuddy</Select.Option>
          </Select>
        </Form.Item>

        <Divider />

        <Form.Item
          name="localPath"
          label={t('settings.ssh.localPath')}
          rules={[{ required: true, message: t('settings.ssh.localPathRequired') }]}
          extra={t('settings.ssh.localPathHint')}
        >
          <Input placeholder="~/.config/opencode/config.json" />
        </Form.Item>

        <Form.Item
          name="remotePath"
          label={t('settings.ssh.remotePath')}
          rules={[{ required: true, message: t('settings.ssh.remotePathRequired') }]}
          extra={t('settings.ssh.remotePathHint')}
        >
          <Input placeholder="~/.config/opencode/config.json" />
        </Form.Item>

        <Divider />

        <Form.Item
          name="enabled"
          label={
            <Space>
              <Text>{t('settings.ssh.enableMapping')}</Text>
            </Space>
          }
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>

        <Form.Item
          name="isPattern"
          label={t('settings.ssh.patternMode')}
          valuePropName="checked"
          extra={t('settings.ssh.patternModeHint')}
        >
          <Switch />
        </Form.Item>

        <Form.Item
          name="isDirectory"
          label={t('settings.ssh.directoryMode')}
          valuePropName="checked"
          extra={t('settings.ssh.directoryModeHint')}
        >
          <Switch onChange={handleDirectoryModeChange} />
        </Form.Item>

        <Form.Item
          noStyle
          shouldUpdate={(previousValues, currentValues) => previousValues.isDirectory !== currentValues.isDirectory}
        >
          {({ getFieldValue }) => {
            if (!getFieldValue('isDirectory')) {
              return null;
            }

            return (
              <Form.Item
                name="directoryExcludes"
                label={t('settings.ssh.directoryExcludes')}
                extra={t('settings.ssh.directoryExcludesHint')}
              >
                <Select
                  mode="tags"
                  tokenSeparators={[',', '\n']}
                  placeholder={t('settings.ssh.directoryExcludesPlaceholder')}
                />
              </Form.Item>
            );
          }}
        </Form.Item>

        <Form.Item
          noStyle
          shouldUpdate={(previousValues, currentValues) =>
            previousValues.isDirectory !== currentValues.isDirectory ||
            previousValues.isPattern !== currentValues.isPattern ||
            previousValues.localPath !== currentValues.localPath ||
            previousValues.remotePath !== currentValues.remotePath
          }
        >
          {({ getFieldValue }) => {
            if (!supportsCleanupPaths({
              isDirectory: getFieldValue('isDirectory'),
              isPattern: getFieldValue('isPattern'),
              targetPath: getFieldValue('remotePath'),
              sourcePath: getFieldValue('localPath'),
            })) {
              return null;
            }

            return (
              <Form.Item
                name="cleanupPaths"
                label={t('settings.ssh.cleanupPaths')}
                extra={t('settings.ssh.cleanupPathsHint')}
                rules={[
                  {
                    validator: (_, value) => {
                      const invalidPaths = invalidCleanupPaths(value);
                      if (invalidPaths.length > 0) {
                        return Promise.reject(
                          new Error(t('settings.ssh.cleanupPathsInvalid', { path: invalidPaths[0] })),
                        );
                      }
                      return Promise.resolve();
                    },
                  },
                ]}
              >
                <Select
                  mode="tags"
                  tokenSeparators={[',', '\n']}
                  placeholder={t('settings.ssh.cleanupPathsPlaceholder')}
                />
              </Form.Item>
            );
          }}
        </Form.Item>
      </Form>
    </Modal>
  );
};
