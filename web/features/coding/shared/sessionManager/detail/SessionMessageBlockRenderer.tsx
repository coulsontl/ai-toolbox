import React from 'react';
import { Brain, ChevronDown, FileText, Image, Info, Lock } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import MarkdownPreview from '@/components/common/MarkdownPreview';
import type { SessionMessage, SessionMessageBlock } from '../types';
import { hasSessionCommandTags } from './domain/commandTags';
import { getBlockText, valueToSearchText } from './domain/messageBlocks';
import { getVisibleMessageBlocks, type SessionContentFilter } from './domain/messageFilters';
import SessionCommandBlock from './SessionCommandBlock';
import SessionRendererCard from './SessionRendererCard';
import SessionSearchHighlight from './SessionSearchHighlight';
import SessionToolExecutionCard from './SessionToolExecutionCard';
import styles from './SessionDetailWorkbench.module.less';

interface SessionMessageBlockRendererProps {
  message: SessionMessage;
  query: string;
  contentFilter: SessionContentFilter;
}

const SessionMessageBlockRenderer: React.FC<SessionMessageBlockRendererProps> = ({ message, query, contentFilter }) => {
  const blocks = getVisibleMessageBlocks(message, contentFilter);

  return (
    <div className={styles.blockStack}>
      {blocks.map((block, index) => (
        <BlockRenderer
          key={`${block.kind}-${block.toolId ?? block.title ?? index}`}
          block={block}
          role={message.role}
          query={query}
        />
      ))}
    </div>
  );
};

interface BlockRendererProps {
  block: SessionMessageBlock;
  role: string;
  query: string;
}

const BlockRenderer: React.FC<BlockRendererProps> = ({ block, role, query }) => {
  const blockText = block.text || valueToSearchText(block.output);
  if (blockText && hasSessionCommandTags(blockText)) {
    return <SessionCommandBlock text={blockText} query={query} />;
  }

  if (block.kind === 'tool_call' || block.kind === 'tool_result' || block.kind === 'tool_execution') {
    return <SessionToolExecutionCard block={block} query={query} />;
  }

  if (block.kind === 'thinking') {
    return (
      <SessionRendererCard icon={Brain} title={block.title || 'Thinking'} variant="thinking">
        <TextBlock text={block.text || getBlockText(block)} role="assistant" query={query} surface="plain" />
      </SessionRendererCard>
    );
  }

  if (block.kind === 'redacted_thinking') {
    return (
      <SessionRendererCard icon={Lock} title={block.title || 'Redacted thinking'} variant="neutral">
        <div className={styles.resultMuted}>
          <SessionSearchHighlight text={block.text || 'Reasoning content is hidden.'} query={query} />
        </div>
      </SessionRendererCard>
    );
  }

  if (block.kind === 'summary') {
    return (
      <SessionRendererCard icon={FileText} title={block.title || 'Summary'} variant="document">
        <TextBlock text={block.text || ''} role="assistant" query={query} surface="plain" />
      </SessionRendererCard>
    );
  }

  if (block.kind === 'system') {
    return (
      <SessionRendererCard icon={Info} title={block.title || 'System'} variant="system">
        <TextBlock text={block.text || valueToSearchText(block.output)} role="system" query={query} surface="plain" />
      </SessionRendererCard>
    );
  }

  if (block.kind === 'image') {
    const source = block.text || valueToSearchText(block.output);
    return (
      <SessionRendererCard icon={Image} title={block.title || 'Image'} variant="document">
        {source ? <img className={styles.imagePreview} src={source} alt={block.title || 'Session image'} /> : null}
      </SessionRendererCard>
    );
  }

  if (block.kind === 'unknown') {
    return (
      <SessionRendererCard icon={Info} title={block.title || 'Unknown block'} variant="neutral">
        <pre className={styles.preBlock}>
          <SessionSearchHighlight text={block.text || valueToSearchText(block.metadata) || valueToSearchText(block.output)} query={query} />
        </pre>
      </SessionRendererCard>
    );
  }

  return <TextBlock text={block.text || valueToSearchText(block.output)} role={role} query={query} />;
};

interface TextBlockProps {
  text: string;
  role: string;
  query: string;
  surface?: 'bubble' | 'plain';
}

const TextBlock: React.FC<TextBlockProps> = ({ text, role, query, surface = 'bubble' }) => {
  if (!text) {
    return null;
  }

  const normalizedRole = role.toLowerCase();
  const shouldRenderMarkdown = normalizedRole === 'assistant';
  const content = query.trim() ? (
    <div className={styles.messageText}>
      <SessionSearchHighlight text={text} query={query} />
    </div>
  ) : shouldRenderMarkdown ? (
    <CollapsibleMarkdown content={text} />
  ) : (
    <div className={styles.messageText}>{text}</div>
  );

  if (surface === 'plain') {
    return content;
  }

  return (
    <div className={`${styles.textBubble} ${getTextBubbleClass(role)}`}>
      {content}
    </div>
  );
};

interface CollapsibleMarkdownProps {
  content: string;
}

const CollapsibleMarkdown: React.FC<CollapsibleMarkdownProps> = ({ content }) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = React.useState(false);
  const [overflowing, setOverflowing] = React.useState(false);
  const contentRef = React.useRef<HTMLDivElement | null>(null);

  const measureOverflow = React.useCallback(() => {
    const node = contentRef.current;
    if (!node) {
      return;
    }
    const lineHeight = Number.parseFloat(window.getComputedStyle(node).lineHeight) || 20;
    const collapsedHeight = lineHeight * 5;
    setOverflowing(node.scrollHeight > collapsedHeight + 1);
  }, []);

  React.useLayoutEffect(() => {
    setExpanded(false);
    const frameId = window.requestAnimationFrame(measureOverflow);
    return () => window.cancelAnimationFrame(frameId);
  }, [content, measureOverflow]);

  React.useEffect(() => {
    const node = contentRef.current;
    if (!node || typeof ResizeObserver === 'undefined') {
      return undefined;
    }
    const observer = new ResizeObserver(measureOverflow);
    observer.observe(node);
    return () => observer.disconnect();
  }, [measureOverflow]);

  return (
    <div className={styles.markdownCollapse}>
      <div
        ref={contentRef}
        className={[
          styles.markdownCollapseContent,
          expanded || !overflowing ? styles.markdownCollapseContentExpanded : '',
        ].filter(Boolean).join(' ')}
      >
        <MarkdownPreview content={content} className={styles.messageMarkdownPreview} />
      </div>
      {overflowing ? (
        <button
          type="button"
          className={styles.markdownCollapseToggle}
          aria-expanded={expanded}
          onClick={() => setExpanded((current) => !current)}
        >
          <ChevronDown
            size={12}
            aria-hidden="true"
            className={`${styles.markdownCollapseToggleIcon}${expanded ? ` ${styles.markdownCollapseToggleIconExpanded}` : ''}`}
          />
          {expanded ? t('sessionManager.collapseMarkdown') : t('sessionManager.expandMarkdown')}
        </button>
      ) : null}
    </div>
  );
};

function getTextBubbleClass(role: string): string {
  const normalizedRole = role.toLowerCase();
  if (normalizedRole === 'user') {
    return styles.textBubbleUser;
  }
  if (normalizedRole === 'assistant') {
    return styles.textBubbleAssistant;
  }
  if (normalizedRole === 'system') {
    return styles.textBubbleSystem;
  }
  return styles.textBubbleNeutral;
}

export default SessionMessageBlockRenderer;
