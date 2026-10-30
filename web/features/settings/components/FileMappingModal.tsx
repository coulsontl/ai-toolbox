/**
 * File Mapping Edit Modal
 *
 * Modal for adding/editing file mappings
 */

import React, { useEffect } from 'react';
import { Modal, Form, Input, Select, Switch, Divider, Button, Modal as AntdModal } from 'antd';
import { useTranslation } from 'react-i18next';
import { wslAddFileMapping, wslUpdateFileMapping } from '@/services/wslSyncApi';
import {
  invalidCleanupPaths,
  normalizeCleanupPaths,
  supportsCleanupPaths,
} from '@/features/settings/utils/fileMappingCleanup';
import { DEFAULT_SSH_DIRECTORY_EXCLUDES } from '@/types/sshsync';
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

export const FileMappingModal: React.FC<FileMappingModalProps> = ({ open, onClose, mapping }) => {
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
          directoryExcludes: [...DEFAULT_SSH_DIRECTORY_EXCLUDES],
          cleanupPaths: [],
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
        directoryExcludes: values.isDirectory
          ? normalizeDirectoryExcludes(values.directoryExcludes)
          : [],
        cleanupPaths: supportsCleanupPaths({
          isDirectory: values.isDirectory,
          isPattern: values.isPattern,
          targetPath: values.wslPath,
          sourcePath: values.windowsPath,
        })
          ? normalizeCleanupPaths(values.cleanupPaths)
          : [],
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
          <Switch onChange={handleDirectoryModeChange} />
        </Form.Item>

        <Form.Item
          noStyle
          shouldUpdate={(previousValues, currentValues) =>
            previousValues.isDirectory !== currentValues.isDirectory
          }
        >
          {({ getFieldValue }) => {
            if (!getFieldValue('isDirectory')) {
              return null;
            }

            return (
              <Form.Item
                name="directoryExcludes"
                label={t('settings.wsl.directoryExcludes')}
                extra={t('settings.wsl.directoryExcludesHint')}
              >
                <Select
                  mode="tags"
                  tokenSeparators={[',', '\n']}
                  placeholder={t('settings.wsl.directoryExcludesPlaceholder')}
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
            previousValues.windowsPath !== currentValues.windowsPath ||
            previousValues.wslPath !== currentValues.wslPath
          }
        >
          {({ getFieldValue }) => {
            if (!supportsCleanupPaths({
              isDirectory: getFieldValue('isDirectory'),
              isPattern: getFieldValue('isPattern'),
              targetPath: getFieldValue('wslPath'),
              sourcePath: getFieldValue('windowsPath'),
            })) {
              return null;
            }

            return (
              <Form.Item
                name="cleanupPaths"
                label={t('settings.wsl.cleanupPaths')}
                extra={t('settings.wsl.cleanupPathsHint')}
                rules={[
                  {
                    validator: (_, value) => {
                      const invalidPaths = invalidCleanupPaths(value);
                      if (invalidPaths.length > 0) {
                        return Promise.reject(
                          new Error(t('settings.wsl.cleanupPathsInvalid', { path: invalidPaths[0] })),
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
                  placeholder={t('settings.wsl.cleanupPathsPlaceholder')}
                />
              </Form.Item>
            );
          }}
        </Form.Item>
      </Form>
    </Modal>
  );
};
