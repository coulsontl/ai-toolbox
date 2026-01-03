/**
 * AI SDK Provider Types
 * 
 * Predefined list of AI SDK provider types.
 * Common providers are listed first for convenience.
 */

export interface ProviderTypeOption {
  value: string;
  label: string;
}

export const PROVIDER_TYPES: ProviderTypeOption[] = [
  // Common providers (most frequently used)
  { value: '@ai-sdk/openai-compatible', label: 'OpenAI Compatible' },
  { value: '@ai-sdk/openai', label: 'OpenAI' },
  { value: '@ai-sdk/anthropic', label: 'Anthropic' },
  { value: '@ai-sdk/google', label: 'Google Generative AI' },
  
  // Other providers (alphabetically sorted)
  { value: '@ai-sdk/amazon-bedrock', label: 'Amazon Bedrock' },
  { value: '@ai-sdk/assemblyai', label: 'AssemblyAI' },
  { value: '@ai-sdk/azure', label: 'Azure OpenAI' },
  { value: '@ai-sdk/baseten', label: 'Baseten' },
  { value: '@ai-sdk/cerebras', label: 'Cerebras' },
  { value: '@ai-sdk/cohere', label: 'Cohere' },
  { value: '@ai-sdk/deepgram', label: 'Deepgram' },
  { value: '@ai-sdk/deepinfra', label: 'DeepInfra' },
  { value: '@ai-sdk/deepseek', label: 'DeepSeek' },
  { value: '@ai-sdk/elevenlabs', label: 'ElevenLabs' },
  { value: '@ai-sdk/fireworks', label: 'Fireworks' },
  { value: '@ai-sdk/gladia', label: 'Gladia' },
  { value: '@ai-sdk/google-vertex', label: 'Google Vertex' },
  { value: '@ai-sdk/groq', label: 'Groq' },
  { value: '@ai-sdk/hume', label: 'Hume' },
  { value: '@ai-sdk/lmnt', label: 'LMNT' },
  { value: '@ai-sdk/mistral', label: 'Mistral' },
  { value: '@ai-sdk/perplexity', label: 'Perplexity' },
  { value: '@ai-sdk/revai', label: 'Rev.ai' },
  { value: '@ai-sdk/togetherai', label: 'Together.ai' },
  { value: '@ai-sdk/xai', label: 'xAI Grok' },
];
