import React, { useRef, useEffect } from 'react';
import {
  createJSONEditor,
  type JSONEditorPropsOptional,
  type Content,
  type OnChange,
} from 'vanilla-jsoneditor';
import './styles.css';

type EditorMode = 'tree' | 'text' | 'table';

export interface JsonEditorProps {
  /** JSON value - can be an object, array, or any JSON-compatible value */
  value: unknown;
  /** Callback when content changes */
  onChange?: (value: unknown, isValid: boolean) => void;
  /** Editor mode: 'tree', 'text', or 'table' */
  mode?: EditorMode;
  /** Read-only mode */
  readOnly?: boolean;
  /** Editor height */
  height?: number | string;
  /** Additional CSS class name */
  className?: string;
}

interface JSONEditorInstance {
  destroy: () => void;
  set: (content: Content) => void;
  get: () => Content;
  updateProps: (props: JSONEditorPropsOptional) => void;
}

const JsonEditor: React.FC<JsonEditorProps> = ({
  value,
  onChange,
  mode = 'text',
  readOnly = false,
  height = 300,
  className,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<JSONEditorInstance | null>(null);
  const valueRef = useRef<unknown>(value);

  // Initialize editor
  useEffect(() => {
    if (!containerRef.current) return;

    const handleChange: OnChange = (content, _previousContent, { contentErrors }) => {
      if (!onChange) return;

      const isValid = !contentErrors;

      // Extract the actual value from content
      if ('json' in content && content.json !== undefined) {
        valueRef.current = content.json;
        onChange(content.json, isValid);
      } else if ('text' in content && content.text !== undefined) {
        try {
          const parsed = JSON.parse(content.text);
          valueRef.current = parsed;
          onChange(parsed, true);
        } catch {
          onChange(content.text, false);
        }
      }
    };

    // Suppress error popups - just log to console
    const handleError = (err: Error) => {
      console.warn('JSON Editor error:', err);
    };

    const initialContent: Content =
      typeof value === 'string'
        ? { text: value }
        : { json: value };

    editorRef.current = createJSONEditor({
      target: containerRef.current,
      props: {
        content: initialContent,
        mode: mode as any,
        readOnly,
        onChange: handleChange,
        onError: handleError,
        mainMenuBar: true,
        navigationBar: false,
        statusBar: true,
        askToFormat: false,
      },
    }) as JSONEditorInstance;

    return () => {
      if (editorRef.current) {
        editorRef.current.destroy();
        editorRef.current = null;
      }
    };
    // Only run on mount/unmount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Update content when value prop changes (from outside)
  useEffect(() => {
    if (!editorRef.current) return;

    // Skip if value hasn't changed (to avoid infinite loops)
    if (JSON.stringify(valueRef.current) === JSON.stringify(value)) {
      return;
    }

    valueRef.current = value;

    const newContent: Content =
      typeof value === 'string'
        ? { text: value }
        : { json: value };

    editorRef.current.set(newContent);
  }, [value]);

  // Update props when mode or readOnly changes
  useEffect(() => {
    if (!editorRef.current) return;

    editorRef.current.updateProps({
      mode: mode as any,
      readOnly,
    });
  }, [mode, readOnly]);

  const heightStyle = typeof height === 'number' ? `${height}px` : height;

  return (
    <div
      ref={containerRef}
      className={className}
      style={{
        height: heightStyle,
        border: '1px solid #d9d9d9',
        borderRadius: 6,
        overflow: 'hidden',
      }}
    />
  );
};

export default JsonEditor;
