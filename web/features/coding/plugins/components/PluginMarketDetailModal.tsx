import React from 'react';
import { Modal } from 'antd';
import { Check, Download, ExternalLink, Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { openExternalUrl } from '@/services';
import type { MarketPluginListing } from '@/services';
import {
  getPluginKindCategory,
  getPluginSummary,
} from '../utils/pluginFormatters';
import PluginIcon from './PluginIcon';
import styles from './PluginMarketDetailModal.module.less';

interface PluginMarketDetailModalProps {
  open: boolean;
  listing: MarketPluginListing | null;
  marketName: string;
  installing: boolean;
  onInstall: () => void;
  onClose: () => void;
}

const PluginMarketDetailModal: React.FC<PluginMarketDetailModalProps> = ({
  open,
  listing,
  marketName,
  installing,
  onInstall,
  onClose,
}) => {
  const { t, i18n } = useTranslation();

  if (!listing) {
    return null;
  }

  const category = getPluginKindCategory(listing.kind);
  const kindLabel = category === 'middleware'
    ? t('plugins.kind.middleware')
    : category === 'both'
      ? t('plugins.kind.both')
      : category === 'page'
        ? t('plugins.kind.page')
        : category === 'unknown'
          ? t('plugins.kind.unknown')
          : t('plugins.kind.provider');
  const sourceLabel = marketName
    || (listing.community ? t('plugins.market.communitySource') : t('plugins.market.detail.unknownSource'));
  const summary = getPluginSummary(listing.summary, i18n.language)
    || listing.description
    || listing.package;
  const description = listing.description || summary;
  const installedLabel = listing.installedVersion
    ? t('plugins.market.installedVersion', { version: listing.installedVersion })
    : t('plugins.market.installed');

  return (
    <Modal
      open={open}
      width={560}
      footer={null}
      onCancel={onClose}
      rootClassName={styles.modalRoot}
      aria-label={t('plugins.market.detail.title')}
    >
      <div className={styles.content}>
        <header className={styles.header}>
          <span className={styles.headerIcon}>
            <PluginIcon icon={listing.icon} name={listing.name || listing.package} size={44} />
          </span>
          <div className={styles.headerText}>
            <h3 className={styles.title}>{listing.name || listing.package}</h3>
            <div className={styles.subtitle}>
              <span>{sourceLabel}</span>
              <span aria-hidden="true">·</span>
              <span className={styles.packageName} title={listing.package}>
                {listing.package}
              </span>
            </div>
          </div>
          <div className={styles.headerAction}>
            {listing.installed ? (
              <span className={styles.installedLabel}>
                <Check size={13} aria-hidden="true" />
                {installedLabel}
              </span>
            ) : (
              <button
                type="button"
                className={styles.installPill}
                disabled={installing}
                onClick={onInstall}
              >
                {installing ? (
                  <Loader2 size={13} className={styles.spin} aria-hidden="true" />
                ) : (
                  <Download size={13} aria-hidden="true" />
                )}
                <span>
                  {installing ? t('plugins.market.installing') : t('plugins.market.install')}
                </span>
              </button>
            )}
          </div>
        </header>

        <p className={styles.summary}>{summary}</p>

        <div className={styles.attributes}>
          <div className={styles.attribute}>
            <span className={styles.attributeLabel}>
              {t('plugins.market.detail.version')}
            </span>
            <span className={styles.attributeValue}>
              {listing.latestVersion ? `v${listing.latestVersion}` : '—'}
            </span>
          </div>
          <div className={styles.attribute}>
            <span className={styles.attributeLabel}>
              {t('plugins.market.detail.type')}
            </span>
            <span className={styles.attributeValue}>{kindLabel}</span>
          </div>
          <div className={styles.attribute}>
            <span className={styles.attributeLabel}>
              {t('plugins.market.detail.source')}
            </span>
            <span className={styles.attributeValue} title={sourceLabel}>
              {sourceLabel}
            </span>
          </div>
          <div className={styles.attribute}>
            <span className={styles.attributeLabel}>
              {t('plugins.market.detail.provides')}
            </span>
            <span className={styles.attributeValue}>
              {listing.providers.length > 0
                ? t('plugins.market.detail.channelCount', {
                  count: listing.providers.length,
                })
                : category === 'page'
                  ? t('plugins.market.detail.pagePlugin')
                  : '—'}
            </span>
          </div>
        </div>

        {listing.package ? (
          <button
            type="button"
            className={styles.npmLink}
            onClick={() => void openExternalUrl(`https://www.npmjs.com/package/${listing.package}`)}
          >
            <ExternalLink size={13} aria-hidden="true" />
            <span>{t('plugins.market.detail.npmLink')}</span>
          </button>
        ) : null}

        <section className={styles.section}>
          <h4 className={styles.sectionTitle}>
            {t('plugins.market.detail.descriptionTitle')}
          </h4>
          <p className={styles.description}>{description}</p>
          {listing.providers.length > 0 ? (
            <div className={styles.providers}>
              <span className={styles.providersLabel}>
                {t('plugins.market.detail.providersTitle')}
              </span>
              <div className={styles.providerList}>
                {listing.providers.map((provider) => (
                  <span key={provider} className={styles.providerTag}>
                    {provider}
                  </span>
                ))}
              </div>
            </div>
          ) : null}
          <div className={styles.installCommand}>
            <span className={styles.installCommandLabel}>
              {t('plugins.market.detail.installCommand')}
            </span>
            <code className={styles.installCommandCode}>npm install {listing.package}</code>
          </div>
        </section>

        <footer className={styles.footer}>
          <span className={styles.safety}>
            {t('plugins.market.detail.safety')}
          </span>
          <button type="button" className={styles.closeButton} onClick={onClose}>
            {t('plugins.market.detail.close')}
          </button>
        </footer>
      </div>
    </Modal>
  );
};

export default PluginMarketDetailModal;
