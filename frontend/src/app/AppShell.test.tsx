import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createI18n } from '../i18n/i18n';
import type { i18n } from 'i18next';
import { AppI18nProvider } from '../i18n/provider';
import { AppShell } from './AppShell';
import { activeHref, crumbsFor, navGroupsFor } from './nav';
import type { AdminRole } from '../features/common/RoleContext';

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

function renderShell(pathname = '/', role: AdminRole | null = null) {
  return render(
    <AppI18nProvider i18n={instance}>
      <AppShell pathname={pathname} role={role}>
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
    expect(screen.getByRole('navigation', { name: 'Navigasi utama' })).toBeInTheDocument();
    // Focus moves into the drawer; React Aria hides the rest of the app from AT
    // instead of setting aria-modal, so assert the observable effect.
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    expect(screen.queryByRole('main')).not.toBeInTheDocument();
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

describe('AppShell — menu per role', () => {
  beforeEach(() => setExpanded(true));

  it('shows the full menu while the role is unknown', () => {
    renderShell('/', null);
    expect(screen.getByRole('link', { name: 'Provider' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Pencarian log per NIK' })).toBeInTheDocument();
  });

  it('auditor sees logs, access audit and NIK search but no configuration', () => {
    renderShell('/', 'auditor');
    expect(screen.getByRole('link', { name: 'Pencarian log per NIK' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Audit akses' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Provider' })).not.toBeInTheDocument();
    expect(screen.queryByText('Konfigurasi')).not.toBeInTheDocument();
  });

  it('operator has no secret, NIK search or access audit', () => {
    renderShell('/', 'operator');
    expect(screen.getByRole('link', { name: 'Kuota' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Secret' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Pencarian log per NIK' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Audit akses' })).not.toBeInTheDocument();
  });

  it('every role keeps logs and settings', () => {
    for (const role of ['platform_admin', 'operator', 'auditor'] as const) {
      const hrefs = navGroupsFor(role).flatMap((g) => g.items.map((i) => i.href));
      expect(hrefs).toContain('/');
      expect(hrefs).toContain('/settings');
    }
  });

  it('uses the inverse surface for the drawer', () => {
    renderShell('/', 'auditor');
    expect(screen.getByRole('complementary', { name: 'Navigasi utama' }).className).toContain(
      'bg-inverse-surface',
    );
  });

  it('lets the drawer links be reached with Tab', async () => {
    renderShell('/', 'auditor');
    await userEvent.tab(); // skip link
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Log transaksi' }));
  });
});

describe('AppShell — breadcrumbs', () => {
  beforeEach(() => setExpanded(true));

  it('renders a trail ending at the current page', () => {
    renderShell('/kuota');
    const trail = screen.getByRole('navigation', { name: 'Jejak halaman' });
    expect(trail).toHaveTextContent('Pemantauan');
    expect(screen.getByText('Kuota', { selector: '[aria-current="page"]' })).toBeInTheDocument();
  });

  it('links back to the list from a detail route', () => {
    renderShell('/provider/abc');
    const trail = screen.getByRole('navigation', { name: 'Jejak halaman' });
    expect(within(trail).getByRole('link', { name: 'Provider' })).toHaveAttribute(
      'href',
      '/provider',
    );
    expect(within(trail).getByText('abc')).toHaveAttribute('aria-current', 'page');
  });

  it('has a trail on the not-found route too', () => {
    renderShell('/tidak-ada');
    const trail = screen.getByRole('navigation', { name: 'Jejak halaman' });
    expect(trail).toHaveTextContent('Halaman tidak ditemukan');
  });
});

describe('crumbsFor', () => {
  it('covers the list, detail, new and unknown routes', () => {
    expect(crumbsFor('/')).toHaveLength(2);
    expect(crumbsFor('/log/TX-1')).toEqual([
      { labelKey: 'nav.group.monitoring' },
      { labelKey: 'nav.drawer.log', href: '/' },
      { text: 'TX-1' },
    ]);
    expect(crumbsFor('/provider/baru').at(-1)).toEqual({ labelKey: 'nav.breadcrumb.new' });
    expect(crumbsFor('/x')).toEqual([{ labelKey: 'nav.breadcrumb.not_found' }]);
  });
});
