import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createI18n } from '../i18n/i18n';
import type { i18n } from 'i18next';
import { AppI18nProvider } from '../i18n/provider';
import { AppShell } from './AppShell';
import { activeHref, crumbsFor, navGroupsFor } from './nav';
import type { AdminRole } from '../features/common/RoleContext';

/** Drive the `large` (>= 1200 px) media query without a real viewport. */
function setLarge(matches: boolean) {
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

function renderShell(
  pathname = '/',
  role: AdminRole | null = null,
  user?: { username: string; peran: string },
) {
  return render(
    <AppI18nProvider i18n={instance}>
      <AppShell pathname={pathname} role={role} {...(user ? { user } : {})}>
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

describe('AppShell — di bawah large (modal)', () => {
  beforeEach(() => setLarge(false));

  it('hides the drawer and exposes a menu button that opens it as a modal', async () => {
    renderShell();
    // Drawer navigation is not in the tree until opened.
    expect(screen.queryByRole('navigation', { name: 'Navigasi utama' })).not.toBeInTheDocument();
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

  it('closes the modal drawer with its own Tutup menu button', async () => {
    renderShell();
    await userEvent.click(screen.getByRole('button', { name: 'Buka menu navigasi' }));
    const dialog = await screen.findByRole('dialog', { name: 'Navigasi utama' });
    const close = within(dialog).getByRole('button', { name: 'Tutup menu' });
    expect(close.className).toContain('outline-inverse-primary!');
    await userEvent.click(close);
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

describe('AppShell — large (drawer standar)', () => {
  beforeEach(() => setLarge(true));

  it('shows a permanent drawer and no menu button', () => {
    renderShell();
    expect(screen.getByRole('navigation', { name: 'Navigasi utama' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Buka menu navigasi' })).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Tutup menu' })).not.toBeInTheDocument();
  });
});

describe('AppShell — structure and a11y', () => {
  beforeEach(() => setLarge(true));

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
    const drawer = screen.getByRole('complementary', { name: 'Navigasi utama' });
    expect(within(drawer).getByText('Pemantauan')).toBeInTheDocument();
    expect(within(drawer).getByText('Konfigurasi')).toBeInTheDocument();
  });
});

describe('activeHref', () => {
  it('picks the longest matching prefix', () => {
    expect(activeHref('/provider/123')).toBe('/provider');
    expect(activeHref('/provider')).toBe('/provider');
    expect(activeHref('/')).toBe('/');
    expect(activeHref('/log/TX-1')).toBe('/');
    expect(activeHref('/client/c1/kredensial')).toBe('/client');
    expect(activeHref('/tidak-ada')).toBeUndefined();
  });
});

describe('AppShell — menu per role', () => {
  beforeEach(() => setLarge(true));

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

  it('operator has no configuration, client, NIK search or access audit', () => {
    renderShell('/', 'operator');
    expect(screen.getByRole('link', { name: 'Kuota' })).toBeInTheDocument();
    for (const name of ['Secret', 'Upstream pool', 'Client dan kredensial', 'Audit akses']) {
      expect(screen.queryByRole('link', { name })).not.toBeInTheDocument();
    }
    expect(screen.queryByRole('link', { name: 'Pencarian log per NIK' })).not.toBeInTheDocument();
  });

  it('matches the Penpot frames exactly: groups and items per role, in order', () => {
    const shape = (role: AdminRole) =>
      navGroupsFor(role).map((g) => [g.id, g.items.map((i) => i.href)]);
    expect(shape('platform_admin')).toEqual([
      ['monitoring', ['/kuota']],
      ['config', ['/provider', '/endpoint', '/secret', '/upstream', '/downstream']],
      ['client', ['/client']],
      ['account', ['/settings']],
    ]);
    expect(shape('operator')).toEqual([
      ['monitoring', ['/', '/kuota']],
      ['account', ['/settings']],
    ]);
    expect(shape('auditor')).toEqual([
      ['monitoring', ['/']],
      ['compliance', ['/audit', '/nik']],
      ['account', ['/settings']],
    ]);
    // client_owner has no frame: only what its screens allow.
    expect(shape('client_owner')).toEqual([
      ['client', ['/client']],
      ['account', ['/settings']],
    ]);
  });

  it('shows group titles and items in the order of the frame for platform admin', () => {
    renderShell('/kuota', 'platform_admin');
    const drawer = screen.getByRole('complementary', { name: 'Navigasi utama' });
    const text = drawer.textContent ?? '';
    const order = [
      'Pemantauan',
      'Kuota',
      'Konfigurasi',
      'Provider',
      'Secret',
      'Upstream pool',
      'Downstream pool',
      'Client',
      'Client dan kredensial',
      'Akun',
      'Pengaturan',
    ];
    const at = order.map((label) => text.indexOf(label));
    expect(at.every((i) => i >= 0)).toBe(true);
    expect([...at].sort((x, y) => x - y)).toEqual(at);
    expect(within(drawer).queryByRole('link', { name: 'Log transaksi' })).not.toBeInTheDocument();
  });

  it('uses the inverse surface for the drawer', () => {
    renderShell('/', 'auditor');
    expect(screen.getByRole('complementary', { name: 'Navigasi utama' }).className).toContain(
      'bg-inverse-surface',
    );
  });

  it('marks the active item with a 12 % inverse-on-surface indicator and an inverse-primary icon', () => {
    renderShell('/audit', 'auditor');
    const active = screen.getByRole('link', { name: 'Audit akses' });
    expect(active.className).toContain('bg-inverse-on-surface/12');
    expect(active.querySelector('svg')?.getAttribute('class')).toContain('text-inverse-primary');
    const other = screen.getByRole('link', { name: 'Log transaksi' });
    expect(other.className).not.toContain('bg-inverse-on-surface/12');
    // Tailwind 4: outline-none mematikan style, jadi ring fokus butuh outline-solid.
    expect(active.className).toContain('focus-visible:outline-solid');
    expect(active.className).toContain('focus-visible:outline-3');
    expect(other.querySelector('svg')).not.toBeNull();
  });

  it('shows the signed-in identity at the bottom of the drawer, not in the top bar', () => {
    renderShell('/', 'operator', { username: 'adm.contoh', peran: 'operator' });
    const drawer = screen.getByRole('complementary', { name: 'Navigasi utama' });
    expect(within(drawer).getByText('adm.contoh')).toBeInTheDocument();
    expect(within(drawer).getByText('Operator')).toBeInTheDocument();
    expect(within(screen.getByRole('banner')).queryByText(/adm\.contoh/)).not.toBeInTheDocument();
  });

  it('puts the product name in the drawer and the environment label in the top bar', () => {
    renderShell('/', 'operator');
    const drawer = screen.getByRole('complementary', { name: 'Navigasi utama' });
    expect(within(drawer).getByText('PoolRouter')).toBeInTheDocument();
    const banner = screen.getByRole('banner');
    expect(within(banner).getByText('PoC')).toBeInTheDocument();
    expect(within(banner).queryByText('PoolRouter')).not.toBeInTheDocument();
  });

  it('lets the drawer links be reached with Tab', async () => {
    renderShell('/', 'auditor');
    await userEvent.tab(); // skip link
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Log transaksi' }));
  });
});

describe('AppShell — breadcrumbs', () => {
  beforeEach(() => setLarge(true));

  it('renders a trail ending at the current page', () => {
    renderShell('/kuota');
    const trail = within(screen.getByRole('banner')).getByRole('navigation', {
      name: 'Jejak halaman',
    });
    expect(trail).toHaveTextContent('Pemantauan');
    expect(within(trail).getByText('Kuota')).toHaveAttribute('aria-current', 'page');
  });

  it('links back to the list from a detail route', () => {
    renderShell('/provider/abc');
    const trail = screen.getByRole('navigation', { name: 'Jejak halaman' });
    expect(within(trail).getByRole('link', { name: 'Provider' })).toHaveAttribute(
      'href',
      '/provider',
    );
    expect(within(trail).getByText('abc')).toHaveAttribute('aria-current', 'page');
    // The group is plain text, not a link (Shell-detail).
    expect(within(trail).queryByRole('link', { name: 'Konfigurasi' })).not.toBeInTheDocument();
    expect(trail.querySelectorAll('[aria-hidden="true"]')).toHaveLength(2);
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
    expect(crumbsFor('/upstream/baru').at(-1)).toEqual({ labelKey: 'nav.breadcrumb.new' });
    expect(crumbsFor('/endpoint').map((c) => c.labelKey)).toEqual([
      'nav.group.config',
      'nav.drawer.endpoint',
    ]);
    expect(crumbsFor('/endpoint/e1')).toEqual([
      { labelKey: 'nav.group.config' },
      { labelKey: 'nav.drawer.endpoint', href: '/endpoint' },
      { text: 'e1' },
    ]);
    expect(crumbsFor('/endpoint/baru').at(-1)).toEqual({ labelKey: 'nav.breadcrumb.new' });
    expect(crumbsFor('/downstream/d1')).toEqual([
      { labelKey: 'nav.group.config' },
      { labelKey: 'nav.drawer.downstream', href: '/downstream' },
      { text: 'd1' },
    ]);
    expect(crumbsFor('/client/c1')).toEqual([
      { labelKey: 'nav.group.client' },
      { labelKey: 'nav.drawer.client', href: '/client' },
      { text: 'c1' },
    ]);
    expect(crumbsFor('/client/c1/kredensial')).toEqual([
      { labelKey: 'nav.group.client' },
      { labelKey: 'nav.drawer.client', href: '/client' },
      { text: 'c1', href: '/client/c1' },
      { labelKey: 'nav.breadcrumb.credentials' },
    ]);
    expect(crumbsFor('/provider/a/webhook')).toEqual([{ labelKey: 'nav.breadcrumb.not_found' }]);
    // Segmen berformat salah tidak boleh melempar URIError (akar tidak boleh unmount).
    for (const path of ['/provider/%/kredensial', '/upstream/100%/webhook', '/log/%zz/webhook']) {
      expect(crumbsFor(path), path).toEqual([{ labelKey: 'nav.breadcrumb.not_found' }]);
    }
    expect(crumbsFor('/provider/%zz').at(-1)).toEqual({ text: '%zz' });
    expect(crumbsFor('/client/100%/kredensial').at(1)).toEqual({
      labelKey: 'nav.drawer.client',
      href: '/client',
    });
    expect(crumbsFor('/x')).toEqual([{ labelKey: 'nav.breadcrumb.not_found' }]);
  });
});
