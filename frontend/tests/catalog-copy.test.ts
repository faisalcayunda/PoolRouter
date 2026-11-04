import { describe, expect, it } from 'vitest';
import id from '../src/i18n/messages/id.json';
import en from '../src/i18n/messages/en.json';

/** Internal work references mean nothing to a BGN admin and must not reach the screen. */
const INTERNAL_REFERENCE = /\b(OQ|SHAA|ADR)-\d+/;

function strings(node: unknown, path = ''): [string, string][] {
  if (typeof node === 'string') return [[path, node]];
  if (node && typeof node === 'object') {
    return Object.entries(node).flatMap(([key, value]) =>
      strings(value, path ? `${path}.${key}` : key),
    );
  }
  return [];
}

describe('catalog copy', () => {
  it.each([
    ['id', id],
    ['en', en],
  ])('has no internal work reference in the %s catalog', (_lng, catalog) => {
    const offenders = strings(catalog).filter(([, text]) => INTERNAL_REFERENCE.test(text));
    expect(offenders).toEqual([]);
  });
});
