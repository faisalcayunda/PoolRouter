import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createI18n } from '../i18n/i18n';
import type { i18n } from 'i18next';
import { AppI18nProvider } from '../i18n/provider';
import { AppShell } from './AppShell';
import { activeHref } from './nav';

/** Drive the `expanded` media query without a real viewport. */
function setExpanded(matches: boolean) {
  window.matchMedia = ((query: string) => ({
    matches,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

let instance: i18n;

function renderShell(pathname = '/') {
  return render(
    <AppI18nProvider i18n={instance}>
      <AppShell pathname={pathname}>
        <p>ISI Halaman</p>
      </AppShell>
    </AppI18nProvider>,
  );
}

beforeEach(() => {
  instance = createI18n({ throwOnMissingKey: true });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('AppShell — compact window size class', () => {
  beforeEach(() => setExpanded(false));

  it('hides the drawer and exposes a menu button that opens it as a modal', async () => {
    renderShell();
    // Drawer navigation is not in the tree until opened.
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
    const opener = screen.getByRole('button', { name: 'Buka menu navigasi' });
    await userEvent.click(opener);
    const dialog = await screen.findByRole('dialog', { name: 'Navigasi utama' });
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(screen.getByRole('navigation', { name: 'Navigasi utama' })).toBeInTheDocument();
  });

  it('closes the modal drawer with Escape', async () => {
    renderShell();
    await userEvent.click(screen.getByRole('button', { name: 'Buka menu navigasi' }));
    await screen.findByRole('dialog');
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('closes the modal drawer after navigating', async () => {
    renderShell();
    await userEvent.click(screen.getByRole('button', { name: 'Buka menu navigasi' }));
    const link = await screen.findByRole('link', { name: 'Provider' });
    await userEvent.click(link);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });
});

describe('AppShell — expanded window size class', () => {
  beforeEach(() => setExpanded(true));

  it('shows a permanent drawer and no menu button', () => {
    renderShell();
    expect(screen.getByRole('navigation', { name: 'Navigasi utama' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Buka menu navigasi' })).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('AppShell — structure and a11y', () => {
  beforeEach(() => setExpanded(true));

  it('renders a skip link that targets the main landmark', () => {
    renderShell();
    const skip = screen.getByRole('link', { name: 'Lompat ke konten' });
    expect(skip).toHaveAttribute('href', '#main-content');
    expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content');
  });

  it('renders children inside main', () => {
    renderShell();
    expect(screen.getByRole('main')).toHaveTextContent('ISI Halaman');
  });

  it('marks the active item with aria-current="page"', () => {
    renderShell('/provider');
    expect(screen.getByRole('link', { name: 'Provider' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('link', { name: 'Secret' })).not.toHaveAttribute('aria-current');
  });

  it('every drawer label comes from the catalog (no missing key)', () => {
    // createI18n({ throwOnMissingKey: true }) throws on any key absent from id.json.
    expect(() => renderShell()).not.toThrow();
    expect(screen.getByText('Pemantauan')).toBeInTheDocument();
    expect(screen.getByText('Konfigurasi')).toBeInTheDocument();
  });
});

describe('activeHref', () => {
  it('picks the longest matching prefix', () => {
    expect(activeHref('/provider/123')).toBe('/provider');
    expect(activeHref('/provider')).toBe('/provider');
    expect(activeHref('/')).toBe('/');
    expect(activeHref('/tidak-ada')).toBeUndefined();
  });
});
