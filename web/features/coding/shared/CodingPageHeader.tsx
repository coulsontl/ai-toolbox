import React from 'react';
import { Button, Space, Tooltip, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  EditOutlined,
  EllipsisOutlined,
  EyeOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  SyncOutlined,
} from '@ant-design/icons';
import { openUrl } from '@tauri-apps/plugin-opener';

const { Text, Title, Link } = Typography;

export interface CodingPageHeaderProps {
  /** Page title, e.g. `t('codex.title')`. */
  title: React.ReactNode;
  /** Docs URL. The "official docs" link is hidden when omitted. */
  docsUrl?: string;
  /** Shows the "preview config" link when provided. */
  onPreviewConfig?: () => void;

  /** Resolved config path; callers own their own fallback value. */
  configPath: string;
  /**
   * What `configPath` points at. Some CLIs manage a root **directory** rather
   * than one file (pi, oh_my_pi, dsh, hermes, omo_native), and calling a
   * directory a "config file path" is wrong rather than merely worded
   * differently. Both labels live in `common.*`; the caller only says which
   * kind of thing its path is. Defaults to `'file'`.
   */
  configPathKind?: 'file' | 'directory';
  /** Shows the "customize config dir" button when provided. */
  onCustomizeConfig?: () => void;
  customizeConfigDisabled?: boolean;
  /**
   * Why the customize button is disabled. A disabled antd button cannot show
   * its own tooltip (the browser drops pointer events on it), so the wrapper
   * below is what carries the explanation. Always pass it together with
   * `customizeConfigDisabled`, otherwise the button just looks broken.
   */
  customizeConfigDisabledReason?: string;
  /** Shows the "open folder" button when provided. */
  onOpenFolder?: () => void;
  /** Shows the "refresh config" button when provided. */
  onRefresh?: () => void;

  /** Shows the "more options" button when provided. */
  onMoreOptions?: () => void;

  /** Extra text buttons appended to the config-path row (e.g. OpenClaw's "Open Web UI"). */
  extraActions?: React.ReactNode;
}

const textButtonStyle: React.CSSProperties = { padding: 0, fontSize: 12 };

/**
 * Shared page header for every coding tab.
 *
 * Locks the header skeleton (title + docs/preview links, config path row with
 * customize/open/refresh, and the more-options entry) so newly onboarded CLIs
 * inherit the same layout instead of re-deriving it. Every label comes from
 * `common.*`: the wording is identical across tools, so per-CLI overrides only
 * produced near-duplicates ("刷新" vs "刷新配置") that drifted apart.
 */
const CodingPageHeader: React.FC<CodingPageHeaderProps> = ({
  title,
  docsUrl,
  onPreviewConfig,
  configPath,
  configPathKind = 'file',
  onCustomizeConfig,
  customizeConfigDisabled,
  customizeConfigDisabledReason,
  onOpenFolder,
  onRefresh,
  onMoreOptions,
  extraActions,
}) => {
  const { t } = useTranslation();

  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div>
          <div style={{ marginBottom: 8 }}>
            <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
              {title}
            </Title>
            {docsUrl && (
              <Link
                type="secondary"
                style={{ fontSize: 12 }}
                onClick={(event) => {
                  event.stopPropagation();
                  void openUrl(docsUrl);
                }}
              >
                <LinkOutlined /> {t('common.viewDocs')}
              </Link>
            )}
            {onPreviewConfig && (
              <Link
                type="secondary"
                style={{ fontSize: 12, marginLeft: 16 }}
                onClick={(event) => {
                  event.stopPropagation();
                  onPreviewConfig();
                }}
              >
                <EyeOutlined /> {t('common.previewConfig')}
              </Link>
            )}
          </div>
          <Space size="small">
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t(configPathKind === 'directory' ? 'common.configDir' : 'common.configPath')}:
            </Text>
            <Text code style={{ fontSize: 12 }}>
              {configPath}
            </Text>
            {onCustomizeConfig && (
              // A disabled antd button stops emitting pointer events, so a
              // Tooltip on the button itself never opens. The span keeps the
              // hover target; without the tooltip the disabled state has no
              // explanation on screen and reads as a broken button.
              <Tooltip title={customizeConfigDisabled ? customizeConfigDisabledReason : undefined}>
                <span>
                  <Button
                    type="text"
                    size="small"
                    icon={<EditOutlined />}
                    onClick={onCustomizeConfig}
                    disabled={customizeConfigDisabled}
                    style={textButtonStyle}
                  >
                    {t('common.customizeConfigDir')}
                  </Button>
                </span>
              </Tooltip>
            )}
            {onOpenFolder && (
              <Button
                type="text"
                size="small"
                icon={<FolderOpenOutlined />}
                onClick={onOpenFolder}
                style={textButtonStyle}
              >
                {t('common.openFolder')}
              </Button>
            )}
            {onRefresh && (
              <Button
                type="text"
                size="small"
                icon={<SyncOutlined />}
                onClick={onRefresh}
                style={textButtonStyle}
              >
                {t('common.refreshConfig')}
              </Button>
            )}
            {extraActions}
          </Space>
        </div>

        {onMoreOptions && (
          <Space>
            <Button type="text" icon={<EllipsisOutlined />} onClick={onMoreOptions}>
              {t('common.moreOptions')}
            </Button>
          </Space>
        )}
      </div>
    </div>
  );
};

export default CodingPageHeader;
