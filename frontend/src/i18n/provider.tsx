import { useEffect, type ReactNode } from 'react';
import { I18nextProvider, useTranslation } from 'react-i18next';
import { I18nProvider } from 'react-aria-components';
import type { i18n } from 'i18next';
import { appI18n } from './i18n';
import { LOCALES, DEFAULT_LOCALE, isBahasaUi } from './locales';

function LangSync({ children }: { children: ReactNode }) {
  const { i18n: instance } = useTranslation();
  const tag = isBahasaUi(instance.language) ? instance.language : DEFAULT_LOCALE;
  useEffect(() => {
    document.documentElement.lang = tag;
  }, [tag]);
  return <I18nProvider locale={LOCALES[tag].formatLocale}>{children}</I18nProvider>;
}

/** I18nextProvider + React Aria I18nProvider + <html lang> (Addendum A section 4). */
export function AppI18nProvider({
  i18n = appI18n,
  children,
}: {
  i18n?: i18n;
  children: ReactNode;
}) {
  return (
    <I18nextProvider i18n={i18n}>
      <LangSync>{children}</LangSync>
    </I18nextProvider>
  );
}
