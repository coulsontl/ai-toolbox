import React from 'react';
import PluginIcon, { hasPluginIconVisual, isPluginRemoteIcon } from './PluginIcon';
import styles from './PluginPageGlyph.module.less';

interface PluginPageGlyphProps {
  iconDataUrl?: string | null;
  pluginIcon?: string | null;
  pluginName: string;
  title: string;
  size?: number;
  className?: string;
}

/** The first visible character of a page title, for the icon-less fallback. */
const getTitleInitial = (title: string): string => {
  const first = Array.from(title.trim())[0] ?? '?';
  return first === first.toLowerCase() ? first.toUpperCase() : first;
};

/**
 * A page's icon: its packaged data URL, a brand mark, or a compact rounded
 * square holding the title's first character. The fallback has to stay legible
 * at 22px in the collapsed sidebar, so it never renders a two-letter monogram.
 */
const PluginPageGlyph: React.FC<PluginPageGlyphProps> = ({
  iconDataUrl,
  pluginIcon,
  pluginName,
  title,
  size = 24,
  className,
}) => {
  const [imageFailed, setImageFailed] = React.useState(false);
  const remoteIcon = isPluginRemoteIcon(pluginIcon ?? '');
  const imageSource = iconDataUrl || (remoteIcon ? pluginIcon ?? '' : '');

  React.useEffect(() => {
    setImageFailed(false);
  }, [imageSource]);

  if (imageSource && !imageFailed) {
    return (
      <span
        className={`${styles.glyph} ${className ?? ''}`}
        style={{ width: size, height: size }}
        aria-hidden="true"
      >
        <img
          src={imageSource}
          alt=""
          className={styles.image}
          onError={() => setImageFailed(true)}
        />
      </span>
    );
  }

  if (!remoteIcon && hasPluginIconVisual(pluginIcon, pluginName)) {
    return <PluginIcon icon={pluginIcon ?? ''} name={pluginName} size={size} className={className} />;
  }

  return (
    <span
      className={`${styles.glyph} ${className ?? ''}`}
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <span className={styles.initial}>{getTitleInitial(title)}</span>
    </span>
  );
};

export default PluginPageGlyph;
