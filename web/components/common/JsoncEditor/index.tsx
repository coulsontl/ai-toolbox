import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { editor } from 'monaco-editor';
import * as monaco from 'monaco-editor';
import JSON5 from 'json5';
import MonacoEditor from 'react-monaco-editor';

import { useThemeStore } from '@/stores/themeStore';

export interface JsoncEditorProps {
  value: string;
  onChange?: (value: string, isValid: boolean, parsed: unknown | null) => void;
  onBlur?: (value: string, isValid: boolean, parsed: unknown | null) => void;
  readOnly?: boolean;
  height?: number | string;
  minHeight?: number;
  maxHeight?: number;
  resizable?: boolean;
  className?: string;
  placeholder?: string;
}

const parseJsonc = (content: string): { isValid: boolean; parsed: unknown | null; error?: string } => {
  const trimmedContent = content.trim();
  if (!trimmedContent) {
    return { isValid: true, parsed: null };
  }

  try {
    return { isValid: true, parsed: JSON5.parse(trimmedContent) };
  } catch (error) {
    return {
      isValid: false,
      parsed: null,
      error: error instanceof Error ? error.message : String(error),
    };
  }
};

const JsoncEditor: React.FC<JsoncEditorProps> = ({
  value,
  onChange,
  onBlur,
  readOnly = false,
  height = 320,
  minHeight = 180,
  maxHeight = 900,
  resizable = true,
  className,
  placeholder,
}) => {
  const { resolvedTheme } = useThemeStore();
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const validateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const editorContentRef = useRef<string>(value);
  const lastExternalValueRef = useRef<string>(value);
  const isUserEditingRef = useRef(false);
  const [editorContent, setEditorContent] = useState(value);
  const [isUserEditing, setIsUserEditing] = useState(false);
  const initialHeight = typeof height === 'number' ? height : parseInt(height, 10) || 320;
  const [currentHeight, setCurrentHeight] = useState(initialHeight);
  const isResizingRef = useRef(false);
  const startYRef = useRef(0);
  const startHeightRef = useRef(0);

  const validateAndSetMarkers = useCallback((content: string) => {
    const editorModel = editorRef.current?.getModel();
    if (!editorModel) return parseJsonc(content);

    const result = parseJsonc(content);
    if (result.isValid) {
      monaco.editor.setModelMarkers(editorModel, 'jsonc', []);
      return result;
    }

    monaco.editor.setModelMarkers(editorModel, 'jsonc', [
      {
        severity: monaco.MarkerSeverity.Error,
        startLineNumber: 1,
        startColumn: 1,
        endLineNumber: Math.max(1, editorModel.getLineCount()),
        endColumn: editorModel.getLineMaxColumn(Math.max(1, editorModel.getLineCount())),
        message: result.error || 'Invalid JSONC',
      },
    ]);
    return result;
  }, []);

  const handleEditorDidMount = useCallback((editorInstance: editor.IStandaloneCodeEditor) => {
    editorRef.current = editorInstance;
    editorContentRef.current = value;
    lastExternalValueRef.current = value;
    setEditorContent(value);
    validateAndSetMarkers(value);

    editorInstance.onDidFocusEditorText(() => {
      isUserEditingRef.current = true;
      setIsUserEditing(true);
      editorInstance.updateOptions({ renderLineHighlight: 'line' });
    });

    editorInstance.onDidBlurEditorText(() => {
      isUserEditingRef.current = false;
      setIsUserEditing(false);
      editorInstance.updateOptions({ renderLineHighlight: 'none' });
      const currentContent = editorInstance.getValue();
      const result = validateAndSetMarkers(currentContent);
      onBlur?.(currentContent, result.isValid, result.parsed);
    });
  }, [onBlur, validateAndSetMarkers, value]);

  const handleChange = useCallback((nextValue: string) => {
    editorContentRef.current = nextValue;
    setEditorContent(nextValue);
    const immediateResult = parseJsonc(nextValue);
    onChange?.(nextValue, immediateResult.isValid, immediateResult.parsed);

    if (validateTimeoutRef.current) {
      clearTimeout(validateTimeoutRef.current);
    }

    validateTimeoutRef.current = setTimeout(() => {
      validateAndSetMarkers(nextValue);
    }, 250);
  }, [onChange, validateAndSetMarkers]);

  useEffect(() => {
    if (lastExternalValueRef.current === value) return;
    if (isUserEditing) return;

    lastExternalValueRef.current = value;
    editorContentRef.current = value;
    setEditorContent(value);
    editorRef.current?.getModel()?.setValue(value);
    validateAndSetMarkers(value);
  }, [isUserEditing, validateAndSetMarkers, value]);

  useEffect(() => () => {
    if (validateTimeoutRef.current) {
      clearTimeout(validateTimeoutRef.current);
    }
  }, []);

  const handleMouseDown = useCallback((event: React.MouseEvent) => {
    event.preventDefault();
    isResizingRef.current = true;
    startYRef.current = event.clientY;
    startHeightRef.current = currentHeight;
    document.body.style.cursor = 'ns-resize';
    document.body.style.userSelect = 'none';
  }, [currentHeight]);

  useEffect(() => {
    if (!resizable) return undefined;

    const handleMouseMove = (event: MouseEvent) => {
      if (!isResizingRef.current) return;
      const nextHeight = Math.min(
        maxHeight,
        Math.max(minHeight, startHeightRef.current + event.clientY - startYRef.current),
      );
      setCurrentHeight(nextHeight);
    };

    const handleMouseUp = () => {
      if (!isResizingRef.current) return;
      isResizingRef.current = false;
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [maxHeight, minHeight, resizable]);

  const actualHeight = resizable ? currentHeight : height;
  const monacoTheme = resolvedTheme === 'dark' ? 'vs-dark' : 'vs';
  const borderColor = resolvedTheme === 'dark' ? 'var(--color-border-secondary)' : '#d9d9d9';
  const placeholderColor = resolvedTheme === 'dark' ? 'rgba(255, 255, 255, 0.45)' : '#999';
  const showPlaceholder = placeholder && editorContent.trim() === '';

  const options: editor.IStandaloneEditorConstructionOptions = {
    readOnly,
    minimap: { enabled: false },
    lineNumbers: 'on',
    lineNumbersMinChars: 3,
    lineDecorationsWidth: 8,
    scrollBeyondLastLine: false,
    wordWrap: 'on',
    automaticLayout: true,
    fontSize: 13,
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
    formatOnPaste: false,
    formatOnType: false,
  };

  return (
    <div style={{ position: 'relative', height: actualHeight }}>
      <div
        className={className}
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
          language="json"
          theme={monacoTheme}
          value={editorContent}
          options={options}
          onChange={handleChange}
          editorDidMount={handleEditorDidMount}
        />
        {showPlaceholder && (
          <div
            style={{
              position: 'absolute',
              top: 9,
              left: 44,
              color: placeholderColor,
              fontSize: 13,
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
            right: 0,
            bottom: 0,
            width: 16,
            height: 16,
            cursor: 'ns-resize',
            opacity: 0.5,
          }}
        />
      )}
    </div>
  );
};

export default JsoncEditor;
