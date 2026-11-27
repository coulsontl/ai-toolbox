import React from 'react';
import {
  Antigravity,
  AzureAI,
  Bedrock,
  ByteDance,
  Claude,
  Cline,
  CodeBuddy,
  Cohere,
  Copilot,
  Cursor,
  DeepSeek,
  Devin,
  Doubao,
  Fireworks,
  Gemini,
  Github,
  Grok,
  Groq,
  HermesAgent,
  HuggingFace,
  Hunyuan,
  Kimi,
  Kiro,
  Meta,
  Minimax,
  Mistral,
  Nvidia,
  Ollama,
  OpenAI,
  OpenClaw,
  OpenCode,
  OpenRouter,
  Perplexity,
  Poe,
  Qoder,
  Qwen,
  RooCode,
  SiliconCloud,
  Stepfun,
  Together,
  Trae,
  VertexAI,
  Volcengine,
  Windsurf,
  XiaomiMiMo,
  ZAI,
  Zhipu,
} from '@lobehub/icons';
import droidMarkRaw from '@/assets/agent-icons/droid.svg?raw';
import workbuddyIconUrl from '@/assets/agent-icons/workbuddy.png';
import {
  getPluginAccentIndex,
  getPluginInitials,
} from '../utils/pluginFormatters';
import styles from './PluginIcon.module.less';

interface PluginIconProps {
  /** A remote image URL or `data:image` URI supplied by the plugin. */
  icon?: string;
  name: string;
  size?: number;
  className?: string;
}

interface BrandMatcher {
  pattern: RegExp;
  render: (size: number) => React.ReactNode;
}

// Local marks for brands LobeHub has no icon for. The PNG is drawn inside the
// plugin tile; the droid SVG is `currentColor`, so it follows the tile's
// theme-aware text/accent colour.
const WorkbuddyMark: React.FC<{ size: number }> = ({ size }) => (
  <img
    src={workbuddyIconUrl}
    alt=""
    width={size}
    height={size}
    draggable={false}
    className={styles.brandImage}
  />
);

const DroidMark: React.FC<{ size: number }> = ({ size }) => (
  <span
    className={styles.rawBrand}
    style={{ width: size, height: size }}
    aria-hidden="true"
    dangerouslySetInnerHTML={{ __html: droidMarkRaw }}
  />
);

