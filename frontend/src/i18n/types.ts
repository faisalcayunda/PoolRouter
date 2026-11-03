import type { ParseKeys } from 'i18next';

/** Typed catalog key (Addendum A section 4). Components accept this, never raw strings. */
export type MessageId = ParseKeys;
