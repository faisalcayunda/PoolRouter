export type BahasaUi = 'id' | 'en';

export interface LocaleEntry {
  /** i18next language tag. */
  tag: BahasaUi;
  /** Locale for Intl and the React Aria I18nProvider (Addendum A section 5.1). */
  formatLocale: 'id-ID' | 'en-GB';
  /** Endonym: never translated (Addendum A section 10.1). */
  endonym: string;
  load: () => Promise<Record<string, unknown>>;
}

/** Registry of UI languages (I-9): add a language by adding a catalog file and one entry here. */
export const LOCALES: Record<BahasaUi, LocaleEntry> = {
  id: {
    tag: 'id',
    formatLocale: 'id-ID',
    endonym: 'Bahasa Indonesia',
    load: () => import('./messages/id.json').then((m) => m.default),
  },
  en: {
    tag: 'en',
    formatLocale: 'en-GB',
    endonym: 'English',
    load: () => import('./messages/en.json').then((m) => m.default),
  },
};

export const DEFAULT_LOCALE: BahasaUi = 'id';
export const SUPPORTED_LOCALES = Object.keys(LOCALES) as BahasaUi[];

export function isBahasaUi(value: unknown): value is BahasaUi {
  return typeof value === 'string' && value in LOCALES;
}
