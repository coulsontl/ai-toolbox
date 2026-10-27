import type { ModelPricing } from '@/services';
import { filterProviderItems } from '../../shared/providerList/sortProviders';

/**
 * Local search over the model pricing list. The backend returns the whole
 * price table at once, so filtering stays in the modal.
 *
 * Only `model_id` and `display_name` are searchable: the price fields are
 * strings like `"0"` or `"5"`, so including them would make a one-digit query
 * match nearly every row. Matching (trim, case-insensitive, empty keyword
 * returns the same array) comes from `filterProviderItems`.
 */
export const filterModelPricing = (pricingList: ModelPricing[], keyword: string) =>
  filterProviderItems(pricingList, keyword, (pricing) => [
    pricing.model_id,
    pricing.display_name,
  ]);
