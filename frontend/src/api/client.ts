import type { components } from './schema.gen';

export type Schemas = components['schemas'];
export type LogCariRequest = Schemas['LogCariRequest'];
export type LogPage = Schemas['LogPage'];
export type LogBaris = Schemas['LogBaris'];
export type KuotaTracker = Schemas['KuotaTracker'];
export type NikCariRequest = Schemas['NikCariRequest'];
export type PayloadOpenRequest = Schemas['PayloadOpenRequest'];
export type PayloadView = Schemas['PayloadView'];
export type EksporRequest = Schemas['EksporRequest'];
export type AuditCariRequest = Schemas['AuditCariRequest'];
export type AuditAksesPage = Schemas['AuditAksesPage'];
export type AuditAksesBaris = Schemas['AuditAksesBaris'];
export type UpstreamPool = Schemas['UpstreamPool'];
export type ValidasiErrorAdmin = Schemas['ValidasiErrorAdmin'];

const ADMIN_PREFIX = '/admin/v1';

/** Problem detail (RFC 9457) the admin API returns on 4xx; the UI maps `kode` to catalog copy. */
export interface Problem {
  type?: string;
  title?: string;
  status: number;
  /** Stable machine code, e.g. `pencarian_nik_dilarang`, `ekspor_terlalu_besar`. */
  kode?: string;
  /** Field-level errors for 422 `validasi_gagal`. */
  errors?: ValidasiErrorAdmin[];
}

/**
 * Custom headers the admin API requires beyond the session cookie. `X-Alasan` carries the auditor
 * reason that lands in the access audit (ADR-0003 rev 3 3.6a–c); it never goes into the URL or the
 * query string (section 9: no NIK in the URL).
 */
export interface RequestOptions {
  /** Wire body for POST search endpoints; kept out of the URL on purpose. */
  body?: unknown;
  /** Auditor reason; sent as `X-Alasan` on every logged read. */
  alasan?: string;
  signal?: AbortSignal;
}

/** A CSV export with the digest the API returns for the downloaded file (R-18). */
export interface EksporBerkas {
  blob: Blob;
  filename: string;
  /** Base64 SHA-256 from `Repr-Digest: sha-256=...`, when the server sent it. */
  digest: string | null;
}

export class ApiError extends Error {
  readonly status: number;
  readonly problem: Problem | null;

  constructor(status: number, problem: Problem | null) {
    super(problem?.kode ?? problem?.title ?? `HTTP ${status}`);
    this.name = 'ApiError';
    this.status = status;
    this.problem = problem;
  }

  /** Stable code from the problem body, or a synthetic `http_<status>` when the body had none. */
  get kode(): string {
    return this.problem?.kode ?? `http_${this.status}`;
  }
}

/**
 * The API client is a plain function so tests can pass a stub and the app can swap the base URL
 * via `VITE_ADMIN_API_BASE` without touching call sites.
 */
export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

export interface ApiClientOptions {
  /** Same-origin by default; the admin API is served under `/admin/v1`. */
  baseUrl?: string;
  fetch?: Fetch;
}

export interface ApiClient {
  /** POST /log/cari — POST so filters never reach the access log (section 18.3). */
  cariLog(body: LogCariRequest, options?: RequestOptions): Promise<LogPage>;
  /** POST /auditor/log/cari-nik — auditor only, exact 16-digit NIK. */
  cariLogNik(body: NikCariRequest, options?: RequestOptions): Promise<LogPage>;
  /** POST /auditor/payload — one `lihat_payload` audit row per opening. */
  bukaPayload(body: PayloadOpenRequest, options?: RequestOptions): Promise<PayloadView>;
  /** POST /auditor/log/ekspor — CSV; above 10,000 rows a 422 arrives instead of a file. */
  eksporLog(body: EksporRequest, options?: RequestOptions): Promise<EksporBerkas>;
  /** POST /auditor/audit-akses/cari — auditor only, range max 31 days. */
  cariAuditAkses(body: AuditCariRequest, options?: RequestOptions): Promise<AuditAksesPage>;
  /** GET /kuota — read-only quota tracker rows from the DB (US-12, KP-07). */
  kuota(upstreamPoolId?: string, options?: RequestOptions): Promise<KuotaTracker[]>;
  /** GET /upstream-pools — read-only lookup for the quota tracker filter. */
  upstreamPools(options?: RequestOptions): Promise<UpstreamPool[]>;
}

function problemFrom(status: number, body: unknown): Problem | null {
  if (body && typeof body === 'object') {
    const obj = body as Record<string, unknown>;
    return {
      type: typeof obj.type === 'string' ? obj.type : undefined,
      title: typeof obj.title === 'string' ? obj.title : undefined,
      status,
      kode: typeof obj.kode === 'string' ? obj.kode : undefined,
      errors: Array.isArray(obj.errors) ? (obj.errors as ValidasiErrorAdmin[]) : undefined,
    };
  }
  return null;
}

