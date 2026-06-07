import React from 'react';
import {
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  Clock3,
  type LucideIcon,
} from 'lucide-react';

import type { SessionToolVariant } from './domain/toolCatalog';
import styles from './SessionDetailWorkbench.module.less';

interface SessionRendererCardProps {
  icon: LucideIcon;
  title: string;
  variant: SessionToolVariant;
  status?: string;
  meta?: React.ReactNode;
  children?: React.ReactNode;
}

const variantClassMap: Record<SessionToolVariant, string> = {
  terminal: styles.variantTerminal,
  code: styles.variantCode,
  file: styles.variantFile,
  search: styles.variantSearch,
  task: styles.variantTask,
  web: styles.variantWeb,
  mcp: styles.variantMcp,
  document: styles.variantDocument,
  system: styles.variantSystem,
  thinking: styles.variantThinking,
  success: styles.variantSuccess,
  warning: styles.variantWarning,
  error: styles.variantError,
  neutral: styles.variantNeutral,
};

const statusClassMap: Record<string, string> = {
  pending: styles.statusPending,
  running: styles.statusPending,
  success: styles.statusSuccess,
  completed: styles.statusSuccess,
  warning: styles.statusWarning,
  error: styles.statusError,
  failed: styles.statusError,
};

const SessionRendererCard: React.FC<SessionRendererCardProps> = ({
  icon: Icon,
  title,
  variant,
  status,
  meta,
  children,
}) => {
  const [expanded, setExpanded] = React.useState(false);
  const normalizedStatus = status?.toLowerCase();
  const statusClassName = normalizedStatus ? statusClassMap[normalizedStatus] : undefined;
  const StatusIcon = getStatusIcon(normalizedStatus);
  const hasBody = React.Children.count(children) > 0;

  return (
    <section className={`${styles.rendererCard} ${variantClassMap[variant]}`}>
      <button
        type="button"
        className={styles.rendererHeader}
        aria-expanded={hasBody ? expanded : undefined}
        disabled={!hasBody}
        onClick={() => {
          if (hasBody) {
            setExpanded((current) => !current);
          }
        }}
      >
        <span className={styles.rendererTitleGroup}>
          {hasBody ? (
            <ChevronRight
              size={14}
              aria-hidden="true"
              className={`${styles.rendererChevron}${expanded ? ` ${styles.rendererChevronOpen}` : ''}`}
            />
          ) : null}
          <Icon size={16} aria-hidden="true" className={styles.rendererIcon} />
          <span className={styles.rendererTitle}>{title}</span>
        </span>
        <span className={styles.rendererHeaderMeta}>
          {meta}
          {normalizedStatus ? (
            <span className={`${styles.statusBadge} ${statusClassName ?? styles.statusPending}`}>
              {StatusIcon ? <StatusIcon size={12} aria-hidden="true" /> : null}
              {normalizedStatus}
            </span>
          ) : null}
        </span>
      </button>
      {hasBody && expanded ? <div className={styles.rendererBody}>{children}</div> : null}
    </section>
  );
};

function getStatusIcon(status?: string): LucideIcon | null {
  if (!status) {
    return null;
  }
  if (status === 'success' || status === 'completed') {
    return CheckCircle2;
  }
  if (status === 'error' || status === 'failed') {
    return AlertTriangle;
  }
  return Clock3;
}

export default SessionRendererCard;
