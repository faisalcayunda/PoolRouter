import { describe, expect, it, vi } from 'vitest';
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError, type ApiClient } from '../src/api/client';
import { LoginScreen } from '../src/features/auth/LoginScreen';
import { renderWithI18n } from './render';

// Semua nilai sintetis dan jelas palsu; tidak ada kode TOTP atau secret nyata.
const SECRET = 'CONTOHSECRETSINTETISBUKANASLI234';
const URI = `otpauth://totp/PoolRouter:a?secret=${SECRET}&issuer=PoolRouter`;
const KODE_PEMULIHAN = Array.from(
  { length: 10 },
  (_, i) => `CONTOHPM${String(i).padStart(2, '0')}`,
);

function stub(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    login: vi.fn().mockResolvedValue('mfa'),
    verifikasiMfa: vi.fn().mockResolvedValue(undefined),
    daftarMfa: vi.fn().mockResolvedValue({ otpauth_uri: URI, rahasia_base32: SECRET }),
    konfirmasiMfa: vi.fn().mockResolvedValue({ kode_pemulihan: KODE_PEMULIHAN }),
    ...overrides,
  } as unknown as ApiClient;
}

async function masuk() {
  await userEvent.type(screen.getByLabelText('Nama pengguna'), 'a');
  await userEvent.type(screen.getByLabelText('Kata sandi'), 'b');
  await userEvent.click(screen.getByRole('button', { name: 'Masuk' }));
}

