import React from 'react';

import styles from './SessionDetailWorkbench.module.less';

interface SessionAnsiTextProps {
  text: string;
  className?: string;
}

interface AnsiState {
  bold: boolean;
  dim: boolean;
  color: string;
}

interface AnsiSegment {
  text: string;
  state: AnsiState;
}

const ANSI_PATTERN = /\u001b\[([0-9;]*)m/g;
const DEFAULT_STATE: AnsiState = { bold: false, dim: false, color: '' };

const colorClassMap: Record<string, string> = {
  black: styles.ansiBlack,
  red: styles.ansiRed,
  green: styles.ansiGreen,
  yellow: styles.ansiYellow,
  blue: styles.ansiBlue,
  magenta: styles.ansiMagenta,
  cyan: styles.ansiCyan,
  white: styles.ansiWhite,
};

const SessionAnsiText: React.FC<SessionAnsiTextProps> = ({ text, className }) => {
  const segments = React.useMemo(() => parseAnsiSegments(text), [text]);
  const firstSegment = segments[0];

  if (segments.length === 1 && firstSegment && !firstSegment.state.bold && !firstSegment.state.dim && !firstSegment.state.color) {
    return <span className={className}>{text}</span>;
  }

  return (
    <span className={className}>
      {segments.map((segment, index) => (
        <span key={index} className={stateToClassName(segment.state)}>
          {segment.text}
        </span>
      ))}
    </span>
  );
};

function parseAnsiSegments(text: string): AnsiSegment[] {
  const segments: AnsiSegment[] = [];
  let state = { ...DEFAULT_STATE };
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  ANSI_PATTERN.lastIndex = 0;
  while ((match = ANSI_PATTERN.exec(text)) !== null) {
    if (match.index > lastIndex) {
      segments.push({
        text: text.slice(lastIndex, match.index),
        state: { ...state },
      });
    }

    state = applyAnsiCodes(state, match[1]);
    lastIndex = ANSI_PATTERN.lastIndex;
  }

  if (lastIndex < text.length) {
    segments.push({
      text: text.slice(lastIndex),
      state: { ...state },
    });
  }

  return segments.length > 0 ? segments : [{ text, state: { ...DEFAULT_STATE } }];
}

function applyAnsiCodes(currentState: AnsiState, rawCodes: string): AnsiState {
  const codes = rawCodes
    ? rawCodes.split(';').map((code) => Number.parseInt(code, 10)).filter((code) => Number.isFinite(code))
    : [0];
  const nextState = { ...currentState };

  for (const code of codes) {
    if (code === 0) {
      nextState.bold = false;
      nextState.dim = false;
      nextState.color = '';
    } else if (code === 1) {
      nextState.bold = true;
    } else if (code === 2) {
      nextState.dim = true;
    } else if (code === 22) {
      nextState.bold = false;
      nextState.dim = false;
    } else if (code === 39) {
      nextState.color = '';
    } else {
      const color = ansiColorName(code);
      if (color) {
        nextState.color = color;
      }
    }
  }

  return nextState;
}

function ansiColorName(code: number): string {
  const normalizedCode = code >= 90 && code <= 97 ? code - 60 : code;
  switch (normalizedCode) {
    case 30:
      return 'black';
    case 31:
      return 'red';
    case 32:
      return 'green';
    case 33:
      return 'yellow';
    case 34:
      return 'blue';
    case 35:
      return 'magenta';
    case 36:
      return 'cyan';
    case 37:
      return 'white';
    default:
      return '';
  }
}

function stateToClassName(state: AnsiState): string {
  return [
    state.bold ? styles.ansiBold : '',
    state.dim ? styles.ansiDim : '',
    state.color ? colorClassMap[state.color] : '',
  ].filter(Boolean).join(' ');
}

export default SessionAnsiText;
