import React from 'react';
import { App, Collapse, Form, Space, Tabs, Typography } from 'antd';
import { SettingOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import JsoncEditor from '@/components/common/JsoncEditor';
import {
  saveOmoNativeOtherConfig,
  saveOmoNativeOtherSettings,
} from '@/services/omoNativeApi';
import type { OmoNativeRuntimeConfig } from '@/types/omoNative';
import {
  buildOtherSettingsPatch,
  parseOtherConfigPayload,
} from '../utils/omoNativeOtherSettings';
import { isRecordEmpty } from '../utils/omoNativeProviders';

const { Text } = Typography;

interface OmoNativeOtherConfigSectionProps {
  /** 共享 base 键（`omo.jsonc` 顶层、不属于任何 harness 块的部分）。 */
  sharedBase: Record<string, unknown>;
  /** `settings.json` 中不由本页其他区块管理的键。 */
  otherSettings: Record<string, unknown>;
  /** `omo.jsonc` 保存后重读配置，让生效视图与预览跟上。 */
  onSaved: () => Promise<void> | void;
  /** `settings.json` 保存命令已返回整份新配置，直接更新页面，避免二次全量读取。 */
  onSettingsSaved: (config: OmoNativeRuntimeConfig) => void;
}

/**
 * 「其他配置」区块：两个 Tab 分别编辑两份文件。
 *
 * - `omo.jsonc`：不属于任何 harness 块的顶层共享键。**不能整份重写文件**：
 *   `[native]` / `[opencode]` 两个块、控制键与全部注释都要原样保留，后端
 *   `save_omo_native_other_config` 按顶层键做原地补丁。
 * - `settings.json`：引擎 settings 里不由本页其他区块管理的键（如 `lookAt`）。
 *   `defaultProvider` / `defaultModel` / `defaultThinkingLevel` 归「模型设置」，
 *   `packages` / `extensions` 归「扩展」；前端只提交相对基线改动的 `set` /
 *   `remove`，后端 `save_omo_native_other_settings` 在盘上最新内容打补丁，
 *   受保护键原样保留。
 *
 * 形态与 OpenCode / Pi / OMP 的同名区块一致（`Collapse` + 失焦自动保存）。
 *
 * ⚠️ **用 `JsoncEditor` 而不是 `JsonEditor`**：`omo.jsonc` 是 JSONC 文件；
 * 引擎的 `settings.jsonc` 回退文件同样允许注释与尾逗号。`JsonEditor` 用
 * `JSON.parse` 校验，注释和尾逗号会被判为语法错误，失焦保存会静默跳过。
 * `JsoncEditor` 用 JSON5 解析，与后端 `json5::from_str` 的读法一致。
 */
const OmoNativeOtherConfigSection: React.FC<OmoNativeOtherConfigSectionProps> = ({
  sharedBase,
  otherSettings,
  onSaved,
  onSettingsSaved,
}) => {
  const { t } = useTranslation();
  const [collapsed, setCollapsed] = React.useState(true);

  const saveSharedBase = React.useCallback(
    async (payload: Record<string, unknown>) => {
      await saveOmoNativeOtherConfig(payload);
      await onSaved();
    },
    [onSaved],
  );

  const saveOtherSettings = React.useCallback(
    async (payload: Record<string, unknown>, baseline: Record<string, unknown>) => {
      const nextConfig = await saveOmoNativeOtherSettings(
        buildOtherSettingsPatch(baseline, payload),
      );
      onSettingsSaved(nextConfig);
    },
    [onSettingsSaved],
  );

  return (
    <div
      id="omo-native-other-configuration"
      data-sidebar-section="true"
      data-sidebar-title={t('omoNative.otherConfig.title')}
    >
      <Collapse
        style={{ marginBottom: 16 }}
        activeKey={collapsed ? [] : ['other']}
        onChange={(keys) => setCollapsed(!keys.includes('other'))}
        items={[
          {
            key: 'other',
            label: (
              <Space>
                <SettingOutlined style={{ marginRight: 8 }} />
                <Text strong>{t('omoNative.otherConfig.title')}</Text>
              </Space>
            ),
            children: (
              <Tabs
                defaultActiveKey="config"
                items={[
                  {
                    key: 'config',
                    label: 'omo.jsonc',
                    children: (
                      <OtherConfigEditor
                        value={sharedBase}
                        hint={t('omoNative.otherConfig.hint')}
                        autoSaveHint={t('omoNative.otherConfig.autoSaveHint')}
                        onSave={saveSharedBase}
                      />
                    ),
                  },
                  {
                    key: 'settings',
                    label: 'settings.json',
                    children: (
                      <OtherConfigEditor
                        value={otherSettings}
                        hint={t('omoNative.otherConfig.settingsHint')}
                        autoSaveHint={t('omoNative.otherConfig.autoSaveHint')}
                        onSave={saveOtherSettings}
                      />
                    ),
                  },
                ]}
              />
            ),
          },
        ]}
      />
    </div>
  );
};

interface OtherConfigEditorProps {
  value: Record<string, unknown>;
  hint: string;
  autoSaveHint: string;
  onSave: (
    payload: Record<string, unknown>,
    baseline: Record<string, unknown>,
  ) => Promise<void>;
}

/**
 * 单个文件的 JSONC 编辑器。两个 Tab 共用同一套「外部刷新 / 失焦保存 / 错误提示」
 * 逻辑，只有数据源与保存命令不同。
 */
const OtherConfigEditor: React.FC<OtherConfigEditorProps> = ({
  value,
  hint,
  autoSaveHint,
  onSave,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();

  // `JsoncEditor` 收发的是**文本**：它要保留注释与格式，交回来的必须是原文。
  // 解析（JSON5）在 `onChange` / `onBlur` 的第三个参数里完成。
  const displayText = React.useMemo(() => formatConfig(value), [value]);
  const [text, setText] = React.useState(displayText);
  const [focused, setFocused] = React.useState(false);

  // 编辑器文本当前对应的服务端快照。聚焦期间 Monaco 不会应用外部 value，
  // 所以基线也不能跟着 `value` 前进——否则用户下一次失焦会把聚焦期间引擎
  // 写入的键（`tipsHistory` 等）误判成「用户删除」。
  const baselineRef = React.useRef(value);

  // 外部刷新（应用方案、恢复备份、切根目录）后要跟上，但用户正在编辑时不要打断。
  const lastExternalRef = React.useRef(displayText);
  React.useEffect(() => {
    if (lastExternalRef.current === displayText) return;
    if (focused) return;
    lastExternalRef.current = displayText;
    setText(displayText);
    baselineRef.current = value;
  }, [displayText, focused, value]);

  const handleBlur = async (
    rawText: string,
    parsed: unknown | null,
    isValid: boolean,
  ) => {
    const result = parseOtherConfigPayload(rawText, parsed, isValid);
    if (!result.ok) {
      if (result.reason === 'notObject') {
        message.error(t('omoNative.otherConfig.objectRequired'));
      }
      return;
    }

    const payload = result.payload;
    const baseline = baselineRef.current;
    if (JSON.stringify(payload) === JSON.stringify(baseline)) return;
    try {
      await onSave(payload, baseline);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save OmO Native other config:', error);
      message.error(error instanceof Error ? error.message : t('common.error'));
    }
  };

  return (
    <Form.Item
      help={
        <span>
          <Text type="secondary">{hint}，</Text>
          <span style={{ color: 'var(--ant-color-primary)' }}>{autoSaveHint}</span>
        </span>
      }
      style={{ marginBottom: 0 }}
    >
      <JsoncEditor
        value={text}
        height={260}
        onChange={(nextText) => setText(nextText)}
        onFocusChange={setFocused}
        onBlur={(nextText, isValid, parsed) => {
          void handleBlur(nextText, parsed, isValid);
        }}
      />
    </Form.Item>
  );
};

/** 把配置对象渲染成编辑器文本；空对象给 `{}` 而不是空串（空串会被当成"还没加载"）。 */
function formatConfig(value: Record<string, unknown>): string {
  return isRecordEmpty(value) ? '{}' : JSON.stringify(value, null, 2);
}

export default OmoNativeOtherConfigSection;