describe('login dua langkah (AC-29)', () => {
  it('shows the code screen after a password-only 200 and finishes with the TOTP code', async () => {
    const client = stub();
    const onBerhasil = vi.fn();
    renderWithI18n(<LoginScreen client={client} onBerhasil={onBerhasil} />);
    await masuk();

    expect(await screen.findByRole('heading', { name: 'Verifikasi dua langkah' })).toBeVisible();
    expect(onBerhasil).not.toHaveBeenCalled();
    await userEvent.type(screen.getByRole('textbox', { name: 'Kode authenticator' }), '123456');
    await userEvent.click(screen.getByRole('button', { name: 'Verifikasi' }));

    expect(client.verifikasiMfa).toHaveBeenCalledWith({ kode: '123456' });
    expect(onBerhasil).toHaveBeenCalledTimes(1);
  });

  it('sends a recovery code under kode_pemulihan, not kode', async () => {
    const client = stub();
    renderWithI18n(<LoginScreen client={client} onBerhasil={vi.fn()} />);
    await masuk();
    await userEvent.click(await screen.findByRole('button', { name: 'Pakai kode pemulihan' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Kode pemulihan' }), 'contoh-pm00');
    await userEvent.click(screen.getByRole('button', { name: 'Verifikasi' }));

    expect(client.verifikasiMfa).toHaveBeenCalledWith({ kode_pemulihan: 'CONTOHPM00' });
  });

  it('keeps the user on the code screen with an error on a 401, and the back link returns to login', async () => {
    const client = stub({
      verifikasiMfa: vi
        .fn()
        .mockRejectedValue(new ApiError(401, { status: 401, kode: 'kredensial_salah' })),
    });
    const onBerhasil = vi.fn();
    renderWithI18n(<LoginScreen client={client} onBerhasil={onBerhasil} />);
    await masuk();
    await userEvent.type(
      await screen.findByRole('textbox', { name: 'Kode authenticator' }),
      '000000',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Verifikasi' }));

    expect(await screen.findByText(/Kode tidak cocok atau sesi sudah habis/)).toBeInTheDocument();
    expect(onBerhasil).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: 'Kembali ke login' }));
    expect(screen.getByRole('heading', { name: 'Masuk ke akun' })).toBeVisible();
    // Kata sandi tidak dibawa kembali ke form.
    expect(screen.getByLabelText('Kata sandi')).toHaveValue('');
  });

  it('maps a 429 on the code step to the throttle copy', async () => {
    const client = stub({
      verifikasiMfa: vi
        .fn()
        .mockRejectedValue(new ApiError(429, { status: 429, kode: 'login_dibatasi' }, 30)),
    });
    renderWithI18n(<LoginScreen client={client} onBerhasil={vi.fn()} />);
    await masuk();
    await userEvent.type(
      await screen.findByRole('textbox', { name: 'Kode authenticator' }),
      '000000',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Verifikasi' }));

    expect(
      await screen.findByText('Terlalu banyak percobaan. Coba lagi beberapa saat lagi.'),
    ).toBeInTheDocument();
  });
});

describe('pendaftaran MFA dan kode pemulihan (AC-30/31)', () => {
  async function sampaiQr(client: ApiClient, onBerhasil = vi.fn()) {
    renderWithI18n(<LoginScreen client={client} onBerhasil={onBerhasil} />);
    await masuk();
    await userEvent.type(
      await screen.findByRole('textbox', { name: 'Kode pendaftaran' }),
      'contoh-daftar',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Lanjut' }));
    await screen.findByRole('heading', { name: 'Pindai kode QR' });
    return onBerhasil;
  }

  it('walks registration code, QR, confirmation and the recovery dialog before leaving', async () => {
    const client = stub({ login: vi.fn().mockResolvedValue('daftar_mfa') });
    const onBerhasil = await sampaiQr(client);

    expect(client.daftarMfa).toHaveBeenCalledWith({ kode_pendaftaran: 'CONTOHDAFTAR' });
    expect(screen.getByText('CONT OHSE CRET SINT ETIS BUKA NASL I234')).toBeInTheDocument();
    await userEvent.type(
      screen.getByRole('textbox', { name: 'Kode dari authenticator' }),
      '123456',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Konfirmasi' }));

    expect(client.konfirmasiMfa).toHaveBeenCalledWith({ kode_totp: '123456' });
    const dialog = await screen.findByRole('alertdialog', { name: 'Simpan kode pemulihan' });
    expect(dialog).toHaveTextContent('CONTOHPM00');
    expect(dialog).toHaveTextContent('CONTOHPM09');
    // Rahasia pendaftaran sudah dibuang dari layar begitu kode pemulihan tampil.
    expect(screen.queryByText('CONT OHSE CRET SINT ETIS BUKA NASL I234')).not.toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Pindai kode QR' })).not.toBeInTheDocument();
    expect(onBerhasil).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: 'Sudah saya simpan' }));
    expect(onBerhasil).toHaveBeenCalledTimes(1);
    expect(screen.queryByText('CONTOHPM00')).not.toBeInTheDocument();
  });

  it('stays on the registration code screen with a specific error on a 401', async () => {
    const client = stub({
      login: vi.fn().mockResolvedValue('daftar_mfa'),
      daftarMfa: vi
        .fn()
        .mockRejectedValue(new ApiError(401, { status: 401, kode: 'kredensial_salah' })),
    });
    renderWithI18n(<LoginScreen client={client} onBerhasil={vi.fn()} />);
    await masuk();
    await userEvent.type(await screen.findByRole('textbox', { name: 'Kode pendaftaran' }), 'salah');
    await userEvent.click(screen.getByRole('button', { name: 'Lanjut' }));

    expect(await screen.findByText(/Kode pendaftaran tidak cocok/)).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Pindai kode QR' })).not.toBeInTheDocument();
  });

  it('shows a confirmation error on the QR screen without dropping the setup key', async () => {
    const client = stub({
      login: vi.fn().mockResolvedValue('daftar_mfa'),
      konfirmasiMfa: vi.fn().mockRejectedValue(new ApiError(500, null)),
    });
    await sampaiQr(client);
    await userEvent.type(
      screen.getByRole('textbox', { name: 'Kode dari authenticator' }),
      '123456',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Konfirmasi' }));

    expect(await screen.findByText(/Belum bisa terhubung/)).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Pindai kode QR' })).toBeVisible();
  });
});
