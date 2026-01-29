import React, { useRef, useCallback, useEffect, useState } from 'react';
import MonacoEditor from 'react-monaco-editor';
import type { editor } from 'monaco-editor';
import * as monaco from 'monaco-editor';
import { parse as parseToml } from 'smol-toml';
import { useThemeStore } from '@/stores/themeStore';

export interface TomlEditorProps {
  /** TOML 内容值 */
  value: string;
  /** 内容变化回调 */
  onChange?: (value: string) => void;
  /** 编辑器失去焦点回调 */
  onBlur?: (value: string) => void;
  /** 编辑器高度 */
  height?: number | string;
  /** 是否只读 */
  readOnly?: boolean;
  /** 主题: 'vs' | 'vs-dark' | 'hc-black' */
  theme?: string;
  /** 占位符提示文本 */
  placeholder?: string;
  /** 最小高度（可调整大小时） */
  minHeight?: number;
  /** 最大高度（可调整大小时） */
  maxHeight?: number;
  /** 是否可调整大小 */
  resizable?: boolean;
}

// 注册 TOML 语言（只注册一次）
// 完全按照 https://github.com/microsoft/monaco-editor/pull/4786 实现
let tomlRegistered = false;
const registerTomlLanguage = () => {
  if (tomlRegistered) return;
  tomlRegistered = true;

  // 检查语言是否已注册
  const languages = monaco.languages.getLanguages();
  if (languages.some(lang => lang.id === 'toml')) {
    return;
  }

  // 注册语言
  monaco.languages.register({
    id: 'toml',
    extensions: ['.toml'],
    aliases: ['TOML', 'toml'],
    mimetypes: ['text/x-toml'],
  });

  // 语言配置
  monaco.languages.setLanguageConfiguration('toml', {
    comments: {
      lineComment: '#',
    },
    brackets: [
      ['{', '}'],
      ['[', ']'],
      ['(', ')'],
    ],
    autoClosingPairs: [
      { open: '{', close: '}' },
      { open: '[', close: ']' },
      { open: '(', close: ')' },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
    folding: {
      offSide: true,
    },
    onEnterRules: [
      {
        beforeText: /[\{\[]\s*$/,
        action: {
          indentAction: monaco.languages.IndentAction.Indent,
        },
      },
    ],
  });

  // 创建单行字面字符串状态
  const createSingleLineLiteralStringState = (tokenClass: string): monaco.languages.IMonarchLanguageRule[] => [
    [/[^']+/, tokenClass],
    [/'/, tokenClass, '@pop'],
  ];

  // 创建单行字符串状态
  const createSingleLineStringState = (tokenClass: string): monaco.languages.IMonarchLanguageRule[] => [
    [/[^"\\]+/, tokenClass],
    [/@escapes/, 'constant.character.escape'],
    [/\\./, 'constant.character.escape.invalid'],
    [/"/, tokenClass, '@pop'],
  ];

  // 创建标识符链状态
  const createIdentChainStates = (tokenClass: string) => {
    const singleQuotedState = `identChain.${tokenClass}.singleQuoted`;
    const singleQuoteClass = `${tokenClass}.string.literal`;
    const doubleQuotedState = `identChain.${tokenClass}.doubleQuoted`;
    const doubleQuoteClass = `${tokenClass}.string`;
    return {
      [`identChain.${tokenClass}`]: [
        { include: '@whitespace' },
        { include: '@comment' },
        [/@identifier/, tokenClass],
        [/\./, 'delimiter'],
        [/'[^']*$/, `${tokenClass}.invalid`],
        [/'/, { token: singleQuoteClass, next: `@${singleQuotedState}` }],
        [/"(\\.|[^"])*$/, `${tokenClass}.invalid`],
        [/"/, { token: doubleQuoteClass, next: `@${doubleQuotedState}` }],
        [/./, '@rematch', '@pop'],
      ],
      [singleQuotedState]: createSingleLineLiteralStringState(singleQuoteClass),
      [doubleQuotedState]: createSingleLineStringState(doubleQuoteClass),
    };
  };

  // 设置语法高亮规则 (Monarch tokenizer)
  // 完全按照 PR #4786 实现
  monaco.languages.setMonarchTokensProvider('toml', {
    tokenPostfix: '.toml',
    brackets: [
      { token: 'delimiter.bracket', open: '{', close: '}' },
      { token: 'delimiter.square', open: '[', close: ']' },
    ],

    // 数字正则
    numberInteger: /[+-]?(0|[1-9](_?[0-9])*)/,
    numberOctal: /0o[0-7](_?[0-7])*/,
    numberHex: /0x[0-9a-fA-F](_?[0-9a-fA-F])*/,
    numberBinary: /0b[01](_?[01])*/,

    floatFractionPart: /\.[0-9](_?[0-9])*/,
    floatExponentPart: /[eE][+-]?[0-9](_?[0-9])*/,

    // RFC 3339 日期时间
    date: /\d{4}-\d\d-\d\d/,
    time: /\d\d:\d\d:\d\d(\.\d+)?/,
    offset: /[+-]\d\d:\d\d/,

    // 转义序列
    escapes: /\\([btnfr"\\]|u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8})/,
    identifier: /([\w-]+)/,
    identChainStart: /([\w-"'])/,
    valueStart: /(["'tf0-9+\-in\[\{])/,

    tokenizer: {
      root: [
        { include: '@comment' },
        { include: '@whitespace' },
        // 键值对
        [/@identChainStart/, '@rematch', '@kvpair'],
        // 表头
        [/\[/, '@brackets', '@table'],
        // 无效的值（没有键）
        [/=/, 'delimiter', '@value'],
      ],

      comment: [[/#.*$/, 'comment']],
      whitespace: [[/[ \t\r\n]+/, 'white']],

      // 键值对解析
      kvpair: [
        { include: '@whitespace' },
        { include: '@comment' },
        [/@identChainStart/, '@rematch', '@identChain.variable'],
        [/=/, { token: 'delimiter', switchTo: '@value' }],
        [/./, '@rematch', '@pop'],
      ],

      // 变量标识符链
      ...createIdentChainStates('variable'),

      // 表头解析
      table: [
        { include: '@whitespace' },
        { include: '@comment' },
        [/\[/, '@brackets', '@table'],
        [/@identChainStart/, '@rematch', '@identChain.type'],
        [/\]/, '@brackets', '@pop'],
      ],

      // 类型标识符链
      ...createIdentChainStates('type'),

      // 值解析
      value: [
        { include: '@whitespace' },
        { include: '@comment' },
        { include: '@value.cases' },
        [/./, '@rematch', '@pop'],
      ],

      'value.string.singleQuoted': createSingleLineLiteralStringState('string.literal'),
      'value.string.doubleQuoted': createSingleLineStringState('string'),

      'value.string.multi.doubleQuoted': [
        [/[^"\\]+/, 'string.multi'],
        [/@escapes/, 'constant.character.escape'],
        [/\\$/, 'constant.character.escape'],
        [/\\./, 'constant.character.escape.invalid'],
        [/"""(""|")?/, 'string.multi', '@pop'],
        [/"/, 'string.multi'],
      ],

      'value.string.multi.singleQuoted': [
        [/[^']+/, 'string.literal.multi'],
        [/'''(''|')?/, 'string.literal.multi', '@pop'],
        [/'/, 'string.literal.multi'],
      ],

      // 数组
      'value.array': [
        { include: '@whitespace' },
        { include: '@comment' },
        [/\]/, '@brackets', '@pop'],
        [/,/, 'delimiter'],
        [/@valueStart/, '@rematch', '@value.array.entry'],
        [/.+(?=[,\]])/, 'source'],
      ],

      'value.array.entry': [
        { include: '@whitespace' },
        { include: '@comment' },
        { include: '@value.cases' },
        [/.+(?=[,\]])/, 'source', '@pop'],
        [/./, 'source', '@pop'],
      ],

      // 内联表
      'value.inlinetable': [
        { include: '@whitespace' },
        { include: '@comment' },
        [/\}/, '@brackets', '@pop'],
        [/,/, 'delimiter'],
        [/@identChainStart/, '@rematch', '@value.inlinetable.entry'],
        [/=/, 'delimiter', '@value.inlinetable.value'],
        [/@valueStart/, '@rematch', '@value.inlinetable.value'],
        [/.+(?=[,\}])/, 'source', '@pop'],
      ],

      'value.inlinetable.entry': [
        { include: '@whitespace' },
        { include: '@comment' },
        [/@identChainStart/, '@rematch', '@identChain.variable'],
        [/=/, { token: 'delimiter', switchTo: '@value.inlinetable.value' }],
        [/.+(?=[,\}])/, 'source', '@pop'],
      ],

      'value.inlinetable.value': [
        { include: '@whitespace' },
        { include: '@comment' },
        { include: '@value.cases' },
        [/.+(?=[,\}])/, 'source', '@pop'],
        [/./, 'source', '@pop'],
      ],

      'value.cases': [
        // 多行双引号字符串
        [/"""/, { token: 'string.multi', switchTo: '@value.string.multi.doubleQuoted' }],
        [/"(\\.|[^"])*$/, 'string.invalid'],
        [/"/, { token: 'string', switchTo: '@value.string.doubleQuoted' }],

        // 多行单引号字符串
        [/'''/, { token: 'string.literal.multi', switchTo: '@value.string.multi.singleQuoted' }],
        [/'[^']*$/, 'string.literal.invalid'],
        [/'/, { token: 'string.literal', switchTo: '@value.string.singleQuoted' }],

        // 布尔值
        [/(true|false)/, 'constant.language.boolean', '@pop'],

        // 数组
        [/\[/, { token: '@brackets', switchTo: '@value.array' }],

        // 内联表
        [/\{/, { token: '@brackets', switchTo: '@value.inlinetable' }],

        // 整数
        [/@numberInteger(?![0-9_oxbeE\.:-])/, 'number', '@pop'],

        // 浮点数
        [/@numberInteger(@floatFractionPart@floatExponentPart?|@floatExponentPart)/, 'number.float', '@pop'],

        // 其他数字类型
        [/@numberOctal/, 'number.octal', '@pop'],
        [/@numberHex/, 'number.hex', '@pop'],
        [/@numberBinary/, 'number.binary', '@pop'],

        // 特殊浮点值
        [/[+-]?inf/, 'number.inf', '@pop'],
        [/[+-]?nan/, 'number.nan', '@pop'],

        // 日期时间
        [/@date[Tt ]@time(@offset|Z)?/, 'number.datetime', '@pop'],
        [/@date/, 'number.date', '@pop'],
        [/@time/, 'number.time', '@pop'],
      ],
    },
  });
};

/**
 * 验证 TOML 内容并返回错误信息
 */
const validateToml = (content: string): { line: number; column: number; message: string } | null => {
  if (!content.trim()) {
    return null;
  }

  try {
    parseToml(content);
    return null;
  } catch (err: unknown) {
    if (err instanceof Error) {
      const message = err.message;
      const lineMatch = message.match(/line\s+(\d+)/i);
      const colMatch = message.match(/col(?:umn)?\s+(\d+)/i);

      const line = lineMatch ? parseInt(lineMatch[1], 10) : 1;
      const column = colMatch ? parseInt(colMatch[1], 10) : 1;

      return { line, column, message };
    }
    return { line: 1, column: 1, message: String(err) };
  }
};

/**
 * 基于 Monaco Editor 的 TOML 编辑器组件
 */
const TomlEditor: React.FC<TomlEditorProps> = ({
  value,
  onChange,
  onBlur,
  height = 300,
  readOnly = false,
  theme,
  placeholder,
  minHeight = 150,
  maxHeight = 800,
  resizable = true,
}) => {
  const { resolvedTheme } = useThemeStore();
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const validateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Monaco theme based on app theme (or explicit theme prop)
  const monacoTheme = theme || (resolvedTheme === 'dark' ? 'vs-dark' : 'vs');
  const borderColor = resolvedTheme === 'dark' ? 'var(--color-border-secondary)' : '#d9d9d9';
  const placeholderColor = resolvedTheme === 'dark' ? 'rgba(255, 255, 255, 0.45)' : '#999';

  // 可调整大小的高度状态
  const initialHeight = typeof height === 'number' ? height : parseInt(height, 10) || 300;
  const [currentHeight, setCurrentHeight] = useState(initialHeight);

  // 调整大小相关
  const isResizingRef = useRef(false);
  const startYRef = useRef(0);
  const startHeightRef = useRef(0);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isResizingRef.current = true;
    startYRef.current = e.clientY;
    startHeightRef.current = currentHeight;
    document.body.style.cursor = 'ns-resize';
    document.body.style.userSelect = 'none';
  }, [currentHeight]);

  useEffect(() => {
    if (!resizable) return;

    const handleMouseMove = (e: MouseEvent) => {
      if (!isResizingRef.current) return;
      const deltaY = e.clientY - startYRef.current;
      const newHeight = Math.min(maxHeight, Math.max(minHeight, startHeightRef.current + deltaY));
      setCurrentHeight(newHeight);
    };

    const handleMouseUp = () => {
      if (isResizingRef.current) {
        isResizingRef.current = false;
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
      }
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [resizable, minHeight, maxHeight]);

  // 在编辑器挂载前注册 TOML 语言
  const handleEditorWillMount = useCallback(() => {
    registerTomlLanguage();
  }, []);

  // 验证 TOML 内容并设置错误标记
  const validateAndSetMarkers = useCallback((content: string) => {
    if (!editorRef.current) return;

    const model = editorRef.current.getModel();
    if (!model) return;

    const error = validateToml(content);

    if (error) {
      monaco.editor.setModelMarkers(model, 'toml', [
        {
          severity: monaco.MarkerSeverity.Error,
          startLineNumber: error.line,
          startColumn: error.column,
          endLineNumber: error.line,
          endColumn: model.getLineMaxColumn(error.line),
          message: error.message,
        },
      ]);
    } else {
      monaco.editor.setModelMarkers(model, 'toml', []);
    }
  }, []);

  const handleEditorDidMount = useCallback((
    editorInstance: editor.IStandaloneCodeEditor,
  ) => {
    editorRef.current = editorInstance;
    validateAndSetMarkers(value);

    // 监听焦点事件，动态切换行高亮
    editorInstance.onDidFocusEditorText(() => {
      editorInstance.updateOptions({ renderLineHighlight: 'line' });
    });
    editorInstance.onDidBlurEditorText(() => {
      editorInstance.updateOptions({ renderLineHighlight: 'none' });
      // 失去焦点时触发 onBlur 回调
      if (onBlur) {
        onBlur(editorInstance.getValue());
      }
    });
  }, [value, validateAndSetMarkers, onBlur]);

  const handleChange = useCallback((newValue: string) => {
    onChange?.(newValue);

    if (validateTimeoutRef.current) {
      clearTimeout(validateTimeoutRef.current);
    }
    validateTimeoutRef.current = setTimeout(() => {
      validateAndSetMarkers(newValue);
    }, 300);
  }, [onChange, validateAndSetMarkers]);

  useEffect(() => {
    return () => {
      if (validateTimeoutRef.current) {
        clearTimeout(validateTimeoutRef.current);
      }
    };
  }, []);

  // 编辑器配置常量
  const FONT_SIZE = 13;
  const LINE_NUMBERS_MIN_CHARS = 3;
  const LINE_DECORATIONS_WIDTH = 8;
  // placeholder 左边距 = 行号区域宽度 + 装饰宽度 + 内边距
  const PLACEHOLDER_LEFT = LINE_NUMBERS_MIN_CHARS * (FONT_SIZE * 0.6) + LINE_DECORATIONS_WIDTH + 12;

  const options: editor.IStandaloneEditorConstructionOptions = {
    readOnly,
    minimap: { enabled: false },
    lineNumbers: 'on',
    lineNumbersMinChars: LINE_NUMBERS_MIN_CHARS,
    scrollBeyondLastLine: false,
    wordWrap: 'on',
    automaticLayout: true,
    fontSize: FONT_SIZE,
    tabSize: 2,
    renderLineHighlight: 'none',
    scrollbar: {
      vertical: 'auto',
      horizontal: 'auto',
      verticalScrollbarSize: 8,
      horizontalScrollbarSize: 8,
    },
    padding: { top: 8, bottom: 8 },
    folding: true,
    lineDecorationsWidth: LINE_DECORATIONS_WIDTH,
  };

  const actualHeight = resizable ? currentHeight : (typeof height === 'number' ? height : parseInt(height, 10) || 300);

  // 判断是否显示 placeholder
  const showPlaceholder = placeholder && value.trim() === '';

  return (
    <div style={{ position: 'relative', height: actualHeight }}>
      <div
        style={{
          height: '100%',
          border: `1px solid ${borderColor}`,
          borderRadius: 6,
          overflow: 'hidden',
        }}
      >
        <MonacoEditor
          width="100%"
          height={actualHeight}
          language="toml"
          theme={monacoTheme}
          value={value}
          options={options}
          onChange={handleChange}
          editorWillMount={handleEditorWillMount}
          editorDidMount={handleEditorDidMount}
        />
        {showPlaceholder && (
          <div
            style={{
              position: 'absolute',
              top: 9,
              left: PLACEHOLDER_LEFT,
              color: placeholderColor,
              fontSize: FONT_SIZE,
              pointerEvents: 'none',
              userSelect: 'none',
              whiteSpace: 'pre',
              fontFamily: 'Menlo, Monaco, "Courier New", monospace',
            }}
          >
            {placeholder}
          </div>
        )}
      </div>
      {resizable && (
        <div
          onMouseDown={handleMouseDown}
          style={{
            position: 'absolute',
            bottom: 0,
            right: 0,
            width: 16,
            height: 16,
            cursor: 'ns-resize',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            opacity: 0.5,
            transition: 'opacity 0.2s',
          }}
          onMouseEnter={(e) => { e.currentTarget.style.opacity = '1'; }}
          onMouseLeave={(e) => { e.currentTarget.style.opacity = '0.5'; }}
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
            <path d="M8 2L2 8M8 5L5 8M8 8L8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </div>
      )}
    </div>
  );
};

export default TomlEditor;
