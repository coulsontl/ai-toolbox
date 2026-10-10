import React from 'react';
import ImeSafeAutoComplete from '@/components/common/ImeSafeAutoComplete';

interface Props {
  id?: string;
  value?: string;
  options: Array<{ value: string; label: string }>;
  placeholder: string;
  disabled?: boolean;
  onChange?: (value: string) => void;
}

/** Keep manual IDs local until committed; never save partial IDs on each keypress. */
const OmpCodexModelInput: React.FC<Props> = ({ value, onChange, ...props }) => {
  const [draft, setDraft] = React.useState(value ?? '');
  const draftRef = React.useRef(value ?? '');
  const committedRef = React.useRef(value ?? '');

  React.useEffect(() => {
    draftRef.current = value ?? '';
    committedRef.current = value ?? '';
    setDraft(value ?? '');
  }, [value]);

  const handleDraftChange = (next: string) => {
    draftRef.current = next;
    setDraft(next);
  };

  const handleCommit = (next = draftRef.current) => {
    const modelId = next.trim();
    handleDraftChange(modelId);
    if (modelId === committedRef.current) return;
    committedRef.current = modelId;
    onChange?.(modelId);
  };

  return (
    <ImeSafeAutoComplete
      {...props}
      value={draft}
      onChange={handleDraftChange}
      onSelect={(next) => handleCommit(next)}
      onBlur={() => handleCommit()}
      onInputKeyDown={(event) => {
        if (event.key === 'Enter' && !event.nativeEvent.isComposing && event.keyCode !== 229) {
          // Let the same Enter event select a highlighted option before saving.
          queueMicrotask(() => handleCommit());
        }
      }}
      filterOption={(input, option) => (option?.value ?? '').toLowerCase().includes(input.toLowerCase())}
    />
  );
};

export default OmpCodexModelInput;
