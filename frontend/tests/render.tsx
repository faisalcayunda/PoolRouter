import type { ReactNode } from 'react';
import { render, type RenderOptions, type RenderResult } from '@testing-library/react';
import type { i18n } from 'i18next';
import { createI18n } from '../src/i18n/i18n';
import { AppI18nProvider } from '../src/i18n/provider';

/**
 * Render with a fresh i18n instance in test mode: a missing key throws instead of silently falling
 * back (Addendum A section 4), so a screen that invents a key fails its own test.
 */
export function renderWithI18n(
  ui: ReactNode,
  options: RenderOptions & { lng?: 'id' | 'en'; instance?: i18n } = {},
): RenderResult & { i18nInstance: i18n } {
  const { lng, instance, ...rest } = options;
  const i18nInstance = instance ?? createI18n({ throwOnMissingKey: true, lng });
  return {
    ...render(<AppI18nProvider i18n={i18nInstance}>{ui}</AppI18nProvider>, rest),
    i18nInstance,
  };
}

/** Frozen "now" so relative copy (reset time, current hour) is deterministic. */
export const FIXED_NOW = new Date('2026-11-03T07:30:00.000Z');
