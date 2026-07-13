import { createGlobalPromptApi } from './globalPromptApi';

export const grokPromptApi = createGlobalPromptApi({
  list: 'list_grok_prompt_configs',
  create: 'create_grok_prompt_config',
  update: 'update_grok_prompt_config',
  delete: 'delete_grok_prompt_config',
  apply: 'apply_grok_prompt_config',
  reorder: 'reorder_grok_prompt_configs',
  saveLocal: 'save_grok_local_prompt_config',
});