// Order matters: `opencode` before `openai`, `qwenwork`/`qoder` before
// `qwen`, and `groq` before the broader `grok` match. Short tokens such as
// `ark`, `aws`, `poe` and `zai` use word boundaries so they do not fire
// inside unrelated words.
const BRAND_MATCHERS: BrandMatcher[] = [
  { pattern: /\bopen[\s-]?code\b/i, render: (size) => <OpenCode size={size} /> },
  { pattern: /\bopen[\s-]?claw\b/i, render: (size) => <OpenClaw.Color size={size} /> },
  { pattern: /(openai|chatgpt|gpt-|codex)/i, render: (size) => <OpenAI size={size} /> },
  { pattern: /(claude|anthropic)/i, render: (size) => <Claude.Color size={size} /> },
  { pattern: /(gemini|google)/i, render: (size) => <Gemini.Color size={size} /> },
  { pattern: /\bantigravity\b/i, render: (size) => <Antigravity.Color size={size} /> },
  { pattern: /deepseek/i, render: (size) => <DeepSeek.Color size={size} /> },
  { pattern: /\bwork[\s-]?buddy\b/i, render: (size) => <WorkbuddyMark size={size} /> },
  { pattern: /\bcode[\s-]?buddy\b/i, render: (size) => <CodeBuddy.Color size={size} /> },
  { pattern: /\bqoder\b|\bqwen[\s-]?work\b/i, render: (size) => <Qoder.Color size={size} /> },
  { pattern: /(qwen|tongyi|alibaba)/i, render: (size) => <Qwen.Color size={size} /> },
  { pattern: /(kimi|moonshot)/i, render: (size) => <Kimi size={size} /> },
  { pattern: /(zhipu|bigmodel|glm)/i, render: (size) => <Zhipu.Color size={size} /> },
  { pattern: /(?:\bzai\b|z\.ai|\bzcode\b)/i, render: (size) => <ZAI size={size} /> },
  { pattern: /\bgroq\b/i, render: (size) => <Groq size={size} /> },
  { pattern: /(?:\bgrok\b|\bxai\b)/i, render: (size) => <Grok size={size} /> },
  { pattern: /mistral/i, render: (size) => <Mistral.Color size={size} /> },
  { pattern: /(?:\bmeta\b|llama)/i, render: (size) => <Meta.Color size={size} /> },
  { pattern: /openrouter/i, render: (size) => <OpenRouter.Color size={size} /> },
  { pattern: /ollama/i, render: (size) => <Ollama size={size} /> },
  { pattern: /copilot/i, render: (size) => <Copilot.Color size={size} /> },
  { pattern: /github/i, render: (size) => <Github size={size} /> },
  { pattern: /perplexity/i, render: (size) => <Perplexity.Color size={size} /> },
  { pattern: /cohere/i, render: (size) => <Cohere.Color size={size} /> },
  { pattern: /\bcursor\b/i, render: (size) => <Cursor size={size} /> },
  { pattern: /\bwindsurf\b/i, render: (size) => <Windsurf size={size} /> },
  { pattern: /\bcline\b/i, render: (size) => <Cline size={size} /> },
  { pattern: /\broo[\s-]?code\b/i, render: (size) => <RooCode size={size} /> },
  { pattern: /\bhermes(?:[\s-]?agent)?\b/i, render: (size) => <HermesAgent size={size} /> },
  { pattern: /\bkiro\b/i, render: (size) => <Kiro.Color size={size} /> },
  { pattern: /\bdevin\b/i, render: (size) => <Devin.Color size={size} /> },
  { pattern: /\btrae\b/i, render: (size) => <Trae.Color size={size} /> },
  { pattern: /\bminimax\b/i, render: (size) => <Minimax.Color size={size} /> },
  { pattern: /\bdoubao\b/i, render: (size) => <Doubao.Color size={size} /> },
  { pattern: /byte[\s-]?dance/i, render: (size) => <ByteDance.Color size={size} /> },
  { pattern: /\bhunyuan\b|\btencent\b/i, render: (size) => <Hunyuan.Color size={size} /> },
  { pattern: /\bvolcengine\b|\bark\b/i, render: (size) => <Volcengine.Color size={size} /> },
  { pattern: /silicon[\s-]?(?:flow|cloud)/i, render: (size) => <SiliconCloud.Color size={size} /> },
  { pattern: /\btogether\b/i, render: (size) => <Together.Color size={size} /> },
  { pattern: /\bfireworks\b/i, render: (size) => <Fireworks.Color size={size} /> },
  { pattern: /\bbedrock\b|\baws\b/i, render: (size) => <Bedrock.Color size={size} /> },
  { pattern: /\bvertex(?:ai)?\b/i, render: (size) => <VertexAI.Color size={size} /> },
  { pattern: /\bazure(?:ai)?\b/i, render: (size) => <AzureAI.Color size={size} /> },
  { pattern: /hugging[\s-]?face/i, render: (size) => <HuggingFace.Color size={size} /> },
  { pattern: /\bnvidia\b/i, render: (size) => <Nvidia.Color size={size} /> },
  { pattern: /\bpoe\b/i, render: (size) => <Poe.Color size={size} /> },
  { pattern: /\bstepfun\b/i, render: (size) => <Stepfun size={size} /> },
  { pattern: /xiaomi|\bmimo(?:code)?\b/i, render: (size) => <XiaomiMiMo size={size} /> },
  { pattern: /\b(?:factory[\s-]?droid|droid)\b/i, render: (size) => <DroidMark size={size} /> },
];

export const isPluginRemoteIcon = (icon: string) => /^(https:\/\/|data:image\/)/i.test(icon.trim());

/** Whether a plugin icon can render as a picture or a known brand mark. */
export const hasPluginIconVisual = (icon: string | null | undefined, name: string): boolean => {
  const normalizedIcon = icon?.trim() ?? '';
  if (!normalizedIcon) {
    return false;
  }
  if (isPluginRemoteIcon(normalizedIcon)) {
    return true;
  }
  return BRAND_MATCHERS.some((matcher) => matcher.pattern.test(`${name} ${normalizedIcon}`));
};

const PluginIcon: React.FC<PluginIconProps> = ({ icon, name, size = 36, className }) => {
  const [imageFailed, setImageFailed] = React.useState(false);
  const normalizedIcon = icon?.trim() ?? '';
  const remoteIcon = isPluginRemoteIcon(normalizedIcon);

  React.useEffect(() => {
    setImageFailed(false);
  }, [normalizedIcon]);

  const accentIndex = getPluginAccentIndex(`${name} ${normalizedIcon}`);
  const accentClassName = styles[`accent${accentIndex}`] ?? '';
  const brand = remoteIcon
    ? undefined
    : BRAND_MATCHERS.find((matcher) => matcher.pattern.test(`${name} ${normalizedIcon}`));

  const containerClassName = [
    styles.icon,
    !remoteIcon || imageFailed ? accentClassName : '',
    className ?? '',
  ].filter(Boolean).join(' ');

  return (
    <span
      className={containerClassName}
      style={{ width: size, height: size, borderRadius: size >= 36 ? 8 : 7 }}
      aria-hidden="true"
    >
      {remoteIcon && !imageFailed ? (
        <img
          src={normalizedIcon}
          alt=""
          className={styles.image}
          onError={() => setImageFailed(true)}
        />
      ) : brand ? (
        brand.render(Math.round(size * 0.56))
      ) : (
        <span>{getPluginInitials(name)}</span>
      )}
    </span>
  );
};

export default PluginIcon;
