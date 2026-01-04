import React from 'react';
import JsonEditor from '@/components/common/JsonEditor';

interface HeadersEditorProps {
  /** Value can be JSON string or object */
  value?: string | Record<string, string>;
  onChange?: (value: string | Record<string, string> | undefined) => void;
  /** Callback when validation state changes */
  onValidationChange?: (isValid: boolean) => void;
  /** Output format: 'string' outputs JSON string, 'object' outputs plain object */
  outputFormat?: 'string' | 'object';
  height?: number;
}

/**
 * A reusable JSON editor component for HTTP headers
 */
const HeadersEditor: React.FC<HeadersEditorProps> = ({
  value,
  onChange,
  onValidationChange,
  outputFormat = 'string',
  height = 200,
}) => {
  const jsonValue = React.useMemo(() => {
    if (!value) return {};
    if (typeof value === 'string') {
      try {
        return JSON.parse(value);
      } catch {
        return {};
      }
    }
    return value;
  }, [value]);

  const handleChange = (newValue: unknown, isValid: boolean) => {
    onValidationChange?.(isValid);
    if (isValid) {
      if (outputFormat === 'string') {
        onChange?.(JSON.stringify(newValue, null, 2));
      } else {
        onChange?.(newValue as Record<string, string>);
      }
    }
  };

  return (
    <JsonEditor
      value={jsonValue}
      onChange={handleChange}
      mode="text"
      height={height}
      resizable
    />
  );
};

export default HeadersEditor;
