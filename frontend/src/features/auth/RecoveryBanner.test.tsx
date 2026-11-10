import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { renderWithI18n } from '../../../tests/render';
import type { Me } from '../../api/client';
import { KUNCI_LOGIN_PEMULIHAN, RecoveryBanner, type RecoveryBannerProps } from './RecoveryBanner';

function makeMe(sisa: number, wajib = true): Me {
  return {
    pengguna_id: '00000000-0000-0000-0000-000000000001',
    username: 'u',
    peran: ['platform_admin'],
    mfa: { wajib, aktif: true, sisa_kode_pemulihan: sisa },
  } as unknown as Me;
}

function renderBanner(sesi: Partial<RecoveryBannerProps['sesi']> & { me: Me | null }) {
  return renderWithI18n(<RecoveryBanner sesi={{ memuat: false, galat: false, ...sesi }} />);
}

beforeEach(() => sessionStorage.clear());

describe('RecoveryBanner (AC-32)', () => {
  it('tidak tampil saat memuat atau saat galat', () => {
    const { unmount } = renderBanner({ memuat: true, me: makeMe(2) });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    unmount();
    renderBanner({ galat: true, me: makeMe(2) });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('tidak tampil untuk peran tanpa kewajiban MFA', () => {
    renderBanner({ me: makeMe(2, false) });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('sisa 0: alert error yang diumumkan assertive', () => {
    renderBanner({ me: makeMe(0) });
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('Kode pemulihan Anda habis. Atur ulang MFA.');
    expect(alert).toHaveAttribute('aria-live', 'assertive');
    expect(screen.queryByRole('button', { name: 'Tutup' })).not.toBeInTheDocument();
  });

  it('sisa <= 3: peringatan dengan jumlah sisa; Tutup hanya menyembunyikan di sesi ini', async () => {
    const { unmount } = renderBanner({ me: makeMe(3) });
    expect(screen.getByRole('alert')).toHaveTextContent('tinggal 3');
    await userEvent.click(screen.getByRole('button', { name: 'Tutup' }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    unmount();
    renderBanner({ me: makeMe(3) });
    expect(screen.getByRole('alert')).toBeInTheDocument();
  });

  it('sisa > 3 tanpa penanda login pemulihan: tidak tampil', () => {
    renderBanner({ me: makeMe(4) });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('sisa > 3 setelah login pakai kode pemulihan: info sekali, penanda dihapus', () => {
    sessionStorage.setItem(KUNCI_LOGIN_PEMULIHAN, '1');
    const { unmount } = renderBanner({ me: makeMe(9) });
    expect(screen.getByRole('alert')).toHaveTextContent('masuk menggunakan kode pemulihan');
    expect(sessionStorage.getItem(KUNCI_LOGIN_PEMULIHAN)).toBeNull();
    unmount();
    renderBanner({ me: makeMe(9) });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
