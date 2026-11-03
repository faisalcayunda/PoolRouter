import type { MessageId } from './types';

/**
 * Label keys for server enum values (Addendum A section 6.3). The `Record<EnumServer, MessageId>`
 * over schema.gen.ts is added with the screens that show these enums; the catalog group is `enum.*`.
 */
export function enumLabelId(group: string, value: string): MessageId {
  return `enum.${group}.${value}` as MessageId;
}
