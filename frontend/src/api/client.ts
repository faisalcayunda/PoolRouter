import createClient from 'openapi-fetch';
import type { components, paths } from './schema.gen';

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
export type LoginRequest = Schemas['LoginRequest'];
export type Me = Schemas['Me'];
export type SesiStatus = Schemas['SesiStatus'];
export type Provider = Schemas['Provider'];
export type ProviderConfig = Schemas['ProviderConfig'];

const ADMIN_PREFIX = '/admin/v1';

/** Problem detail (RFC 9457) the admin API returns on 4xx; the UI maps `kode` to catalog copy. */
export interface Problem {
  type?: string;
  title?: string;
  status?: number;
  kode?: string;
  detail?: string | null;
  errors?: ValidasiErrorAdmin[];
}

export interface EksporBerkas {
  blob: Blob;
  filename: string;
  /** `Repr-Digest` sha-256, when the server sent one. */
  digest: string | null;
}

export class ApiError extends Error {
  readonly status: number;
  readonly problem: Problem | null;
  /** Seconds from the `Retry-After` header, when the server sent one (ADR-0003 3.6 throttle). */
  readonly retryAfterDetik: number | null;

  constructor(status: number, problem: Problem | null, retryAfterDetik: number | null = null) {
    super(problem?.kode ?? problem?.title ?? `HTTP ${status}`);
    this.name = 'ApiError';
    this.status = status;
    this.problem = problem;
    this.retryAfterDetik = retryAfterDetik;
  }

  /** Stable code from the problem body, or a synthetic `http_<status>` when the body had none. */
  get kode(): string {
    return this.problem?.kode ?? `http_${this.status}`;
  }
}

/**
 * The API client is a typed `openapi-fetch` client over `schema.gen.ts` (ADR-0003 4). Tests pass a
 * fetch stub and the app can swap the API origin via `VITE_ADMIN_API_B`.
 */
export type Fetch = (input: Request) => Promise<Response>;

export interface ApiClientOptions {
  /** Origin only (`https://host`); the `/admin/v1` prefix lives in the operation paths. */
  origin?: string;
  fetch?: Fetch;
}

export interface RequestOptions {
  signal?: AbortSignal;
  /** Auditor reason (ADR-0004): travels in `X-Alasan`, never in the URL. */
  alasan?: string;
}

export interface ApiClient {
  /** POST /auth/login — success sets the `__Host-pr_session` cookie. */
  login(body: LoginRequest, options?: RequestOptions): Promise<void>;
  /** POST /auth/logout — clears the session cookie. */
  logout(options?: RequestOptions): Promise<void>;
  /** GET /auth/me — current identity (role drives the shell). */
  me(options?: RequestOptions): Promise<Me>;
  /** GET /auth/sesi — idle and absolute deadlines for the shell warnings. */
  getSesi(options?: RequestOptions): Promise<SesiStatus>;
  /** POST /auth/sesi/perpanjang — resets the idle timer only. */
  perpanjangSesi(options?: RequestOptions): Promise<SesiStatus>;
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
  /** GET /providers — platform admin only. */
  listProviders(options?: RequestOptions): Promise<Provider[]>;
  /** GET /providers/{id} — provider detail for the edit form. */
  getProvider(id: string, options?: RequestOptions): Promise<Provider>;
  /** POST /providers — create; old versions stay intact on update. */
  createProvider(body: ProviderConfig, options?: RequestOptions): Promise<Provider>;
  /** PUT /providers/{id} — update; old versions stay intact (V-03). */
  updateProvider(id: string, body: ProviderConfig, options?: RequestOptions): Promise<Provider>;
}

/** A 401 from any endpoint ends the shell session; screens do not each need their own wiring. */
function announce401(response: Response): void {
  if (response.status === 401 && typeof window !== 'undefined') {
    window.dispatchEvent(new Event(SESI_BERAKHIR_EVENT));
  }
}

