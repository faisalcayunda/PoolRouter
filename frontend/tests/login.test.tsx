import { describe, expect, it, vi } from 'vitest';
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError, type ApiClient } from '../src/api/client';
import { LoginScreen } from '../src/features/auth/LoginScreen';
import { renderWithI18n } from './render';

function stub(overrides: Partial<ApiClient> = {}): ApiClient {
  return { login: vi.fn().mockResolvedValue(undefined), ...overrides } as unknown as ApiClient;
}

describe('S01 masuk', () => {
  it('sends the username and password once, then leaves the form', async () => {
    const client = stub();
    const onBerhasil = vi.fn();
    renderWithI18n(<LoginScreen client={client} onBerhasil={onBerhasil} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), 'auditor_01');
    await userEvent.type(screen.getByLabelText('Kata sandi'), 'rahasia-panjang');
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(client.login).toHaveBeenCalledWith({ username: 'auditor_01', password: 'rahasia-panjang' });
    expect(onBerhasil).toHaveBeenCalledTimes(1);
  });

  it('maps a 401 to the invalid-credentials copy, not a network error', async () => {
    const client = stub({
      login: vi.fn().mockRejectedValue(new ApiError(401, { status: 401, kode: 'kredensial_salah' })),
    });
    renderWithI18n(<LoginScreen client={client} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), 'salah');
    await userEvent.type(screen.getByLabelText('Kata sandi'), 'salah');
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(
      await screen.findByText('Nama pengguna atau kata sandi tidak cocok.'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/koneksi/i)).not.toBeInTheDocument();
  });

  it('maps a throttled login to the 15-minute copy', async () => {
    const client = stub({
      login: vi.fn().mockRejectedValue(new ApiError(429, { status: 429, kode: 'login_dibatasi' })),
    });
    renderWithI18n(<LoginScreen client={client} />);

    await userEvent.type(screen.getByLabelText('Nama pengguna'), 'auditor_01');
    await userEvent.type(screen.getByLabelText('Kata sandi'), 'rahasia-panjang');
    await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));

    expect(
      await screen.findByText('Terlalu banyak percobaan. Coba lagi dalam 15 menit.'),
    ).toBeInTheDocument();
  });

  it('explains an expired session when the shell sent us here with alasan=sesi_berakhir', () => {
    renderWithI18n(<LoginScreen client={stub()} search="?alasan=sesi_berakhir" />);
    expect(
      screen.getByText('Sesi Anda berakhir. Masuk lagi untuk melanjutkan.'),
    ).toBeInTheDocument();
  });
});
