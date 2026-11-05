import type { SesiStatus } from '../../api/client';

/** Handoff spec section 12.1: the warning window before both deadlines. */
export const PERINGATAN_MS = 2 * 60 * 1000;

export type FaseSesi = 'memuat' | 'aman' | 'peringatan_idle' | 'peringatan_absolut' | 'berakhir';

/**
 * Which session state a screen shows at `saat` (epoch ms). The absolute limit wins over the idle
 * one: once the 8-hour deadline is inside the warning window, extending the idle timer is useless.
 */
export function fasePada(sesi: SesiStatus, saat: number): FaseSesi {
  const absolut = Date.parse(sesi.absolut_berakhir_pada);
  const idle = Date.parse(sesi.idle_berakhir_pada);
  if (!Number.isFinite(absolut) || !Number.isFinite(idle)) return 'aman';
  if (saat >= absolut) return 'berakhir';
  if (saat >= absolut - PERINGATAN_MS) return 'peringatan_absolut';
  if (saat >= idle - PERINGATAN_MS) return 'peringatan_idle';
  return 'aman';
}

/** Whole seconds until `iso`; never negative, so a dialog never prints "-3 detik". */
export function detikKe(iso: string, saat: number): number {
  const target = Date.parse(iso);
  if (!Number.isFinite(target)) return 0;
  return Math.max(0, Math.ceil((target - saat) / 1000));
}

/**
 * Client-to-server clock offset. Every deadline is server time, so the countdown uses the server
 * clock rather than trusting the laptop's.
 */
export function offsetServer(sesi: SesiStatus, clientMs: number): number {
  const serverMs = Date.parse(sesi.waktu_server);
  return Number.isFinite(serverMs) ? serverMs - clientMs : 0;
}
