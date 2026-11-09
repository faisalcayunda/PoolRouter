import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import type { ApiClient, SesiStatus } from '../src/api/client';
import { useSesi } from '../src/features/auth/useSesi';

const MENIT = 60_000;
const T0 = Date.parse('2026-11-03T07:30:00.000Z');
const ABSOLUT = '2026-11-03T15:00:00.000Z';
const iso = (ms: number) => new Date(ms).toISOString();

/** Status server pada saat ini (jam palsu), dengan idle berakhir `idleMs` dari epoch. */
function status(idleMs: number): SesiStatus {
  return {
    idle_berakhir_pada: iso(idleMs),
    absolut_berakhir_pada: ABSOLUT,
    waktu_server: iso(Date.now()),
  };
}

function siapkan(idleAwal: number) {
  let idle = idleAwal;
  const getSesi = vi.fn(async () => status(idle));
  const perpanjangSesi = vi.fn(async () => {
    idle = Date.now() + 30 * MENIT;
    return status(idle);
  });
  const client = {
    me: vi.fn().mockResolvedValue({ username: 'a', peran: ['auditor'] }),
    getSesi,
    perpanjangSesi,
    logout: vi.fn(),
  } as unknown as ApiClient;
  /** Meniru request aktif dari tab lain yang menggeser idle di server. */
  const geser = (ke: number) => {
    idle = ke;
  };
  return { client, getSesi, perpanjangSesi, geser };
}

const maju = (ms: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });

describe('useSesi (jadwal sesuai ADR-0003 3.6f)', () => {
  beforeEach(() => {
    vi.useFakeTimers({ now: T0 });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('Dialog tidak muncul jika GET menunjukkan sesi sudah digeser', async () => {
    const { client, getSesi, geser } = siapkan(T0 + 30 * MENIT);
    const onBerakhir = vi.fn();
    const { result } = renderHook(() => useSesi({ client, onBerakhir, onKeluar: vi.fn() }));
    await maju(0);
    expect(result.current.fase).toBe('aman');
    expect(getSesi).toHaveBeenCalledTimes(1);

    // Tab lain aktif pada menit ke-20, jadi server menggeser idle ke menit ke-50.
    await maju(20 * MENIT);
    geser(T0 + 50 * MENIT);

    // Jadwal lokal tiba pada menit ke-28: hook harus bertanya ke server dulu.
    await maju(8 * MENIT + 100);
    expect(getSesi).toHaveBeenCalledTimes(2);
    expect(result.current.fase).toBe('aman');
    expect(onBerakhir).not.toHaveBeenCalled();
  });

  it('membuka dialog idle kalau GET membenarkan jadwal', async () => {
    const { client, getSesi } = siapkan(T0 + 30 * MENIT);
    const { result } = renderHook(() =>
      useSesi({ client, onBerakhir: vi.fn(), onKeluar: vi.fn() }),
    );
    // Flush muatan awal dulu: timer jadwal baru dipasang setelah me() dan getSesi() selesai dan
    // dirender, dan di dalam satu act render itu baru terjadi di akhir, setelah jam maju.
    await maju(0);
    await maju(28 * MENIT + 100);
    expect(getSesi).toHaveBeenCalledTimes(2);
    expect(result.current.fase).toBe('peringatan_idle');
  });

  it('Perpanjang sesi menjadwal ulang pengecekan', async () => {
    const { client, getSesi, perpanjangSesi } = siapkan(T0 + 30 * MENIT);
    const { result } = renderHook(() =>
      useSesi({ client, onBerakhir: vi.fn(), onKeluar: vi.fn() }),
    );
    // Flush muatan awal dulu: timer jadwal baru dipasang setelah me() dan getSesi() selesai dan
    // dirender, dan di dalam satu act render itu baru terjadi di akhir, setelah jam maju.
    await maju(0);
    await maju(28 * MENIT + 100);
    expect(result.current.fase).toBe('peringatan_idle');
    expect(getSesi).toHaveBeenCalledTimes(2);

    await act(async () => {
      await result.current.perpanjang();
    });
    expect(perpanjangSesi).toHaveBeenCalledTimes(1);
    expect(result.current.fase).toBe('aman');

    // Jadwal lama (menit ke-30) sudah dibatalkan; tidak ada GET baru.
    await maju(2 * MENIT);
    expect(getSesi).toHaveBeenCalledTimes(2);

    // Jadwal baru: 28 menit setelah perpanjangan.
    await maju(26 * MENIT + 100);
    expect(getSesi).toHaveBeenCalledTimes(3);
    expect(result.current.fase).toBe('peringatan_idle');
  });

  it('berakhir dan mengalihkan saat batas idle tiba, bukan macet di 0 detik', async () => {
    const { client, getSesi } = siapkan(T0 + 30 * MENIT);
    const onBerakhir = vi.fn();
    const { result } = renderHook(() => useSesi({ client, onBerakhir, onKeluar: vi.fn() }));
    await maju(0); // flush muatan awal supaya timer jadwal terpasang sebelum jam maju
    await maju(28 * MENIT + 100);
    expect(result.current.fase).toBe('peringatan_idle');
    expect(onBerakhir).not.toHaveBeenCalled();

    await maju(2 * MENIT + 100);
    expect(getSesi).toHaveBeenCalledTimes(3);
    expect(result.current.fase).toBe('berakhir');
    expect(onBerakhir).toHaveBeenCalledTimes(1);
  });
});
