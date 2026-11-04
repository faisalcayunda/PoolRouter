import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ApiClient, LogPage } from '../src/api/client';
import { ApiError } from '../src/api/client';
import { LogPageScreen } from '../src/features/log/LogPageScreen';
import { RoleProvider } from '../src/features/common/RoleContext';
import { renderWithI18n } from './render';

const LOG: LogPage = {
  baris: [
    {
      kode_transaksi: 'TRX-2026-0001',
      client_id: 'c1',
      queue_id: 'q1',
      requested_at: '2026-11-03T07:30:00.000Z',
      client_nama: 'Dukcapil Jabar',
      nik: '••••••••••••0001',
      status_akses: 'ok',
    },
  ],
  kursor: null,
  di_luar_retensi_online: false,
};

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    cariLog: vi.fn().mockResolvedValue(LOG),
    cariLogNik: vi.fn().mockResolvedValue(LOG),
    bukaPayload: vi.fn(),
    eksporLog: vi.fn(),
    cariAuditAkses: vi.fn(),
    kuota: vi.fn(),
    upstreamPools: vi.fn(),
    ...overrides,
  } as ApiClient;
}

describe('S11 log transaksi', () => {
  it('posts the search and renders the server-masked NIK', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    await screen.findByRole('link', { name: 'TRX-2026-0001' });
    // The window loads on mount; "Cari" on an untouched form re-reads it rather than reusing it.
    expect(client.cariLog).toHaveBeenCalledTimes(2);
    const body = vi.mocked(client.cariLog).mock.calls.at(-1)![0];
    // Both ends of the default 24-hour window travel in the body, not the URL.
    expect(body.dari).toMatch(/\+07:00$/);
    expect(body.sampai).toMatch(/\+07:00$/);
    // The operator sees the masked value exactly as sent; the screen never re-formats a NIK. The
    // trailing four digits sit in their own element so a narrow column cannot split them, hence
    // the check compares the cell's text rather than one text node.
    expect(screen.getAllByRole('cell').map((cell) => cell.textContent)).toContain(
      '••••••••••••0001',
    );
  });

  it('uses POST so the code filter stays out of the access log', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);

    await userEvent.type(screen.getByRole('textbox', { name: 'Kode transaksi' }), 'TRX-2026-0001');
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    // The initial window loads on mount; the code only appears in the body of the explicit search.
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(2));
    const body = vi.mocked(client.cariLog).mock.calls[1]![0];
    expect(body.kode_transaksi).toBe('TRX-2026-0001');
  });

  it('hides the payload column from a role that cannot open one', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} canOpenPayload={false} />);
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByRole('link', { name: 'TRX-2026-0001' });

    expect(screen.queryByRole('columnheader', { name: 'Lihat payload' })).not.toBeInTheDocument();
    expect(
      screen.getByText(
        'Pencarian per NIK hanya tersedia untuk auditor. Butuh NIK lengkap? Minta auditor.',
      ),
    ).toBeInTheDocument();
  });

  it('offers the payload action to an auditor', async () => {
    const onOpenPayload = vi.fn();
    renderWithI18n(
      <LogPageScreen client={stubClient()} canOpenPayload onOpenPayload={onOpenPayload} />,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByRole('link', { name: 'TRX-2026-0001' });

    await userEvent.click(screen.getByRole('button', { name: 'Lihat payload — TRX-2026-0001' }));
    expect(onOpenPayload).toHaveBeenCalledWith(LOG.baris[0]);
  });

  it('derives the payload affordance from the auditor role, not only an explicit prop', async () => {
    renderWithI18n(
      <RoleProvider current="auditor">
        <LogPageScreen client={stubClient()} />
      </RoleProvider>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByRole('link', { name: 'TRX-2026-0001' });

    expect(screen.getByRole('columnheader', { name: 'Lihat payload' })).toBeInTheDocument();
  });

  it('keeps the payload column from an operator signed in through the role context', async () => {
    renderWithI18n(
      <RoleProvider current="operator">
        <LogPageScreen client={stubClient()} />
      </RoleProvider>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByRole('link', { name: 'TRX-2026-0001' });

    expect(screen.queryByRole('columnheader', { name: 'Lihat payload' })).not.toBeInTheDocument();
  });

  it('opens the payload only after a reason is given on S11 (section 18.3)', async () => {
    const client = stubClient({
      bukaPayload: vi.fn().mockResolvedValue({
        kode_transaksi: 'TRX-2026-0001',
        diambil_pada: '2026-11-03T07:31:00.000Z',
        input: { nik: '3201010101010001' },
        metadata: { client_id: 'c1' },
        hasil: { status: 'ok' },
      }),
    });
    renderWithI18n(<LogPageScreen client={client} canOpenPayload />);
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByRole('link', { name: 'TRX-2026-0001' });

    await userEvent.click(screen.getByRole('button', { name: 'Lihat payload — TRX-2026-0001' }));

    // Nothing is fetched until a reason is submitted: the dialog itself is the guard.
    expect(client.bukaPayload).not.toHaveBeenCalled();
  });

  it('explains a rejected NIK filter instead of showing a generic failure', async () => {
    const client = stubClient({
      cariLog: vi
        .fn()
        .mockRejectedValue(new ApiError(403, { status: 403, kode: 'pencarian_nik_dilarang' })),
    });
    renderWithI18n(<LogPageScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    // A 403 is the only HTTP status the catalog words; it must not surface as a bare table.
    const errorAlert = (await screen.findAllByRole('alert')).find((node) =>
      within(node).queryByText('Anda tidak punya akses ke halaman ini'),
    );
    expect(errorAlert).toBeDefined();
    expect(within(errorAlert!).getByRole('button', { name: 'Coba lagi' })).toBeInTheDocument();
  });

  it('shows the empty state, not a blank table, when nothing matches', async () => {
    const client = stubClient({
      cariLog: vi
        .fn()
        .mockResolvedValue({ baris: [], kursor: null, di_luar_retensi_online: false }),
    });
    renderWithI18n(<LogPageScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    expect(await screen.findByText('Tidak ada log yang cocok')).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it('names the empty client filter "Semua", never the framework default', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} />);
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByRole('link', { name: 'TRX-2026-0001' });

    // A select with no option matching its empty value printed the English "Select an item".
    const trigger = screen.getByRole('button', { name: /Client/ });
    expect(within(trigger).getByText('Semua')).toBeInTheDocument();
    expect(screen.queryByText('Select an item')).not.toBeInTheDocument();
  });

  it('filters on the code a deep link carries, not just fills the field', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} initialCode="TRX-2026-0001" />);

    await waitFor(() => expect(client.cariLog).toHaveBeenCalled());
    // The first request is the one the screen mounts with: `/log/<kode>` must already filter.
    const body = vi.mocked(client.cariLog).mock.calls[0]![0];
    expect(body.kode_transaksi).toBe('TRX-2026-0001');
    expect(screen.getByRole('textbox', { name: 'Kode transaksi' })).toHaveValue('TRX-2026-0001');
  });

  it('re-reads the window when the operator searches again without changing anything', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(1));

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(2));
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    // Identical filters, but the snapshot may have moved on: the second press must ask again.
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(3));
  });

  it('reports a date that is not a real day instead of ignoring it', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    // The window the screen mounts with already ran once; a rejected range adds no request.
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(1));

    const start = screen.getByRole('textbox', { name: 'Dari' });
    await userEvent.clear(start);
    await userEvent.type(start, '31/02/2026');
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    expect(
      await screen.findByText('Tanggal belum sah. Tulis dengan format 03/11/2026.'),
    ).toBeInTheDocument();
    expect(start).toHaveAttribute('aria-invalid', 'true');
    expect(client.cariLog).toHaveBeenCalledTimes(1);
  });

  it('reports an end date before the start date as a pair error', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(1));

    const start = screen.getByRole('textbox', { name: 'Dari' });
    await userEvent.clear(start);
    await userEvent.type(start, '05/11/2026');
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    expect(
      await screen.findByText('Tanggal akhir harus sama dengan atau setelah tanggal awal.'),
    ).toBeInTheDocument();
    expect(client.cariLog).toHaveBeenCalledTimes(1);
  });
});