function filenameFrom(disposition: string | null): string {
  if (!disposition) return 'ekspor-log.csv';
  const utf8 = /filename\*=UTF-8''([^;]+)/i.exec(disposition);
  if (utf8?.[1]) return decodeURIComponent(utf8[1]);
  const plain = /filename="?([^";]+)"?/i.exec(disposition);
  return plain?.[1] ?? 'ekspor-log.csv';
}

function digestFrom(header: string | null): string | null {
  if (!header) return null;
  const match = /sha-256=([^,\s]+)/i.exec(header);
  return match?.[1] ?? null;
}

export function createApiClient(options: ApiClientOptions = {}): ApiClient {
  const baseUrl = options.baseUrl ?? ADMIN_PREFIX;
  const doFetch: Fetch = options.fetch ?? ((input, init) => globalThis.fetch(input, init));

  function buildInit(init: RequestInit, opts: RequestOptions): RequestInit {
    const headers = new Headers(init.headers);
    headers.set('Accept', 'application/json');
    // R-16: every admin read is `no-store`; ask intermediaries not to reuse it either.
    headers.set('Cache-Control', 'no-store');
    if (opts.body !== undefined) headers.set('Content-Type', 'application/json');
    if (opts.alasan !== undefined) headers.set('X-Alasan', opts.alasan);

    return {
      ...init,
      headers,
      // The session cookie is `HttpOnly` + `SameSite=Strict` (ADR-0003 rev 5 3.6 option A), so
      // same-origin `include` keeps it flowing without exposing the token to JavaScript.
      credentials: 'include',
      cache: 'no-store',
      signal: opts.signal,
      ...(opts.body !== undefined ? { body: JSON.stringify(opts.body) } : {}),
    };
  }

  async function request<T>(
    path: string,
    init: RequestInit,
    opts: RequestOptions = {},
  ): Promise<T> {
    const response = await doFetch(`${baseUrl}${path}`, buildInit(init, opts));
    const text = await response.text();
    const parsed: unknown = text ? safeJson(text) : undefined;
    if (!response.ok) throw new ApiError(response.status, problemFrom(response.status, parsed));
    return parsed as T;
  }

  return {
    cariLog: (body, opts) => request<LogPage>('/log/cari', { method: 'POST' }, { ...opts, body }),

    cariLogNik: (body, opts) =>
      request<LogPage>('/auditor/log/cari-nik', { method: 'POST' }, { ...opts, body }),

    bukaPayload: async (body, opts) => {
      const response = await doFetch(
        `${baseUrl}/auditor/payload`,
        buildInit({ method: 'POST' }, { ...opts, body }),
      );
      const text = await response.text();
      const parsed: unknown = text ? safeJson(text) : undefined;
      if (!response.ok) throw new ApiError(response.status, problemFrom(response.status, parsed));
      // R-16 / section 18.3: the payload is only safe to render if the *answer* is uncacheable.
      // The request header alone does not bind an intermediary, so a payload without it is a
      // contract breach and must not reach the screen.
      const cacheControl = response.headers.get('Cache-Control') ?? '';
      if (!/\bno-store\b/i.test(cacheControl)) {
        throw new ApiError(502, { status: 502, kode: 'payload_bukan_no_store' });
      }
      return parsed as PayloadView;
    },

    async eksporLog(body, opts = {}) {
      const response = await doFetch(
        `${baseUrl}/auditor/log/ekspor`,
        buildInit({ method: 'POST' }, { ...opts, body }),
      );
      if (!response.ok) {
        const text = await response.text();
        throw new ApiError(
          response.status,
          problemFrom(response.status, text ? safeJson(text) : undefined),
        );
      }
      return {
        blob: await response.blob(),
        filename: filenameFrom(response.headers.get('Content-Disposition')),
        digest: digestFrom(response.headers.get('Repr-Digest')),
      };
    },

    cariAuditAkses: (body, opts) =>
      request<AuditAksesPage>('/auditor/audit-akses/cari', { method: 'POST' }, { ...opts, body }),

    kuota: (upstreamPoolId, opts) => {
      const query = upstreamPoolId ? `?upstream_pool_id=${encodeURIComponent(upstreamPoolId)}` : '';
      return request<KuotaTracker[]>(`/kuota${query}`, { method: 'GET' }, opts);
    },

    upstreamPools: (opts) => request<UpstreamPool[]>('/upstream-pools', { method: 'GET' }, opts),
  };
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

/** App-wide client (same-origin). Screens accept an override in props for tests. */
export const api = createApiClient();
