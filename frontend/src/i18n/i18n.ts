import i18next, { type i18n } from 'i18next';
import { initReactI18next } from 'react-i18next';
import id from './messages/id.json';
import { DEFAULT_LOCALE, LOCALES, SUPPORTED_LOCALES, type BahasaUi } from './locales';

export interface CreateI18nOptions {
  /** Test mode: a missing key throws (Addendum A section 4, "Key hilang saat runtime"). */
  throwOnMissingKey?: boolean;
  lng?: BahasaUi;
}

/**
 * Mandatory configuration of Addendum A rev 2 section 3.3. `id` is bundled eagerly; other
 * languages are loaded with `loadLanguage` (dynamic import, same-origin chunk, CSP safe).
 * No language detector, HTTP/fs backend, locize or ICU plugin.
 */
export function createI18n(options: CreateI18nOptions = {}): i18n {
  const instance = i18next.createInstance();
  instance.use(initReactI18next);
  void instance.init({
    resources: { id: { translation: id } },
    lng: options.lng ?? DEFAULT_LOCALE,
    supportedLngs: [...SUPPORTED_LOCALES],
    fallbackLng: DEFAULT_LOCALE,
    load: 'languageOnly',
    defaultNS: 'translation',
    ns: ['translation'],
    interpolation: { escapeValue: false },
    returnNull: false,
    saveMissing: false,
    react: { useSuspense: false },
    ...(options.throwOnMissingKey
      ? {
          saveMissing: true,
          missingKeyHandler: (_lngs: readonly string[], _ns: string, key: string) => {
            throw new Error(`i18n: missing key ${key}`);
          },
        }
      : {}),
  });
  return instance;
}

/** Load a language catalog (if needed) and switch to it. Resolves after the switch. */
export async function loadLanguage(instance: i18n, tag: BahasaUi): Promise<void> {
  if (!instance.hasResourceBundle(tag, 'translation')) {
    const messages = await LOCALES[tag].load();
    instance.addResourceBundle(tag, 'translation', messages, true, true);
  }
  await instance.changeLanguage(tag);
}

export const appI18n = createI18n();
