import { describe, expect, it, vi } from 'vitest';
import { ApiError, createApiClient } from '../src/api/client';

function jsonResponse(body: unknown, status = 200, headers: Record<string, string> = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  });
}

function client(fetchImpl: ReturnType<typeof vi.fn>) {
  return createApiClient({ fetch: fetchImpl as never });
}

/** Destructure a recorded fetch call; the assertion helpers treat a missing call as a failure. */
function firstCall(fetchImpl: ReturnType<typeof vi.fn>): [string, RequestInit] {
  const call = fetchImpl.mock.calls[0] as [string, RequestInit] | undefined;
  if (!call) throw new Error('fetch was not called');
  return call;
}

describe('admin API client', () => {
  it('POSTs the log search so filters never reach the access log', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({ baris: [] }));
    await client(fetchImpl).cariLog({
      dari: '2026-11-02T00:00:00+07:00',
      sampai: '2026-11-03T00:00:00+07:00',
    });

    const [url, init] = firstCall(fetchImpl);
    expect(url).toBe('/admin/v1/log/cari');
    expect(url).not.toContain('?');
    expect(init.method).toBe('POST');
    const body = JSON.parse(String(init.body));
    expect(body.dari).toBe('2026-11-02T00:00:00+07:00');
  });

  it('sends the session cookie and asks intermediaries not to cache (R-16)', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({ baris: [] }));
    await client(fetchImpl).cariLog({ dari: 'a', sampai: 'b' });

    const [, init] = firstCall(fetchImpl);
    expect(init.credentials).toBe('include');
    expect(init.cache).toBe('no-store');
    expect(new Headers(init.headers).get('Cache-Control')).toBe('no-store');
  });

  it('carries the auditor reason in a header, never in the URL', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({ baris: [] }));
    await client(fetchImpl).cariLogNik(
      { nik: '3175000000000001', dari: 'a', sampai: 'b', alasan: 'Audit aduan W-2026-07' },
      { alasan: 'Audit aduan W-2026-07' },
    );

    const [url, init] = firstCall(fetchImpl);
    expect(url).toBe('/admin/v1/auditor/log/cari-nik');
    expect(url).not.toContain('3175');
    expect(url).not.toContain('Audit');
    expect(new Headers(init.headers).get('X-Alasan')).toBe('Audit aduan W-2026-07');
  });

  it('surfaces the server problem code so screens can pick catalog copy', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(jsonResponse({ status: 403, kode: 'pencarian_nik_dilarang' }, 403));

    await expect(client(fetchImpl).cariLog({ dari: 'a', sampai: 'b' })).rejects.toMatchObject({
      status: 403,
      kode: 'pencarian_nik_dilarang',
    });
  });

  it('falls back to a synthetic code when the body is not a problem detail', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response('<html>', { status: 502 }));

    await expect(client(fetchImpl).kuota()).rejects.toSatisfy(
      (error: unknown) => error instanceof ApiError && error.kode === 'http_502',
    );
  });

  it('reads the export filename and digest from the response headers', async () => {
    const headers = new Headers({ 'Content-Type': 'text/csv' });
    headers.set('Content-Disposition', "attachment; filename*=UTF-8''ekspor-log-nik.csv");
    headers.set('Repr-Digest', 'sha-256=:abc123:');
    // jsdom's Response only reads string bodies into `blob()`; real responses stream bytes, so a
    // string keeps the test honest about what the client parses.
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(new Response('nik,kode\n', { status: 200, headers }));

    const file = await client(fetchImpl).eksporLog({
      nik: '3175000000000001',
      dari: 'a',
      sampai: 'b',
      alasan: 'Ekspor audit Q4 2026',
    });

    expect(file.filename).toBe('ekspor-log-nik.csv');
    expect(file.digest).toBe(':abc123:');
    expect(await file.blob.text()).toContain('nik,kode');
  });

  it('keeps the JSON body readable: an oversized export arrives as a 422 code, not a file', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(jsonResponse({ status: 422, kode: 'ekspor_terlalu_besar' }, 422));

    await expect(
      client(fetchImpl).eksporLog({ nik: '1', dari: 'a', sampai: 'b', alasan: 'x' }),
    ).rejects.toMatchObject({ status: 422, kode: 'ekspor_terlalu_besar' });
  });
});
