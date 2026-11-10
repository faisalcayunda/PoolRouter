import { describe, expect, it, vi } from 'vitest';
import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError } from '../src/api/client';
import { InlineAlert } from '../src/components/m3/InlineAlert';
import { EmptyState, LoadingState, QueryError } from '../src/features/common/page';
import { Forbidden } from '../src/features/provider/Forbidden';
import { renderWithI18n } from './render';

// Pola state antarmuka (Penpot S16): loading, kosong, galat, sukses, 403, sesi berakhir.
describe('S16 state patterns', () => {
  it('loading: skeleton lines and a bar are hidden, the status line carries the label', () => {
    renderWithI18n(<LoadingState label="Memuat data transaksi…" />);
    const status = screen.getByRole('status');
    expect(within(status).getByText('Memuat data transaksi…')).toBeInTheDocument();
    expect(status.querySelectorAll('.m3-skeleton')).toHaveLength(4);
    expect(status.querySelector('[role="progressbar"]')).not.toBeNull();
  });

  it('empty: icon box, title, cause and an optional action, all in one block', () => {
    const { container } = renderWithI18n(
      <EmptyState
        title="Tidak ada log yang cocok"
        body="Ubah rentang waktu atau hapus filter untuk melihat log."
        action={<button type="button">Hapus filter</button>}
      />,
    );
    const blok = within(container);
    expect(blok.getByText('Tidak ada log yang cocok')).toBeInTheDocument();
    expect(
      blok.getByText('Ubah rentang waktu atau hapus filter untuk melihat log.'),
    ).toBeInTheDocument();
    expect(blok.getByRole('button', { name: 'Hapus filter' })).toBeInTheDocument();
    expect(container.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
  });

  it('error: the alert has a title and a cause, and "Coba lagi" is a filled button below it', async () => {
    const onRetry = vi.fn();
    const { container } = renderWithI18n(
      <QueryError error={new ApiError(500, { status: 500, kode: 'boom' })} onRetry={onRetry} />,
    );
    const alert = within(screen.getByRole('alert'));
    expect(alert.getByText('Data belum bisa dimuat')).toBeInTheDocument();
    expect(alert.getByText('Periksa koneksi jaringan dan coba lagi.')).toBeInTheDocument();
    expect(alert.queryByRole('button')).toBeNull();

    const coba = within(container).getByRole('button', { name: 'Coba lagi' });
    expect(coba).toHaveClass('bg-primary');
    await userEvent.click(coba);
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('success and info alerts use their own container tokens', () => {
    renderWithI18n(
      <>
        <InlineAlert tone="success" heading="Perubahan tersimpan">
          Rule baru berlaku untuk request baru.
        </InlineAlert>
        <InlineAlert tone="info" heading="Sesi Anda telah berakhir">
          Sesi tidak aktif selama lebih dari batas waktu.
        </InlineAlert>
      </>,
    );
    const [sukses, info] = screen.getAllByRole('alert');
    expect(sukses).toHaveClass('bg-success-container', 'rounded-medium');
    expect(within(sukses as HTMLElement).getByText('Perubahan tersimpan')).toBeInTheDocument();
    expect(info).toHaveClass('bg-info-container');
  });

  it('403: heading takes focus, cause is shown and "Ke halaman awal" links home', () => {
    const { container } = renderWithI18n(<Forbidden />);
    const blok = within(container);
    const judul = blok.getByRole('heading', { level: 1, name: 'Akses tidak diizinkan' });
    expect(judul).toHaveFocus();
    expect(
      blok.getByText('Percobaan ini dicatat. Hubungi platform admin bila butuh akses.'),
    ).toBeInTheDocument();
    expect(blok.getByRole('link', { name: 'Ke halaman awal' })).toHaveAttribute('href', '/');
  });
});
