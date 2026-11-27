import React from 'react';
import {
  App,
  Button,
  Card,
  Divider,
  InputNumber,
  Select,
  Space,
  Switch,
  Tag,
  Typography,
} from 'antd';
import {
  ApiOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
  CodeOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import FieldHelp from '@/components/common/FieldHelp';
import {
  getControlApiCliStatus,
  getControlApiSettings,
  getControlApiStatus,
  installControlApiCli,
  saveControlApiSettings,
  uninstallControlApiCli,
  type ControlApiApprovalMode,
  type ControlApiCliStatus,
  type ControlApiSettings,
  type ControlApiStatus,
} from '@/services';
import styles from './ControlApiSettingsCard.module.less';

const { Text } = Typography;

const DEFAULT_SETTINGS: ControlApiSettings = {
  enabled: true,
  approval_mode: 'always_ask',
  auto_approve_local_plugin_reload: false,
  approval_timeout_secs: 60,
};

export const ControlApiSettingsCard: React.FC = () => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [settings, setSettings] = React.useState<ControlApiSettings>(DEFAULT_SETTINGS);
  const [status, setStatus] = React.useState<ControlApiStatus | null>(null);
  const [cliStatus, setCliStatus] = React.useState<ControlApiCliStatus | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [cliBusy, setCliBusy] = React.useState(false);
  const [timeoutDraft, setTimeoutDraft] = React.useState<number | null>(60);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [nextSettings, nextStatus, nextCliStatus] = await Promise.all([
        getControlApiSettings(),
        getControlApiStatus(),
        getControlApiCliStatus(),
      ]);
      setSettings(nextSettings);
      setTimeoutDraft(nextSettings.approval_timeout_secs);
      setStatus(nextStatus);
      setCliStatus(nextCliStatus);
    } catch (error) {
      console.error('Failed to load control API settings:', error);
      message.error(t('settings.controlApi.loadFailed', { error: String(error) }));
    } finally {
      setLoading(false);
    }
  }, [message, t]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  const save = React.useCallback(
    async (patch: Partial<ControlApiSettings>) => {
      const next = { ...settings, ...patch };
      setSaving(true);
      try {
        const nextStatus = await saveControlApiSettings(next);
        setSettings(next);
        setStatus(nextStatus);
        message.success(t('settings.controlApi.saved'));
      } catch (error) {
        console.error('Failed to save control API settings:', error);
        message.error(t('settings.controlApi.saveFailed', { error: String(error) }));
        // Re-read the persisted values so the switches never drift from disk.
        void refresh();
      } finally {
        setSaving(false);
      }
    },
    [message, refresh, settings, t],
  );

  const handleTimeoutBlur = React.useCallback(() => {
    const next = Math.min(300, Math.max(15, Math.round(timeoutDraft ?? 60)));
    setTimeoutDraft(next);
    if (next !== settings.approval_timeout_secs) {
      void save({ approval_timeout_secs: next });
    }
  }, [save, settings.approval_timeout_secs, timeoutDraft]);

  const handleCliInstall = React.useCallback(async () => {
    setCliBusy(true);
    try {
      const next = await installControlApiCli();
      setCliStatus(next);
      message.success(t('settings.controlApi.cliInstalled'));
    } catch (error) {
      console.error('Failed to install aitb:', error);
      message.error(t('settings.controlApi.cliInstallFailed', { error: String(error) }));
    } finally {
      setCliBusy(false);
    }
  }, [message, t]);

  const handleCliUninstall = React.useCallback(async () => {
    setCliBusy(true);
    try {
      const next = await uninstallControlApiCli();
      setCliStatus(next);
      message.success(t('settings.controlApi.cliRemoved'));
    } catch (error) {
      console.error('Failed to uninstall aitb:', error);
      message.error(t('settings.controlApi.cliRemoveFailed', { error: String(error) }));
    } finally {
      setCliBusy(false);
    }
  }, [message, t]);

  const running = status?.running === true;
  const approvalMode = settings.approval_mode;

  return (
    <Card
      className={styles.card}
      title={
        <div className={styles.cardTitle}>
          <span style={{ color: '#1890ff' }}>
            <ApiOutlined />
          </span>
          <span>{t('settings.controlApi.title')}</span>
        </div>
      }
      extra={
        <Space size={8}>
          <Tag
            color={running ? 'green' : 'default'}
            icon={running ? <CheckCircleOutlined /> : <CloseCircleOutlined />}
          >
            {running
              ? t('settings.controlApi.statusRunning')
              : t('settings.controlApi.statusStopped')}
          </Tag>
          <Button
            size="small"
            type="text"
            icon={<ReloadOutlined spin={loading} />}
            onClick={() => void refresh()}
            aria-label={t('settings.controlApi.refresh')}
          />
        </Space>
      }
    >
      <Text type="secondary" className={styles.hint}>
        {t('settings.controlApi.description')}
      </Text>

      <div className={styles.row}>
        <Space size={4}>
          <Text>{t('settings.controlApi.enabled')}</Text>
          <FieldHelp text={t('settings.controlApi.enabledHelp')} />
        </Space>
        <Switch
          checked={settings.enabled}
          loading={saving}
          disabled={loading}
          onChange={(checked) => void save({ enabled: checked })}
          aria-label={t('settings.controlApi.enabled')}
        />
      </div>

      <div className={styles.metaBlock}>
        <div className={styles.metaRow}>
          <Text type="secondary">{t('settings.controlApi.url')}</Text>
          <Text code>{status?.url ?? '-'}</Text>
        </div>
        <div className={styles.metaRow}>
          <Text type="secondary">{t('settings.controlApi.discoveryFile')}</Text>
          <Text className={styles.path} title={status?.discoveryFile}>
            {status?.discoveryFile ?? '-'}
          </Text>
        </div>
      </div>

      <Divider className={styles.divider} />

      <div className={styles.row}>
        <Space size={4}>
          <Text>{t('settings.controlApi.approvalMode')}</Text>
          <FieldHelp text={t('settings.controlApi.approvalModeHelp')} />
        </Space>
        <Select<ControlApiApprovalMode>
          value={approvalMode}
          loading={saving}
          disabled={loading}
          onChange={(value) => void save({ approval_mode: value })}
          className={styles.approvalSelect}
          options={[
            {
              value: 'always_ask',
              label: t('settings.controlApi.approvalModeAlwaysAsk'),
            },
            {
              value: 'ask_high_risk',
              label: t('settings.controlApi.approvalModeAskHighRisk'),
            },
            {
              value: 'auto_approve',
              label: t('settings.controlApi.approvalModeAutoApprove'),
            },
            {
              value: 'read_only',
              label: t('settings.controlApi.approvalModeReadOnly'),
            },
          ]}
        />
      </div>
      <Text type="secondary" className={styles.hint}>
        {t(`settings.controlApi.approvalModeHint.${approvalMode}`)}
      </Text>

      <div className={styles.row}>
        <Space size={4}>
          <Text>{t('settings.controlApi.approvalTimeout')}</Text>
          <FieldHelp text={t('settings.controlApi.approvalTimeoutHelp')} />
        </Space>
        <InputNumber
          min={15}
          max={300}
          step={5}
          value={timeoutDraft}
          disabled={loading}
          onChange={setTimeoutDraft}
          onBlur={handleTimeoutBlur}
          suffix={t('settings.controlApi.seconds')}
          aria-label={t('settings.controlApi.approvalTimeout')}
        />
      </div>

      <div className={styles.row}>
        <Space size={4}>
          <Text>{t('settings.controlApi.autoApproveLocalReload')}</Text>
          <FieldHelp text={t('settings.controlApi.autoApproveLocalReloadHelp')} />
        </Space>
        <Switch
          checked={settings.auto_approve_local_plugin_reload}
          loading={saving}
          disabled={loading || approvalMode !== 'always_ask'}
          onChange={(checked) =>
            void save({ auto_approve_local_plugin_reload: checked })
          }
          aria-label={t('settings.controlApi.autoApproveLocalReload')}
        />
      </div>

      <Divider className={styles.divider} />

      <div className={styles.cliHeader}>
        <Space size={6}>
          <CodeOutlined />
          <Text strong>{t('settings.controlApi.cliTitle')}</Text>
        </Space>
        <Tag color={cliStatus?.installed ? 'blue' : 'default'}>
          {cliStatus?.installed
            ? t('settings.controlApi.cliInstalledTag')
            : t('settings.controlApi.cliNotInstalledTag')}
        </Tag>
      </div>
      <Text type="secondary" className={styles.hint}>
        {t('settings.controlApi.cliDescription')}
      </Text>
      <div className={styles.metaRow}>
        <Text type="secondary">{t('settings.controlApi.cliPath')}</Text>
        <Text className={styles.path} title={cliStatus?.installPath ?? undefined}>
          {cliStatus?.installPath ?? '-'}
        </Text>
      </div>
      {cliStatus?.installed && !cliStatus.onPath && (
        <Text type="warning" className={styles.hint}>
          {t('settings.controlApi.cliNotOnPath')}
        </Text>
      )}
      {!cliStatus?.installed && !cliStatus?.binarySource && (
        <Text type="secondary" className={styles.hint}>
          {t('settings.controlApi.cliBuildHint')}
        </Text>
      )}
      <Space wrap className={styles.cliActions}>
        {cliStatus?.installed ? (
          <Button
            danger
            loading={cliBusy}
            onClick={() => void handleCliUninstall()}
          >
            {t('settings.controlApi.cliRemove')}
          </Button>
        ) : (
          <Button
            type="primary"
            icon={<CodeOutlined />}
            loading={cliBusy}
            disabled={!cliStatus?.binarySource}
            onClick={() => void handleCliInstall()}
          >
            {t('settings.controlApi.cliInstall')}
          </Button>
        )}
      </Space>
      {!cliStatus?.installed && cliStatus?.binarySource && (
        <Text type="secondary" className={styles.hint}>
          {t('settings.controlApi.cliSource', { path: cliStatus.binarySource })}
        </Text>
      )}
      <div className={styles.securityNote}>
        <SafetyCertificateOutlined />
        <Text type="secondary">{t('settings.controlApi.securityNote')}</Text>
      </div>
    </Card>
  );
};

export default ControlApiSettingsCard;
