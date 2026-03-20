/**
 * File Mapping Edit Modal
 *
 * Modal for adding/editing file mappings
 */

import React, { useEffect } from 'react';
import { Modal, Form, Input, Select, Switch, Divider, Button, Modal as AntdModal } from 'antd';
import { useTranslation } from 'react-i18next';
import { wslAddFileMapping, wslUpdateFileMapping } from '@/services/wslSyncApi';
import type { FileMapping } from '@/types/wslsync';

interface FileMappingModalProps {
  open: boolean;
  onClose: () => void;
  mapping: FileMapping | null;
}

interface PathIssue {
  windowsPathIssue: boolean;
  wslPathIssue: boolean;
  fixedWindowsPath?: string;
  fixedWslPath?: string;
}

/**
 * Detect path separator issues
 * - Windows path should use backslashes (\)
 * - WSL path should use forward slashes (/)
 */
const detectPathIssues = (windowsPath: string, wslPath: string): PathIssue => {
  let windowsPathIssue = false;
  let wslPathIssue = false;
  let fixedWindowsPath = windowsPath;
  let fixedWslPath = wslPath;

  // Check Windows path: should use backslashes only
  // Detect forward slashes in the path (ignore :// in protocols if any)
  if (windowsPath.includes('/')) {
    windowsPathIssue = true;
    fixedWindowsPath = windowsPath.replace(/\//g, '\\');
  }

  // Check WSL path: should use forward slashes only
  if (wslPath.includes('\\')) {
    wslPathIssue = true;
    fixedWslPath = wslPath.replace(/\\/g, '/');
  }

  return {
    windowsPathIssue,
    wslPathIssue,
    fixedWindowsPath: windowsPathIssue ? fixedWindowsPath : undefined,
    fixedWslPath: wslPathIssue ? fixedWslPath : undefined,
  };
};

export const FileMappingModal: React.FC<FileMappingModalProps> = ({ open, onClose, mapping }) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();

  const isEdit = mapping !== null;

  useEffect(() => {
    if (open) {
      if (mapping && mapping.id) {
        form.setFieldsValue(mapping);
      } else {
        form.resetFields();
        form.setFieldsValue({
          module: mapping?.module || 'opencode',
          enabled: true,
          isPattern: false,
          isDirectory: false,
        });
      }
    }
  }, [open, mapping, form]);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();

      // Check for path separator issues
      const issues = detectPathIssues(values.windowsPath, values.wslPath);

      if (issues.windowsPathIssue || issues.wslPathIssue) {
        // Show confirmation dialog with fix option
        let message = t('settings.wsl.pathSeparatorWarning') + '\n\n';
        if (issues.windowsPathIssue) {
          message += `• ${t('settings.wsl.windowsPathShouldUseBackslash')}\n  ${t('common.current')}: ${values.windowsPath}\n  ${t('common.suggestedFix')}: ${issues.fixedWindowsPath}\n\n`;
        }
        if (issues.wslPathIssue) {
          message += `• ${t('settings.wsl.wslPathShouldUseForwardSlash')}\n  ${t('common.current')}: ${values.wslPath}\n  ${t('common.suggestedFix')}: ${issues.fixedWslPath}\n\n`;
        }
        message += t('settings.wsl.continueSaveQuestion');

        AntdModal.confirm({
          title: t('settings.wsl.pathSeparatorCheck'),
          content: message,
          okText: t('settings.wsl.fixAndSave'),
          cancelText: t('settings.wsl.saveAsIs'),
          okButtonProps: { type: 'primary' },
          cancelButtonProps: { type: 'default' },
          onOk: () => {
            // Apply fixes and save
            const fixedValues = { ...values };
            if (issues.fixedWindowsPath) {
              fixedValues.windowsPath = issues.fixedWindowsPath;
            }
            if (issues.fixedWslPath) {
              fixedValues.wslPath = issues.fixedWslPath;
            }
            form.setFieldsValue(fixedValues);
            saveMapping(fixedValues);
          },
          onCancel: () => {
            // Save without fixing
            saveMapping(values);
          },
          // Add a third cancel button using footer
          footer: (_, { OkBtn, CancelBtn }) => (
            <>
              <Button onClick={() => AntdModal.destroyAll()}>{t('common.cancel')}</Button>
              <CancelBtn />
              <OkBtn />
            </>
          ),
        });
        return;
      }

      // No issues, save directly
      saveMapping(values);
    } catch (error) {
      console.error('Failed to save mapping:', error);
    }
  };

  const saveMapping = async (values: any) => {
    try {
      // Generate ID if new
      const id = mapping?.id || `custom-${Date.now()}`;

      const newMapping: FileMapping = {
        ...values,
        id,
      };

      // Save to database (will trigger wsl-config-changed event to refresh UI)
      if (isEdit && mapping?.id) {
        await wslUpdateFileMapping(newMapping);
      } else {
        await wslAddFileMapping(newMapping);
      }

      onClose();
    } catch (error) {
      console.error('Failed to save mapping:', error);
    }
  };

  return (
    <Modal
      title={isEdit && mapping?.id ? t('settings.wsl.editMapping') : t('settings.wsl.addMapping')}
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
          label={t('settings.wsl.mappingName')}
          rules={[{ required: true, message: t('settings.wsl.mappingNameRequired') }]}
        >
          <Input placeholder={t('settings.wsl.mappingNamePlaceholder')} />
        </Form.Item>

        <Form.Item
          name="module"
          label={t('settings.wsl.module')}
          rules={[{ required: true }]}
        >
          <Select>
            <Select.Option value="opencode">OpenCode</Select.Option>
            <Select.Option value="claude">Claude Code</Select.Option>
            <Select.Option value="codex">Codex</Select.Option>
            <Select.Option value="openclaw">OpenClaw</Select.Option>
          </Select>
        </Form.Item>

        <Divider />

        <Form.Item
          name="windowsPath"
          label={t('settings.wsl.windowsPath')}
          rules={[{ required: true, message: t('settings.wsl.windowsPathRequired') }]}
          extra={t('settings.wsl.windowsPathHint')}
        >
          <Input placeholder="%USERPROFILE%\.config\opencode\config.json" />
        </Form.Item>

        <Form.Item
          name="wslPath"
          label={t('settings.wsl.wslPath')}
          rules={[{ required: true, message: t('settings.wsl.wslPathRequired') }]}
          extra={t('settings.wsl.wslPathHint')}
        >
          <Input placeholder="~/.config/opencode/config.json" />
        </Form.Item>

        <Divider />

        <Form.Item
          name="enabled"
          label={t('settings.wsl.enableMapping')}
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>

        <Form.Item
          name="isPattern"
          label={t('settings.wsl.patternMode')}
          valuePropName="checked"
          extra={t('settings.wsl.patternModeHint')}
        >
          <Switch />
        </Form.Item>

        <Form.Item
          name="isDirectory"
          label={t('settings.wsl.directoryMode')}
          valuePropName="checked"
          extra={t('settings.wsl.directoryModeHint')}
        >
          <Switch />
        </Form.Item>
      </Form>
    </Modal>
  );
};
