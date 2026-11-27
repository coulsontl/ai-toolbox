import React from 'react';
import { Tooltip } from 'antd';
import type { ProviderCardState } from './types';

type PluginBadge = NonNullable<ProviderCardState['pluginBadge']>;

const BADGE_STYLE: React.CSSProperties = {
  display: 'inline-flex',
  height: 16,
  minHeight: 16,
  flex: '0 0 auto',
  alignItems: 'center',
  padding: '0 5px',
  border: '1px solid var(--color-border)',
  borderRadius: 4,
  background: 'var(--color-bg-hover)',
  color: 'var(--color-text-secondary)',
  fontSize: 10,
  fontWeight: 560,
  lineHeight: '14px',
  whiteSpace: 'nowrap',
};

/**
 * The plugin-origin chip shared by every provider card style and the compact
 * grid cell.
 *
 * A fixed 16px box with `flex: 0 0 auto` is deliberate: a plugin badge must not
 * change the title row's height or get squeezed by a long provider name. The
 * tooltip carries the provider id and any context from the bridge, which is
 * where a user checks "which channel is this, exactly".
 */
const ProviderPluginBadge: React.FC<{ badge: PluginBadge }> = ({ badge }) => {
  const chip = (
    <span style={BADGE_STYLE} data-plugin-provider-id={badge.providerId}>
      {badge.label}
    </span>
  );

  return badge.tooltip ? (
    <Tooltip title={badge.tooltip}>
      <span style={{ display: 'inline-flex', flex: '0 0 auto' }}>{chip}</span>
    </Tooltip>
  ) : chip;
};

export default ProviderPluginBadge;
