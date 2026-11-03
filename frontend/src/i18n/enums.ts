import type { i18n } from 'i18next';
import type { MessageId } from './types';

/**
 * Label keys for server enum values (Addendum A section 6.3). The catalog group is `enum.*`.
 */
export function enumLabelId(group: string, value: string): MessageId {
  return `enum.${group}.${value}` as MessageId;
}

/**
 * Translate a server enum value through the shared `enum.*` group.
 *
 * The generated schema types some enum fields as `unknown`, so the value is narrowed to a string
 * first. A value outside the set renders as the raw code rather than as a missing-key placeholder:
 * an enum can grow before the catalog does, and a raw code is at least honest.
 */
export function enumLabel(instance: i18n, group: string, value: unknown): string {
  if (typeof value !== 'string' || value === '') return '—';
  const raw: string = value;
  const key = enumLabelId(group, raw);
  return instance.exists(key as string) ? instance.t(key) : raw;
}
