import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { AppI18nProvider } from './i18n/provider';
import { AppShell } from './app/AppShell';
import './styles/m3/tailwind.css';

const root = document.getElementById('root');
if (!root) throw new Error('index.html: #root tidak ditemukan');

createRoot(root).render(
  <StrictMode>
    <AppI18nProvider>
      <AppShell pathname={window.location.pathname} />
    </AppI18nProvider>
  </StrictMode>,
);
