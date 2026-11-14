/**
 * State antar file smoke (ARC-378, spec Testing Decisions: "Satu worker Playwright, gelombang
 * diurutkan lewat project dependencies karena objek G2 dipakai G3 sampai G5"). Project `api`
 * (`playwright.smoke.config.ts`) jalan `workers: 1` dan `fullyParallel: false`; Playwright memakai
 * satu proses worker untuk semua file test satu project selama tidak ada yang memaksa restart, jadi
 * modul-level state di sini tetap hidup lintas file — dipakai untuk meneruskan id objek G2
 * (provider/upstream/downstream/endpoint/client/credential/secret/allowlist) ke G3-G5.
 *
 * Urutan file (nama file numerik, Playwright jalan file dalam satu `testDir` berurutan nama)
 * menjaga gelombang G1 -> G2 -> G3 -> G4 -> G5 -> G6; lihat spec bagian 2 untuk definisi gelombang.
 * Nama field di bawah pakai istilah inventaris.md: P/U/Dn/E/K/Ko/Ka/Sc/W.
 */
export const state: {
  providerId?: string;
  upstreamPoolId?: string;
  downstreamPoolId?: string;
  endpointId?: string;
  clientId?: string;
  credentialOauth2Id?: string;
  credentialOauth2Secret?: string;
  credentialApiKeyId?: string;
  credentialApiKey?: string;
  secretNama?: string;
  allowlistDomain?: string;
  queueIdSync?: string;
  queueIdAsync?: string;
  kodeTransaksiSync?: string;
} = {};

/** Nama objek `smoke-*` dipakai di seluruh file (inventaris.md: K/P/U/Dn/E/Sc identitas sintetis). */
export const NAMA = {
  provider: 'smoke-provider',
  upstream: 'smoke-upstream',
  downstream: 'smoke-downstream',
  endpoint: 'smoke-endpoint',
  client: 'smoke-client',
  secret: 'smoke-secret',
  allowlistDomain: 'smoke.example.test',
} as const;

/** NIK sintetis G3 (inventaris.md: N), selalu diawali `00` (Keamanan). */
export const NIK_SINTETIS = '0000000000000001';
