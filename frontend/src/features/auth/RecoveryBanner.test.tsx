import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createI18n } from '../../i18n/i18n';
import type { i18n } from 'i18next';
import { AppI18nProvider } from '../../i18n/provider';
import { RecoveryBanner } from './RecoveryBanner';
import type { SesiState } from './useSesi';
import type { Me } from '../../api/client';

let instance: i18n;

function makeSesi(overrides: Partial<SesiState> & { me?: Me | null }): SesiState {
  return {
    me: overrides.me ?? null,
    sesi: null,
    fase: 'aman',
    memuat: false,
    galat: false,
    perpanjangGalat: false,
    detikIdle: 0,
    muatUlang: () => {},
    perpanjang: async () => true,
    keluar: async () => {},
    ...overrides,
  } as SesiState;
}

function makeMe(sisa: number, wajib = true): Me {
  return {
    pengguna_id: '00000000-0000-0000-0000-000000000001',
    username: 'u',
    peran: ['platform_admin'],
    mfa: { wajib, aktif: true, sisa_kode_pemulihan: sisa },
  } as unknown as Me;
}

beforeEach(() => {
  instance = createI18n({ throwOnMissingKey: true });
  sessionStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

function renderBanner(sesi: SesiState) {
  return render(
    <AppI18nProvider i18n={instance}>
      <RecoveryBanner sesi={sesi} />
    </AppI18nProvider>,
  );
}

describe('RecoveryBanner (AC-32)', () => {
  it('tidak render saat loading', () => {
    renderBanner(makeSesi({ memuat: true, me: makeMe(2) }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('tidak render saat error', () => {
    renderBanner(makeSesi({ galat: true, me: makeMe(2) }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('tidak render untuk operator/client owner (wajib=false)', () => {
    renderBanner(makeSesi({ me: makeMe(2, false) }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('sisa=0 → alert error', () => {
    renderBanner(makeSesi({ me: makeMe(0) }));
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent(/habis/i);
  });

  it('sisa≤3 → alert warning + tombol Tutup; klik menutup sesi ini', async () => {
    renderBanner(makeSesi({ me: makeMe(2) }));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    const btn = screen.getByRole('button', { name: 'Tutup' });
    await userEvent.click(btn);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('sisa>3 tanpa flag recovery → tidak tampil', () => {
    renderBanner(makeSesi({ me: makeMe(5) }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('sisa>3 dengan flag recovery → info sekali, flag dihapus', () => {
    sessionStorage.setItem('pr_recovery_login', '1');
    renderBanner(makeSesi({ me: makeMe(5) }));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(sessionStorage.getItem('pr_recovery_login')).toBeNull();
  });
});
