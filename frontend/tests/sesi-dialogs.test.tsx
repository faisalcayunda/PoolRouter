import { describe, expect, it, vi } from 'vitest';
import { fireEvent, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { SesiStatus } from '../src/api/client';
import { SesiDialogs } from '../src/features/auth/SesiDialogs';
import type { SesiState } from '../src/features/auth/useSesi';
import { renderWithI18n } from './render';

const SESI: SesiStatus = {
  idle_berakhir_pada: '2026-11-03T08:00:00.000Z',
  absolut_berakhir_pada: '2026-11-03T15:00:00.000Z',
  waktu_server: '2026-11-03T07:30:00.000Z',
};

function state(overrides: Partial<SesiState> = {}): SesiState {
  return {
    me: { username: 'auditor_01', peran: ['auditor'] } as SesiState['me'],
    sesi: SESI,
    fase: 'aman',
    memuat: false,
    galat: false,
    perpanjangGalat: false,
    detikIdle: 42,
    muatUlang: vi.fn(),
    perpanjang: vi.fn().mockResolvedValue(true),
    keluar: vi.fn(),
    ...overrides,
  };
}

describe('dialog sesi', () => {
  it('warns two minutes ahead as an alertdialog and shows the live countdown', () => {
    renderWithI18n(<SesiDialogs sesi={state({ fase: 'peringatan_idle' })} />);
    const dialog = screen.getByRole('alertdialog');
    expect(dialog).toHaveAccessibleName('Sesi akan berakhir');
    expect(dialog).toHaveAccessibleDescription(/42 detik/);
  });

  it('extends the session and confirms with a snackbar', async () => {
    const perpanjang = vi.fn().mockResolvedValue(true);
    renderWithI18n(<SesiDialogs sesi={state({ fase: 'peringatan_idle', perpanjang })} />);

    await userEvent.click(screen.getByRole('button', { name: 'Perpanjang sesi' }));

    expect(perpanjang).toHaveBeenCalledTimes(1);
    expect(await screen.findByText('Sesi diperpanjang')).toBeInTheDocument();
  });

  it('cannot be dismissed with Escape while idle (DV-14)', () => {
    renderWithI18n(<SesiDialogs sesi={state({ fase: 'peringatan_idle' })} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
  });

  it('states the absolute time, then leaves a banner after Mengerti', async () => {
    renderWithI18n(<SesiDialogs sesi={state({ fase: 'peringatan_absolut' })} />);
    const dialog = screen.getByRole('alertdialog');
    expect(dialog).toHaveAccessibleName('Sesi telah berakhir');
    // 15.00 UTC is 22.00 WIB.
    expect(dialog).toHaveAccessibleDescription(/22\.00 WIB/);

    await userEvent.click(screen.getByRole('button', { name: 'Mengerti' }));

    expect(await screen.findByText('Sesi Anda telah berakhir sejak 22.00 WIB')).toBeInTheDocument();
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('offers only Keluar and Perpanjang while idle', () => {
    renderWithI18n(<SesiDialogs sesi={state({ fase: 'peringatan_idle' })} />);
    const dialog = screen.getByRole('alertdialog');
    expect(dialog).toContainElement(screen.getByRole('button', { name: 'Keluar' }));
    expect(dialog).toContainElement(screen.getByRole('button', { name: 'Perpanjang sesi' }));
    expect(screen.queryByRole('button', { name: 'Tutup' })).not.toBeInTheDocument();
  });
});
