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

/** Read the recorded request (openapi-fetch passes a `Request`) as URL path + init-like view. */
function firstCall(fetchImpl: ReturnType<typeof vi.fn>): [string, RequestInit] {
  const request = fetchImpl.mock.calls[0]?.[0] as Request | undefined;
  if (!request) throw new Error('fetch was not called');
  return [
    new URL(request.url).pathname,
    {
      method: request.method,
      headers: request.headers,
      cache: request.cache,
      credentials: request.credentials,
    },
  ];
}

/** JSON body of the recorded request; openapi-fetch serialises it into the Request stream. */
async function requestJson(fetchImpl: ReturnType<typeof vi.fn>): Promise<unknown> {
  const request = fetchImpl.mock.calls[0]?.[0] as Request | undefined;
  if (!request) throw new Error('fetch was not called');
  return request.clone().json();
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
    const body = (await requestJson(fetchImpl)) as { dari: string; sampai: string };
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

  it('opens a payload with no-store and the reason in a header (section 18.3)', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ input: {}, metadata: {}, hasil: {} }, 200, { 'Cache-Control': 'no-store' }),
      );
    await client(fetchImpl).bukaPayload(
      { kode_transaksi: 'TRX-2026-0042', alasan: 'Tindak lanjut aduan W-2026-09' },
      { alasan: 'Tindak lanjut aduan W-2026-09' },
    );

    const [url, init] = firstCall(fetchImpl);
    expect(url).toBe('/admin/v1/auditor/payload');
    expect(url).not.toContain('TRX-2026-0042');
    expect(url).not.toContain('Tindak');
    expect(init.method).toBe('POST');
    // The payload must never be reused from a cache, by us or by an intermediary.
    expect(init.cache).toBe('no-store');
    expect(new Headers(init.headers).get('Cache-Control')).toBe('no-store');
    expect(new Headers(init.headers).get('X-Alasan')).toBe('Tindak lanjut aduan W-2026-09');
  });

  it('fails a payload response that omits Cache-Control: no-store (R-16)', async () => {
    // The request asks for `no-store`, but the contract is only kept if the *response* says it
    // too: a caching intermediary that ignores the request header would otherwise store the
    // payload. An answer without it is a contract breach and must reject, not resolve.
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ input: { nik: '3175000000000001' }, metadata: {}, hasil: {} }),
      );

    await expect(
      client(fetchImpl).bukaPayload(
        { kode_transaksi: 'TRX-2026-0042', alasan: 'Tindak lanjut aduan W-2026-09' },
        { alasan: 'Tindak lanjut aduan W-2026-09' },
      ),
    ).rejects.toSatisfy(
      (error: unknown) => error instanceof ApiError && error.kode === 'payload_bukan_no_store',
    );
  });

  it('accepts a payload response that states no-store', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(
        jsonResponse(
          { input: { nik: '3175000000000001' }, metadata: {}, hasil: { status: 'ok' } },
          200,
          { 'Cache-Control': 'no-store' },
        ),
      );

    const view = await client(fetchImpl).bukaPayload(
      { kode_transaksi: 'TRX-2026-0042', alasan: 'Tindak lanjut aduan W-2026-09' },
      { alasan: 'Tindak lanjut aduan W-2026-09' },
    );
    expect(view.hasil).toEqual({ status: 'ok' });
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
  it('carries Retry-After from a throttled login as whole seconds', async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ status: 429, kode: 'login_dibatasi' }, 429, { 'Retry-After': '37' }),
      );

    await expect(
      createApiClient({ fetch: fetchImpl as never }).login({ username: 'a', password: 'b' }),
    ).rejects.toMatchObject({ status: 429, kode: 'login_dibatasi', retryAfterDetik: 37 });
  });

  it('reads an HTTP-date Retry-After and leaves it null when the header is absent', async () => {
    const sampai = new Date(Date.now() + 40_000).toUTCString();
    const bertanggal = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ status: 429, kode: 'login_dibatasi' }, 429, { 'Retry-After': sampai }),
      );
    const polos = vi
      .fn()
      .mockResolvedValue(jsonResponse({ status: 429, kode: 'login_dibatasi' }, 429));

    const errTanggal = await createApiClient({ fetch: bertanggal as never })
      .login({ username: 'a', password: 'b' })
      .catch((error: unknown) => error as ApiError);
    expect(errTanggal).toMatchObject({ status: 429 });
    expect((errTanggal as ApiError).retryAfterDetik).toBeGreaterThan(30);

    await expect(
      createApiClient({ fetch: polos as never }).login({ username: 'a', password: 'b' }),
    ).rejects.toMatchObject({ status: 429, retryAfterDetik: null });
  });
});

describe('admin API client: secret (ARC-101)', () => {
  it('PUTs the secret value in the body, never in the URL', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    await client(fetchImpl).putSecret('PASSWORD_UJI', 'ContohSecretSintetis');

    const [url, init] = firstCall(fetchImpl);
    expect(url).toBe('/admin/v1/secrets/PASSWORD_UJI');
    expect(url).not.toContain('ContohSecretSintetis');
    expect(init.method).toBe('PUT');
    expect(await requestJson(fetchImpl)).toEqual({ nilai: 'ContohSecretSintetis' });
  });

  it('keeps the 409 secret_dipakai detail for the blocked-delete message', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(
      jsonResponse(
        {
          type: 'x',
          title: 'Secret masih dipakai',
          status: 409,
          kode: 'secret_dipakai',
          detail: 'Masih dipakai 1 provider aktif.',
        },
        409,
      ),
    );

    const error = await client(fetchImpl)
      .deleteSecret('USER_ID')
      .catch((e: unknown) => e as ApiError);
    const [url, init] = firstCall(fetchImpl);
    expect(url).toBe('/admin/v1/secrets/USER_ID');
    expect(init.method).toBe('DELETE');
    expect(error).toMatchObject({ status: 409, kode: 'secret_dipakai' });
    expect((error as ApiError).problem?.detail).toBe('Masih dipakai 1 provider aktif.');
  });
});
