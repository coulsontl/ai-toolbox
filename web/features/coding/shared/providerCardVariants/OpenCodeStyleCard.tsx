import React from 'react';
import { Button, Popconfirm, Space, Tooltip, Typography } from 'antd';
import {
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
} from '@ant-design/icons';
import { Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import SdkTag from '@/components/common/SdkTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import CardShell from './CardShell';
import ProviderModelSection from './ProviderModelSection';
import { ProviderEnabledSwitch } from './providerCardActions';
import type { ProviderCardVariantProps } from './types';

const { Text } = Typography;

/**
 * The OpenCode-style provider card.
 *
 * Two lines of header, then a collapsible model section:
 *
 * ```
 * ⠿  Name                          [Switch] ✎ ⧉ ⤴ 🗑
 *    ID: xxx • @ai-sdk/… • https://…
 *    > 模型列表 (3)        [批量删除] [模型测试] [获取模型] [+ 添加模型]
 * ```
 *
 * The distinguishing traits, against the other two styles:
 *
 * - The **second line is uniform**: id, SDK family and endpoint, each optional,
 *   always in that order. The Claude style uses that line for role/model
 *   bindings instead, and the Codex style for a free-form summary.
 * - Header actions are **icon buttons**, not text links. Text links in a header
 *   row compete with the provider name for the eye; icons do not.
 * - There is **no header-level apply action**. "Which one is active" is
 *   expressed on the model row (`设为默认`), because in a multi-channel CLI a
 *   provider is not switched on or off — one model within it is made the
 *   default. A card that offers "应用" here would imply the others are off.
 */
const OpenCodeStyleCard: React.FC<ProviderCardVariantProps> = ({
  provider,
  providerState,
  actions,
  modelSection,
  nameTags,
  footer,
}) => {
  const { t } = useTranslation();
  const {
    isDisabled,
    onToggleDisabled,
    connectivityStatus,
    selectable = false,
    selected = false,
    onSelectChange,
    selectDisabledReason,
    dimmed = false,
    draggable = false,
    sortableId,
    accent,
  } = providerState ?? {};

  const showId = provider.name !== provider.id;

  return (
    <CardShell
      sortableId={sortableId}
      draggable={draggable}
      selectable={selectable}
      selected={selected}
      onSelectChange={onSelectChange}
      selectDisabledReason={selectDisabledReason}
      dimmed={dimmed}
      accent={accent}
    >
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'flex-start',
          marginBottom: 8,
        }}
      >
        <div style={{ minWidth: 0 }}>
          <div style={{ marginBottom: 4, display: 'flex', alignItems: 'center', gap: 8 }}>
            <ProviderConnectivityStatus item={connectivityStatus} />
            <ProviderNameLink
              name={provider.name}
              baseUrl={provider.baseUrl}
              style={{ fontSize: 14, fontWeight: 600 }}
            />
            {nameTags}
          </div>
          <Space size={8} wrap>
            {/* Separators are emitted *between* entries, never after the last
                one: a trailing "•" reads as a missing value. */}
            {[
              showId ? (
                <Text key="id" type="secondary" style={{ fontSize: 12 }}>
                  ID: {provider.id}
                </Text>
              ) : null,
              provider.sdkName ? <SdkTag key="sdk" name={provider.sdkName} /> : null,
              provider.baseUrl ? (
                <Text key="url" type="secondary" style={{ fontSize: 12 }}>
                  {provider.baseUrl}
                </Text>
              ) : null,
            ]
              .filter(Boolean)
              .flatMap((entry, index, all) =>
                index < all.length - 1
                  ? [entry, <Text key={`sep-${index}`} type="secondary" style={{ fontSize: 12 }}>•</Text>]
                  : [entry],
              )}
          </Space>
        </div>

        <Space>
          {onToggleDisabled && (
            <ProviderEnabledSwitch isDisabled={isDisabled} onToggleDisabled={onToggleDisabled} />
          )}
          {actions?.onEdit && (
            <Button size="small" icon={<EditOutlined />} onClick={actions.onEdit} />
          )}
          {actions?.onCopy && (
            <Button size="small" icon={<CopyOutlined />} onClick={actions.onCopy} />
          )}
          {actions?.onShare && (
            <Tooltip title={t('common.share')}>
              <Button
                size="small"
                aria-label={t('common.share')}
                icon={<Share2 size={14} />}
                onClick={actions.onShare}
              />
            </Tooltip>
          )}
          {actions?.onDelete && actions.deleteDisabledReason ? (
            <Tooltip title={actions.deleteDisabledReason}>
              <span>
                <Button size="small" danger icon={<DeleteOutlined />} disabled />
              </span>
            </Tooltip>
          ) : actions?.onDelete && (actions.deleteConfirm ? (
            <Popconfirm
              title={t('common.provider.deleteProvider')}
              description={t('common.provider.confirmDelete', { name: provider.name })}
              onConfirm={actions.onDelete}
              okText={t('common.confirm')}
              cancelText={t('common.cancel')}
            >
              <Button size="small" danger icon={<DeleteOutlined />} />
            </Popconfirm>
          ) : (
            <Button size="small" danger icon={<DeleteOutlined />} onClick={actions.onDelete} />
          ))}
        </Space>
      </div>

      {footer}

      <ProviderModelSection providerId={provider.id} modelSection={modelSection} />
    </CardShell>
  );
};

export default OpenCodeStyleCard;
