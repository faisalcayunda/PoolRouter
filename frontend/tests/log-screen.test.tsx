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
      queue_id: '0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f',
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

const DETAIL = 'Detail transaksi TRX-2026-0001';

describe('S11 log transaksi', () => {
  it('follows the Penpot frame: header, KPI placeholders, one toolbar, table, footer', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} />);
    const detail = await screen.findByRole('link', { name: DETAIL });

    // The h1 takes focus on open for screen readers but draws no frame, as in the design.
    const h1 = screen.getByRole('heading', { level: 1, name: 'Log transaksi' });
    expect(h1).toHaveFocus();
    expect(h1.className).not.toContain('m3-focus');
    expect(
      screen.getByText(
        'Telusuri lewat kode transaksi, queue_id, client, atau waktu. NIK ditampilkan 4 digit akhir.',
      ),
    ).toBeInTheDocument();

    // The API has no daily aggregates yet, so every KPI card is a placeholder, not a made-up number.
    const kpi = within(screen.getByRole('list'));
    for (const title of ['Transaksi hari ini', 'Berhasil', 'Gagal', 'Rata-rata respons']) {
      expect(kpi.getByText(title)).toBeInTheDocument();
    }
    expect(kpi.getAllByText('Menunggu API')).toHaveLength(4);

    // One search box, one client select, one date range, "Hapus filter" — and no "Cari" button.
    expect(screen.getByRole('textbox', { name: 'Cari' })).toHaveAttribute(
      'placeholder',
      'Cari kode transaksi atau queue_id',
    );
    expect(screen.getByRole('button', { name: /Client/ })).toHaveTextContent('Semua client');
    expect(screen.getAllByText('Rentang tanggal')).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Hapus filter' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Cari' })).not.toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: 'Dari' })).not.toBeInTheDocument();

    // Table columns in design order; the masked NIK is shown exactly as the server sent it.
    const table = within(screen.getByRole('table'));
    expect(table.getAllByRole('columnheader').map((h) => h.textContent)).toEqual([
      'Waktu',
      'Kode transaksi',
      'Client',
      'NIK',
      'Status akses',
      'Detail',
    ]);
    const cells = table.getAllByRole('cell').map((cell) => cell.textContent);
    expect(cells).toContain('3 Nov 2026, 14.30');
    expect(cells).toContain('TRX-…0001');
    expect(cells).toContain('Dukcapil Jabar');
    expect(cells).toContain('••••••••••••0001');
    expect(table.getByText('Berhasil')).toBeInTheDocument();
    expect(detail).toHaveAttribute('href', '/log/TRX-2026-0001');
    expect(detail).toHaveTextContent('Detail');

    // Footer: range text and two text buttons.
    const footer = within(screen.getByRole('navigation', { name: 'Halaman 1 dari 1' }));
    expect(footer.getByText('1–1 dari 1')).toBeInTheDocument();
    expect(footer.getByRole('button', { name: 'Sebelumnya' })).toBeDisabled();
    expect(footer.getByRole('button', { name: 'Berikutnya' })).toBeDisabled();
  });

  it('sends the day range as whole WIB days in the POST body', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    await screen.findByRole('link', { name: DETAIL });

    const body = vi.mocked(client.cariLog).mock.calls[0]![0];
    expect(body.dari).toMatch(/^\d{4}-\d{2}-\d{2}T00:00:00\+07:00$/);
    expect(body.sampai).toMatch(/^\d{4}-\d{2}-\d{2}T23:59:59\+07:00$/);
    expect(body.dari.slice(0, 10)).toBe(body.sampai.slice(0, 10));
  });

  it('searches on Enter; a transaction code and a queue_id go to their own fields', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(1));

    const search = screen.getByRole('textbox', { name: 'Cari' });
    await userEvent.type(search, 'TRX-2026-0001{Enter}');
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(2));
    const byCode = vi.mocked(client.cariLog).mock.calls[1]![0];
    expect(byCode.kode_transaksi).toBe('TRX-2026-0001');
    expect(byCode.queue_id).toBeUndefined();

    const queue = '0b19c3b4-7c5f-4e2f-9b3a-1a2b3c4d5e6f';
    await userEvent.clear(search);
    await userEvent.type(search, `${queue}{Enter}`);
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(3));
    const byQueue = vi.mocked(client.cariLog).mock.calls[2]![0];
    expect(byQueue.queue_id).toBe(queue);
    expect(byQueue.kode_transaksi).toBeUndefined();
  });

  it('re-reads the window when Enter is pressed again without changing anything', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(1));

    const search = screen.getByRole('textbox', { name: 'Cari' });
    await userEvent.type(search, '{Enter}');
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(2));
    await userEvent.type(search, '{Enter}');
    // Identical filters, but the snapshot may have moved on: the second Enter must ask again.
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(3));
  });

  it('filters on the code a deep link carries, not just fills the field', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} initialCode="TRX-2026-0001" />);

    await waitFor(() => expect(client.cariLog).toHaveBeenCalled());
    // The first request is the one the screen mounts with: `/log/<kode>` must already filter.
    const body = vi.mocked(client.cariLog).mock.calls[0]![0];
    expect(body.kode_transaksi).toBe('TRX-2026-0001');
    expect(screen.getByRole('textbox', { name: 'Cari' })).toHaveValue('TRX-2026-0001');
  });

  it('applies the client choice at once, shows its chip, and clears everything with Hapus filter', async () => {
    const client = stubClient();
    renderWithI18n(<LogPageScreen client={client} />);
    await screen.findByRole('link', { name: DETAIL });

    await userEvent.click(screen.getByRole('button', { name: /Client/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Dukcapil Jabar' }));

    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(2));
    expect(vi.mocked(client.cariLog).mock.calls[1]![0].client_id).toBe('c1');
    expect(screen.getByText('Client: Dukcapil Jabar')).toBeInTheDocument();

    await userEvent.type(screen.getByRole('textbox', { name: 'Cari' }), 'abc');
    // The chip's remove button shares the name; the toolbar button comes first in the DOM.
    await userEvent.click(screen.getAllByRole('button', { name: 'Hapus filter' })[0]!);
    await waitFor(() => expect(client.cariLog).toHaveBeenCalledTimes(3));
    const body = vi.mocked(client.cariLog).mock.calls[2]![0];
    expect(body.client_id).toBeUndefined();
    expect(body.kode_transaksi).toBeUndefined();
    expect(screen.getByRole('textbox', { name: 'Cari' })).toHaveValue('');
    expect(screen.queryByText('Client: Dukcapil Jabar')).not.toBeInTheDocument();
  });

  it('hides the payload column from a role that cannot open one and says why', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} canOpenPayload={false} />);
    await screen.findByRole('link', { name: DETAIL });

    expect(screen.queryByRole('columnheader', { name: 'Lihat payload' })).not.toBeInTheDocument();
    expect(screen.getByText('Pencarian per NIK hanya tersedia untuk auditor.')).toBeInTheDocument();
  });

  it('offers the payload action to an auditor, without the NIK hint', async () => {
    const onOpenPayload = vi.fn();
    renderWithI18n(
      <LogPageScreen client={stubClient()} canOpenPayload onOpenPayload={onOpenPayload} />,
    );
    await screen.findByRole('link', { name: DETAIL });

    const table = within(screen.getByRole('table'));
    await userEvent.click(table.getByRole('button', { name: 'Lihat payload — TRX-2026-0001' }));
    expect(onOpenPayload).toHaveBeenCalledWith(LOG.baris[0]);
    expect(
      screen.queryByText('Pencarian per NIK hanya tersedia untuk auditor.'),
    ).not.toBeInTheDocument();
  });

  it('derives the payload affordance from the auditor role, not only an explicit prop', async () => {
    renderWithI18n(
      <RoleProvider current="auditor">
        <LogPageScreen client={stubClient()} />
      </RoleProvider>,
    );
    await screen.findByRole('link', { name: DETAIL });

    expect(screen.getByRole('columnheader', { name: 'Lihat payload' })).toBeInTheDocument();
  });

  it('keeps the payload column from an operator signed in through the role context', async () => {
    renderWithI18n(
      <RoleProvider current="operator">
        <LogPageScreen client={stubClient()} />
      </RoleProvider>,
    );
    await screen.findByRole('link', { name: DETAIL });

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
    await screen.findByRole('link', { name: DETAIL });

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

    expect(await screen.findByText('Tidak ada log yang cocok')).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it('names the empty client filter "Semua client", never the framework default', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} />);
    await screen.findByRole('link', { name: DETAIL });

    // A select with no option matching its empty value printed the English "Select an item".
    const trigger = screen.getByRole('button', { name: /Client/ });
    expect(within(trigger).getByText('Semua client')).toBeInTheDocument();
    expect(screen.queryByText('Select an item')).not.toBeInTheDocument();
  });

  it('shows the request time once: no second column repeats requested_at', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} />);

    await screen.findByRole('link', { name: DETAIL });
    const table = within(screen.getByRole('table'));
    expect(table.getAllByRole('columnheader', { name: 'Waktu' })).toHaveLength(1);
    expect(table.queryByRole('columnheader', { name: 'Diminta pada' })).not.toBeInTheDocument();
  });
});
