import { createGlobalPromptApi } from './globalPromptApi';

export const codebuddyPromptApi = createGlobalPromptApi({
  list: 'list_codebuddy_prompt_configs',
  create: 'create_codebuddy_prompt_config',
  update: 'update_codebuddy_prompt_config',
  delete: 'delete_codebuddy_prompt_config',
  apply: 'apply_codebuddy_prompt_config',
  disable: 'disable_codebuddy_prompt_config',
  reorder: 'reorder_codebuddy_prompt_configs',
  saveLocal: 'save_codebuddy_local_prompt_config',
});
