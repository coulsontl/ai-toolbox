import type React from 'react';
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
 * Keeps the original text color; hover only adds an underline.
 */
const ProviderNameLink: React.FC<ProviderNameLinkProps> = ({
  name,
  baseUrl,
  style,
  className,
}) => {
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
  );
};

export default ProviderNameLink;
