import type { AdminRole } from '../common/RoleContext';

/**
 * Siapa yang melihat layar client (K5). `null` = peran belum diketahui: layar tidak menebak, dan
 * API tetap penjaga terakhir untuk setiap aksi. Operator dan auditor tidak punya layar ini.
 */
export function canViewClients(role: AdminRole | null): boolean {
  return role === null || role === 'platform_admin' || role === 'client_owner';
}

/** Buat, ubah, dan (non)aktifkan client hanya untuk platform admin; client_owner hanya membaca. */
export function canManageClients(role: AdminRole | null): boolean {
  return role === null || role === 'platform_admin';
}

/** Batas kredensial aktif per client di server (EN-02). */
export const MAKS_KREDENSIAL_AKTIF = 2;

export function clientPath(clientId: string, bagian?: 'kredensial' | 'webhook'): string {
  const base = `/client/${encodeURIComponent(clientId)}`;
  return bagian ? `${base}/${bagian}` : base;
}
