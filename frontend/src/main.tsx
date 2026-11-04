import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { AppI18nProvider } from './i18n/provider';
import { appI18n, loadLanguage } from './i18n/i18n';
import { DEFAULT_LOCALE, isBahasaUi } from './i18n/locales';
import { AppShell } from './app/AppShell';
import { RoutedScreens } from './app/routes';
import './styles/m3/tailwind.css';

const root = document.getElementById('root');
if (!root) throw new Error('index.html: #root tidak ditemukan');

/**
 * Interface language for this document. The locale switcher in the shell is not part of F0 yet, so
 * `?bahasa=en` is the preview-only seam the e2e suite uses to render a screen in a second language.
 * It is read once, before the first render, so no screen ever paints in the wrong catalogue.
 */
const requested = new URLSearchParams(window.location.search).get('bahasa');
const startLocale = isBahasaUi(requested) ? requested : DEFAULT_LOCALE;

async function start(): Promise<void> {
  if (startLocale !== DEFAULT_LOCALE) await loadLanguage(appI18n, startLocale);

  createRoot(root!).render(
    <StrictMode>
      <AppI18nProvider>
        <AppShell pathname={window.location.pathname}>
          <RoutedScreens pathname={window.location.pathname} search={window.location.search} />
        </AppShell>
      </AppI18nProvider>
    </StrictMode>,
  );
}

void start();
