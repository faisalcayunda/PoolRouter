import type { MessageId } from './types';

/**
 * Catalog keys for UI errors (Addendum A section 6.1). The exhaustive `Record<ProblemKode, MessageId>`
 * over the OpenAPI enum arrives with admin-api.yaml; the catalog v0.13 only has these error keys so far.
 */
export const ERROR_MESSAGE_IDS = {
  network: 'error.network.load',
  forbiddenTitle: 'error.http.403.title',
  forbiddenBody: 'error.http.403.body',
  forbiddenAction: 'error.http.403.action',
} as const satisfies Record<string, MessageId>;
