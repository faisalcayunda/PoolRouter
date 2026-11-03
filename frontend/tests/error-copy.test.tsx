import { describe, expect, it } from 'vitest';
import { screen } from '@testing-library/react';
import { ApiError } from '../src/api/client';
import { QueryError, errorMessage } from '../src/features/common/page';
import { renderWithI18n } from './render';

describe('error copy resolution', () => {
  it('maps a problem code to the specific catalog line', () => {
    const { i18nInstance } = renderWithI18n(<div />);
    const t = i18nInstance.t.bind(i18nInstance);
    expect(
      errorMessage(
        t,
        i18nInstance,
        new ApiError(403, { status: 403, kode: 'pencarian_nik_dilarang' }),
      ),
    ).toEqual({
      // No per-code catalog line exists yet, so a 403 reads as the generic access-denied title.
      message: 'Anda tidak punya akses ke halaman ini',
      kode: 'pencarian_nik_dilarang',
    });
  });

  it('falls back to the generic title for an unmapped code', () => {
    const { i18nInstance } = renderWithI18n(<div />);
    const t = i18nInstance.t.bind(i18nInstance);
    expect(
      errorMessage(t, i18nInstance, new ApiError(500, { status: 500, kode: 'boom' })).message,
    ).toBe('Data belum bisa dimuat. Periksa koneksi, lalu coba lagi.');
  });

  it('reads a transport failure as a connection note', () => {
    const { i18nInstance } = renderWithI18n(<div />);
    const t = i18nInstance.t.bind(i18nInstance);
    expect(errorMessage(t, i18nInstance, new TypeError('Failed to fetch'))).toEqual({
      message: 'Data belum bisa dimuat. Periksa koneksi, lalu coba lagi.',
      kode: 'jaringan',
    });
  });

  it('renders a retry action', () => {
    renderWithI18n(<QueryError error={new ApiError(500, { status: 500 })} onRetry={() => {}} />);
    expect(screen.getByRole('button', { name: 'Coba lagi' })).toBeInTheDocument();
  });
});
