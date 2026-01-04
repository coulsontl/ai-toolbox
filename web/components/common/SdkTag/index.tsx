import React from 'react';
import { Tag } from 'antd';
import { openUrl } from '@tauri-apps/plugin-opener';

const AI_SDK_DOCS_URL = 'https://ai-sdk.dev/docs/foundations/providers-and-models#ai-sdk-providers';

interface SdkTagProps {
  /** The SDK name to display (e.g., '@ai-sdk/openai') */
  name: string;
}

/**
 * A clickable tag component that displays an AI SDK name and opens
 * the AI SDK documentation when clicked.
 */
const SdkTag: React.FC<SdkTagProps> = ({ name }) => {
  const handleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    openUrl(AI_SDK_DOCS_URL);
  };

  return (
    <Tag
      color="blue"
      style={{ margin: 0, cursor: 'pointer' }}
      onClick={handleClick}
    >
      {name}
    </Tag>
  );
};

export default SdkTag;
