import type { i18n } from 'i18next';
import type { MessageId } from '../i18n/types';

/**
 * Translate a key that may legitimately not exist yet, without tripping the missing-key handler.
 *
 * Server enums can grow (`aksi_audit`, `status_akses`) and a problem code the catalog has no line
 * for must render as its raw code or the screen's generic line, not crash the page. i18next's
 * `defaultValue` option does **not** help: it still calls `missingKeyHandler`, which throws in test
 * mode so a typo in the catalog fails the suite (Addendum A section 4). `i18n.exists` is the one
 * lookup that does not count as a missing key.
 */
export function labelOr(instance: i18n, key: string, fallback: string): string {
  return instance.exists(key) ? instance.t(key as MessageId) : fallback;
}
