import type React from 'react';
import { Tooltip } from 'antd';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { getUrlOrigin } from '@/utils/urlOrigin';
import styles from './index.module.less';

interface ProviderNameLinkProps {
  name: string;
  baseUrl?: string | null;
  style?: React.CSSProperties;
  className?: string;
}

/**
 * Provider/channel title that opens the baseUrl origin in the system browser.
 * Keeps the original text color; hover underlines the name and shows that origin.
 */
const ProviderNameLink: React.FC<ProviderNameLinkProps> = ({
  name,
  baseUrl,
  style,
  className,
}) => {
  const { t } = useTranslation();
  const origin = getUrlOrigin(baseUrl);
  const combinedClassName = [className, origin ? styles.clickable : undefined]
    .filter(Boolean)
    .join(' ');

  if (!origin) {
    return (
      <span className={combinedClassName || undefined} style={style}>
        {name}
      </span>
    );
  }

  return (
    <Tooltip title={t('common.openUrl', { url: origin })}>
      <span
        className={combinedClassName}
        style={style}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          void openUrl(origin);
        }}
      >
        {name}
      </span>
    </Tooltip>
  );
};

export default ProviderNameLink;
