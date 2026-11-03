import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { createFormatter, hourRangeLabel, type Formatter } from './format';
import { DEFAULT_LOCALE, isBahasaUi, type BahasaUi } from './locales';

export interface AppFormatter extends Formatter {
  lng: BahasaUi;
  /** "Sen, 07.00–08.00" for the quota tracker's current hour (Addendum A section 5). */
  hourRange: (from: Date, to: Date) => string;
}

/** Formatter bound to the active UI language (Addendum A section 5). */
export function useFormatter(): AppFormatter {
  const { t, i18n } = useTranslation();
  const lng: BahasaUi = isBahasaUi(i18n.language) ? i18n.language : DEFAULT_LOCALE;
  // Intl formatters are expensive; rebuild them only when the language changes.
  return useMemo(() => {
    const base = createFormatter(lng, t);
    const locale = lng;
    return {
      ...base,
      lng,
      hourRange: (from: Date, to: Date) => hourRangeLabel(locale, from, to),
    };
    // `t` is stable per language for i18next; `lng` is the real dependency.
  }, [lng, t]);
}
