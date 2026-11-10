import { describe, expect, it, vi } from 'vitest';
import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError, type ApiClient } from '../src/api/client';
import { LoginScreen } from '../src/features/auth/LoginScreen';
import { renderWithI18n } from './render';

const FIXTURE_USER = 'a';
const FIXTURE_PASS = 'b';
const WRONG_USER = 'a';
const WRONG_PASS = 'b';

function stub(overrides: Partial<ApiClient> = {}): ApiClient {
  return { login: vi.fn().mockResolvedValue(undefined), ...overrides } as unknown as ApiClient;
}

describe('S01 masuk', () => {
  it('follows the Penpot S01 card: brand, env label, heading, subtitle, fields and help line', () => {
    renderWithI18n(<LoginScreen client={stub()} />);
    const main = within(screen.getByRole('main'));

    expect(main.getByText('PoolRouter')).toBeInTheDocument();
    expect(main.getByText('PoC · BGN')).toBeInTheDocument();
    expect(main.getByRole('separator')).toBeInTheDocument();
    const judul = main.getByRole('heading', { level: 1, name: 'Masuk ke akun' });
    // The heading carries the shared focus frame (it is focusable for skip-link targets).
    expect(judul).toHaveClass('m3-focus');
    expect(judul).toHaveAttribute('tabindex', '-1');
    expect(
      main.getByText('Gunakan kredensial yang diberikan oleh administrator Pusdatin.'),
    ).toBeInTheDocument();
    expect(main.getByLabelText('Nama pengguna')).toBeInTheDocument();
    expect(main.getByLabelText('Kata sandi')).toHaveAttribute('type', 'password');
    expect(main.getByRole('button', { name: 'Masuk' })).toHaveClass('w-full');
    expect(main.getByText('Butuh akses? Hubungi platform admin Pusdatin BGN.')).toBeInTheDocument();
  });

  it('sends the username and password once, then leaves the form', async () => {
    const client = stub();
    const onBerhasil = vi.fn();
    renderWithI18n(<LoginScreen client={client} onBerhasil={onBerhasil} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), FIXTURE_USER);
    await userEvent.type(screen.getByLabelText('Kata sandi'), FIXTURE_PASS);
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(client.login).toHaveBeenCalledWith({
      username: FIXTURE_USER,
      password: FIXTURE_PASS,
    });
    expect(onBerhasil).toHaveBeenCalledTimes(1);
  });

  it('maps a 401 to the invalid-credentials copy, not a network error', async () => {
    const client = stub({
      login: vi
        .fn()
        .mockRejectedValue(new ApiError(401, { status: 401, kode: 'kredensial_salah' })),
    });
    renderWithI18n(<LoginScreen client={client} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), WRONG_USER);
    await userEvent.type(screen.getByLabelText('Kata sandi'), WRONG_PASS);
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(
      await screen.findByText('Nama pengguna atau kata sandi tidak cocok.'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/koneksi/i)).not.toBeInTheDocument();
  });

  it('reports the Retry-After seconds a throttled login asked us to wait', async () => {
    const client = stub({
      login: vi
        .fn()
        .mockRejectedValue(new ApiError(429, { status: 429, kode: 'login_dibatasi' }, 37)),
    });
    renderWithI18n(<LoginScreen client={client} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), FIXTURE_USER);
    await userEvent.type(screen.getByLabelText('Kata sandi'), FIXTURE_PASS);
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    const pesan = await screen.findByRole('alert');
    expect(pesan).toHaveTextContent('Terlalu banyak percobaan. Coba lagi dalam 37 detik.');
    expect(pesan).toContainElement(
      screen.getByText('37 detik', { selector: '.font-semibold' }) as HTMLElement,
    );
  });

  it('falls back to the static throttle copy when the 429 carries no Retry-After', async () => {
    const client = stub({
      login: vi.fn().mockRejectedValue(new ApiError(429, { status: 429, kode: 'login_dibatasi' })),
    });
    renderWithI18n(<LoginScreen client={client} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), FIXTURE_USER);
    await userEvent.type(screen.getByLabelText('Kata sandi'), FIXTURE_PASS);
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(
      await screen.findByText('Terlalu banyak percobaan. Coba lagi dalam 15 menit.'),
    ).toBeInTheDocument();
  });

  it('shows the catalog network copy when the request never reaches the server', async () => {
    const client = stub({ login: vi.fn().mockRejectedValue(new TypeError('Failed to fetch')) });
    renderWithI18n(<LoginScreen client={client} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), FIXTURE_USER);
    await userEvent.type(screen.getByLabelText('Kata sandi'), FIXTURE_PASS);
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(
      await screen.findByText('Belum bisa terhubung. Periksa jaringan, lalu coba lagi.'),
    ).toBeInTheDocument();
  });

  it('explains an expired session when the shell sent us here with alasan=sesi_berakhir', () => {
    renderWithI18n(<LoginScreen client={stub()} search="?alasan=sesi_berakhir" />);
    expect(screen.getByText('Sesi Anda telah berakhir')).toBeInTheDocument();
    expect(screen.getByText('Sesi tidak aktif selama lebih dari batas waktu.')).toBeInTheDocument();
  });
});
