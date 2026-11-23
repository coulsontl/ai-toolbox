export interface GlobalPromptTargetFileOption {
  value: string;
  label: string;
  hint?: string;
}

export interface GlobalPromptConfig {
  id: string;
  name: string;
  content: string;
  targetFile?: string;
  isApplied: boolean;
  sortIndex?: number;
  createdAt?: string;
  updatedAt?: string;
}

export interface GlobalPromptConfigInput {
  id?: string;
  name: string;
  content: string;
  targetFile?: string;
}
