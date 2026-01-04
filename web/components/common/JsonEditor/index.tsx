import React, { useRef, useEffect, useState, useCallback } from 'react';
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
  /** Minimum height when resizable */
  minHeight?: number;
  /** Maximum height when resizable */
  maxHeight?: number;
  /** Enable resize handle */
  resizable?: boolean;
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
  minHeight = 150,
  maxHeight = 800,
  resizable = false,
  className,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<JSONEditorInstance | null>(null);
  const valueRef = useRef<unknown>(value);
  const externalValueRef = useRef<string>(JSON.stringify(value));
  
  // Convert initial height to number for resizable mode
  const initialHeight = typeof height === 'number' ? height : parseInt(height, 10) || 300;
  const [currentHeight, setCurrentHeight] = useState(initialHeight);
  
  // Resize handling
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

  // Initialize editor
  useEffect(() => {
    if (!containerRef.current) return;

    // Clear any existing content (handles React StrictMode double-mount)
    containerRef.current.innerHTML = '';

    const handleChange: OnChange = (content, _previousContent, { contentErrors }) => {
      if (!onChange) return;

      const isValid = !contentErrors;

      // Extract the actual value from content
      if ('json' in content && content.json !== undefined) {
        valueRef.current = content.json;
        onChange(content.json, isValid);
      } else if ('text' in content && content.text !== undefined) {
        // Treat empty or whitespace-only string as valid empty object
        const trimmedText = content.text.trim();
        if (trimmedText === '') {
          valueRef.current = {};
          onChange({}, true);
          return;
        }
        
        // Non-empty string must be valid JSON
        try {
          const parsed = JSON.parse(content.text);
          valueRef.current = parsed;
          onChange(parsed, true);
        } catch {
          // Invalid JSON - mark as invalid
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

    const newValueStr = JSON.stringify(value);
    
    // Skip if external value hasn't changed (to avoid resetting during editing)
    if (externalValueRef.current === newValueStr) {
      return;
    }

    // Also skip if the current internal value matches (user just typed this)
    if (JSON.stringify(valueRef.current) === newValueStr) {
      externalValueRef.current = newValueStr;
      return;
    }

    externalValueRef.current = newValueStr;
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

  const actualHeight = resizable ? currentHeight : (typeof height === 'number' ? height : parseInt(height, 10) || 300);

  return (
    <div style={{ position: 'relative', height: actualHeight }}>
      <div
        ref={containerRef}
        className={`json-editor-wrapper ${className || ''}`}
        style={{
          height: '100%',
          border: '1px solid #d9d9d9',
          borderRadius: 6,
          overflow: 'hidden',
        }}
      />
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

export default JsonEditor;
