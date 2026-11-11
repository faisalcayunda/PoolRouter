import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { api } from '../src/api/client';
import { AppRoot } from '../src/app/AppRoot';
import { renderWithI18n } from './render';

/** Layar diganti stub supaya tes ini hanya menguji router (ARC-259), bukan isi layar. */
vi.mock('../src/app/routes', () => ({
  RoutedScreens: ({ pathname, search }: { pathname: string; search: string }) => (
    <p data-testid="layar">{`${pathname}${search}`}</p>
  ),
}));

const user = userEvent.setup({ delay: null });

describe('navigasi sisi klien AppRoot', () => {
  let me: ReturnType<typeof vi.spyOn>;
  let assign: ReturnType<typeof vi.fn>;
  const asli = window.location;

  beforeEach(() => {
    window.history.replaceState(null, '', '/nik');
    // Drawer standar (>= 1200 px) supaya link nav selalu ter-render; yang modal tertutup di jsdom.
    vi.spyOn(window, 'matchMedia').mockImplementation(
      (query) =>
        ({
          matches: true,
          media: query,
          addEventListener() {},
          removeEventListener() {},
        }) as unknown as MediaQueryList,
    );
    me = vi.spyOn(api, 'me').mockResolvedValue({
      pengguna_id: '00000000-0000-4000-8000-000000000001',
      username: 'a',
      peran: ['platform_admin'],
      mfa: { aktif: true, sisa_kode_pemulihan: 8, wajib: true },
    });
    vi.spyOn(api, 'getSesi').mockResolvedValue({
      idle_berakhir_pada: new Date(Date.now() + 3_600_000).toISOString(),
      absolut_berakhir_pada: new Date(Date.now() + 7_200_000).toISOString(),
      waktu_server: new Date().toISOString(),
    });
    assign = vi.fn();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: {
        ...asli,
        assign,
        get pathname() {
          return asli.pathname;
        },
        get search() {
          return asli.search;
        },
      },
    });
  });
  afterEach(() => {
    Object.defineProperty(window, 'location', { configurable: true, value: asli });
    vi.restoreAllMocks();
  });

  it('klik nav mengganti layar tanpa reload dan tanpa memuat ulang sesi', async () => {
    renderWithI18n(<AppRoot pathname="/nik" search="" />);
    expect(await screen.findByTestId('layar')).toHaveTextContent('/nik');

    await user.click(screen.getAllByRole('link', { name: /kuota/i })[0]!);

    await waitFor(() => expect(screen.getByTestId('layar')).toHaveTextContent('/kuota'));
    expect(window.location.pathname).toBe('/kuota');
    expect(assign).not.toHaveBeenCalled();
    expect(me).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/memuat/i)).not.toBeInTheDocument();
  });

  it('popstate (Back) mengganti layar', async () => {
    renderWithI18n(<AppRoot pathname="/nik" search="" />);
    await screen.findByTestId('layar');
    await user.click(screen.getAllByRole('link', { name: /kuota/i })[0]!);
    await waitFor(() => expect(screen.getByTestId('layar')).toHaveTextContent('/kuota'));

    await act(async () => {
      window.history.replaceState(null, '', '/audit');
      window.dispatchEvent(new PopStateEvent('popstate'));
    });

    await waitFor(() => expect(screen.getByTestId('layar')).toHaveTextContent('/audit'));
    expect(me).toHaveBeenCalledTimes(1);
  });
});