/** `Retry-After` is either delta-seconds or an HTTP date; screens only show whole seconds. */
function retryAfterFrom(response: Response): number | null {
  const raw = response.headers.get('Retry-After')?.trim();
  if (!raw) return null;
  if (/^\d+$/.test(raw)) return Number(raw);
  const at = Date.parse(raw);
  return Number.isFinite(at) ? Math.max(0, Math.ceil((at - Date.now()) / 1000)) : null;
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

/** Any screen's 401 means the shell's session is gone; the client announces it once, here. */
export const SESI_BERAKHIR_EVENT = 'pr:sesi-berakhir';

export function createApiClient(options: ApiClientOptions = {}): ApiClient {
  const origin =
    options.origin ??
    import.meta.env.VITE_ADMIN_API_BASE ??
    globalThis.location?.origin ??
    'http://localhost';
  const client = createClient<paths>({
    baseUrl: origin,
    fetch: options.fetch,
    // The session cookie is `HttpOnly` + `SameSite=Strict` (ADR-0003 rev 5 3.6 option A), so
    // same-origin `include` keeps it flowing without exposing the token to JavaScript.
    credentials: 'include',
    cache: 'no-store',
    headers: {
      Accept: 'application/json',
      // R-16: every admin read is `no-store`; ask intermediaries not to reuse it either.
      'Cache-Control': 'no-store',
    },
  });
  const headers = (opts: RequestOptions): Record<string, string> =>
    opts.alasan === undefined ? {} : { 'X-Alasan': opts.alasan };

  const ok = <T>(result: { data?: T; error?: unknown; response: Response }): T => {
    if (!result.response.ok) {
      announce401(result.response);
      throw new ApiError(
        result.response.status,
        problemFrom(result.response.status, result.error),
        retryAfterFrom(result.response),
      );
    }
    return result.data as T;
  };

  return {
    async login(body, opts = {}) {
      ok(
        await client.POST(`${ADMIN_PREFIX}/auth/login`, {
          body,
          headers: headers(opts),
          signal: opts.signal,
        }),
      );
    },

    async logout(opts = {}) {
      ok(
        await client.POST(`${ADMIN_PREFIX}/auth/logout`, {
          headers: headers(opts),
          signal: opts.signal,
        }),
      );
    },

    me: async (opts = {}) =>
      ok(
        await client.GET(`${ADMIN_PREFIX}/auth/me`, {
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    getSesi: async (opts = {}) =>
      ok(
        await client.GET(`${ADMIN_PREFIX}/auth/sesi`, {
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    perpanjangSesi: async (opts = {}) =>
      ok(
        await client.POST(`${ADMIN_PREFIX}/auth/sesi/perpanjang`, {
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    cariLog: async (body, opts = {}) =>
      ok(
        await client.POST(`${ADMIN_PREFIX}/log/cari`, {
          body,
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    cariLogNik: async (body, opts = {}) =>
      ok(
        await client.POST(`${ADMIN_PREFIX}/auditor/log/cari-nik`, {
          body,
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    bukaPayload: async (body, opts = {}) => {
      const result = await client.POST(`${ADMIN_PREFIX}/auditor/payload`, {
        body,
        headers: headers(opts),
        signal: opts.signal,
      });
      const view = ok(result);
      // The payload is only safe to render if the *answer* is uncacheable (R-16).
      const cacheControl = result.response.headers.get('Cache-Control') ?? '';
      if (!/\bno-store\b/i.test(cacheControl)) {
        throw new ApiError(502, {
          status: 502,
          kode: 'payload_bukan_no_store',
          title: 'Payload response is cacheable',
        });
      }
      return view;
    },

    eksporLog: async (body, opts = {}) => {
      const result = await client.POST(`${ADMIN_PREFIX}/auditor/log/ekspor`, {
        body,
        headers: headers(opts),
        signal: opts.signal,
        parseAs: 'blob',
      });
      if (!result.response.ok) {
        announce401(result.response);
        throw new ApiError(
          result.response.status,
          problemFrom(result.response.status, result.error),
        );
      }
      return {
        blob: result.data as Blob,
        filename: filenameFrom(result.response.headers.get('Content-Disposition')),
        digest: digestFrom(result.response.headers.get('Repr-Digest')),
      };
    },

    cariAuditAkses: async (body, opts = {}) =>
      ok(
        await client.POST(`${ADMIN_PREFIX}/auditor/audit-akses/cari`, {
          body,
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    kuota: async (upstreamPoolId, opts = {}) =>
      ok(
        await client.GET(`${ADMIN_PREFIX}/kuota`, {
          params: { query: upstreamPoolId ? { upstream_pool_id: upstreamPoolId } : {} },
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    upstreamPools: async (opts = {}) =>
      ok(
        await client.GET(`${ADMIN_PREFIX}/upstream-pools`, {
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    listProviders: async (opts = {}) =>
      ok(
        await client.GET(`${ADMIN_PREFIX}/providers`, {
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    getProvider: async (id, opts = {}) =>
      ok(
        await client.GET(`${ADMIN_PREFIX}/providers/{id}`, {
          params: { path: { id } },
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    createProvider: async (body, opts = {}) =>
      ok(
        await client.POST(`${ADMIN_PREFIX}/providers`, {
          body,
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),

    updateProvider: async (id, body, opts = {}) =>
      ok(
        await client.PUT(`${ADMIN_PREFIX}/providers/{id}`, {
          params: { path: { id } },
          body,
          headers: headers(opts),
          signal: opts.signal,
        }),
      ),
  };
}

/** App-wide client (same-origin). Screens accept an override in props for tests. */
export const api = createApiClient();
