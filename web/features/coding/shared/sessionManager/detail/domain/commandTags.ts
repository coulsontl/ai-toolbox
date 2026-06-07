export interface ParsedSessionCommand {
  commandName?: string;
  commandMessage?: string;
  commandArgs?: string;
  localStdoutBlocks: string[];
  outputTags: Array<{
    type: 'stdout' | 'stderr';
    name: string;
    content: string;
  }>;
  caveats: string[];
  remainingText: string;
}

const COMMAND_NAME_PATTERN = /<command-name>\s*([\s\S]*?)\s*<\/command-name>/g;
const COMMAND_MESSAGE_PATTERN = /<command-message>\s*([\s\S]*?)\s*<\/command-message>/g;
const COMMAND_ARGS_PATTERN = /<command-args>\s*([\s\S]*?)\s*<\/command-args>/g;
const LOCAL_COMMAND_STDOUT_PATTERN = /<local-command-stdout>\s*([\s\S]*?)\s*<\/local-command-stdout>/g;
const LOCAL_COMMAND_CAVEAT_PATTERN = /<local-command-caveat>\s*([\s\S]*?)\s*<\/local-command-caveat>/g;
const STDOUT_PATTERN = /<(?!local-command-stdout)([^>]*(?:stdout|output)[^>]*)>\s*([\s\S]*?)\s*<\/\1>/g;
const STDERR_PATTERN = /<([^>]*(?:stderr|error)[^>]*)>\s*([\s\S]*?)\s*<\/\1>/g;

export function hasSessionCommandTags(text: string): boolean {
  return /<command-name>[\s\S]*?<\/command-name>/.test(text)
    || /<command-message>[\s\S]*?<\/command-message>/.test(text)
    || /<command-args>[\s\S]*?<\/command-args>/.test(text)
    || /<local-command-caveat>[\s\S]*?<\/local-command-caveat>/.test(text)
    || /<local-command-stdout>[\s\S]*?<\/local-command-stdout>/.test(text)
    || /<[^>]*(?:stdout|stderr|output|error)[^>]*>[\s\S]*?<\/[^>]+>/.test(text);
}

export function parseSessionCommandTags(text: string): ParsedSessionCommand {
  const commandName = firstTagContent(text, COMMAND_NAME_PATTERN);
  const rawCommandMessage = firstTagContent(text, COMMAND_MESSAGE_PATTERN);
  const commandArgs = firstTagContent(text, COMMAND_ARGS_PATTERN);
  const normalizedName = commandName?.replace(/^\//, '');
  const commandMessage = rawCommandMessage && rawCommandMessage !== normalizedName
    ? rawCommandMessage
    : undefined;

  const localStdoutBlocks = collectTagContents(text, LOCAL_COMMAND_STDOUT_PATTERN);
  const caveats = collectTagContents(text, LOCAL_COMMAND_CAVEAT_PATTERN);
  const outputTags = [
    ...collectNamedOutputTags(text, STDOUT_PATTERN, 'stdout'),
    ...collectNamedOutputTags(text, STDERR_PATTERN, 'stderr'),
  ];
  const remainingText = text
    .replace(COMMAND_NAME_PATTERN, '')
    .replace(COMMAND_MESSAGE_PATTERN, '')
    .replace(COMMAND_ARGS_PATTERN, '')
    .replace(LOCAL_COMMAND_STDOUT_PATTERN, '')
    .replace(LOCAL_COMMAND_CAVEAT_PATTERN, '')
    .replace(STDOUT_PATTERN, '')
    .replace(STDERR_PATTERN, '')
    .replace(/^\s*\n/gm, '')
    .trim();

  return {
    commandName,
    commandMessage,
    commandArgs,
    localStdoutBlocks,
    outputTags,
    caveats,
    remainingText,
  };
}

function firstTagContent(text: string, pattern: RegExp): string | undefined {
  pattern.lastIndex = 0;
  const match = pattern.exec(text);
  const content = match?.[1]?.trim();
  pattern.lastIndex = 0;
  return content || undefined;
}

function collectTagContents(text: string, pattern: RegExp): string[] {
  const values: string[] = [];
  pattern.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    const content = match[1]?.trim();
    if (content) {
      values.push(content);
    }
  }
  pattern.lastIndex = 0;
  return values;
}

function collectNamedOutputTags(
  text: string,
  pattern: RegExp,
  type: 'stdout' | 'stderr',
): ParsedSessionCommand['outputTags'] {
  const values: ParsedSessionCommand['outputTags'] = [];
  pattern.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    const name = match[1]?.trim();
    const content = match[2]?.trim();
    if (name && content) {
      values.push({ type, name, content });
    }
  }
  pattern.lastIndex = 0;
  return values;
}
