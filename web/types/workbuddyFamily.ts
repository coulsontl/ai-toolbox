// Types shared by WorkBuddy desktop and CodeBuddy Code.
//
// Both products are front ends over one engine and read the same `models.json`
// shape, so the channel/model view and the model input are one contract, not
// two. Keeping them here is what stops the two pages from drifting apart.

/** One model entry inside a channel. */
export interface WorkbuddyFamilyModelView {
  id: string;
  name: string;
  baseUrl: string;
  /**
   * Plaintext on purpose: AI Toolbox is a configuration manager, so credentials
   * are shown and editable instead of masked.
   */
  apiKey: string;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  temperature?: number;
  supportsToolCall: boolean;
  supportsImages: boolean;
  supportsReasoning: boolean;
  onlyReasoning: boolean;
  useCustomProtocol: boolean;
}

/** A group of model entries sharing one endpoint and API key. */
export interface WorkbuddyFamilyProviderView {
  /** Derived from the `(baseUrl, apiKey)` pair; never stored. */
  id: string;
  name: string;
  vendor: string;
  baseUrl: string;
  apiKey: string;
  models: WorkbuddyFamilyModelView[];
}

/** Input for creating or updating one model entry. */
export interface WorkbuddyFamilyModelInput {
  /** Existing id when editing; the form's new id otherwise. */
  previousId?: string | null;
  id: string;
  name?: string;
  vendor?: string;
  baseUrl: string;
  apiKey?: string;
  maxInputTokens?: number | null;
  maxOutputTokens?: number | null;
  temperature?: number | null;
  supportsToolCall?: boolean;
  supportsImages?: boolean;
  supportsReasoning?: boolean;
  onlyReasoning?: boolean;
  useCustomProtocol?: boolean;
}
