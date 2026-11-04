import { useCallback, useEffect, useRef, useState } from 'react';
import {
  api,
  ApiError,
  SESI_BERAKHIR_EVENT,
  type ApiClient,
  type Me,
  type SesiStatus,
} from '../../api/client';
import { detikKe, fasePada, offsetServer, PERINGATAN_MS, type FaseSesi } from './sesi';

export interface SesiArgs {
  client?: ApiClient;
  /** Navigate to the login screen with `alasan=sesi_berakhir`. */
  onBerakhir: () => void;
  /** Navigate to the login screen after an explicit logout. */
  onKeluar: () => void;
}

export interface SesiState {
  me: Me | null;
  sesi: SesiStatus | null;
  fase: FaseSesi;
  memuat: boolean;
  galat: boolean;
  perpanjangGalat: boolean;
  /** Seconds left before the idle deadline, for the dialog countdown. */
  detikIdle: number;
  muatUlang: () => void;
  perpanjang: () => Promise<boolean>;
  keluar: () => Promise<void>;
}

/**
 * Session state for the shell (handoff spec section 12, ADR-0003 rev 5 3.6f). It loads identity and
 * deadlines once, schedules exactly two timers (idle warning, absolute warning) and re-reads the
 * deadlines on `visibilitychange`; there is no polling (A01-14).
 */
export function useSesi({ client = api, onBerakhir, onKeluar }: SesiArgs): SesiState {
  const [me, setMe] = useState<Me | null>(null);
  const [sesi, setSesi] = useState<SesiStatus | null>(null);
  const [fase, setFase] = useState<FaseSesi>('memuat');
  const [galat, setGalat] = useState(false);
  const [memuat, setMemuat] = useState(true);
  const [perpanjangGalat, setPerpanjangGalat] = useState(false);
  const [detikIdle, setDetikIdle] = useState(0);
  const offset = useRef(0);
  const berakhir = useRef(onBerakhir);
  useEffect(() => {
    berakhir.current = onBerakhir;
  }, [onBerakhir]);

  const sekarang = useCallback(() => Date.now() + offset.current, []);

  const pakai = useCallback((next: SesiStatus) => {
    offset.current = offsetServer(next, Date.now());
    setSesi(next);
    setFase(fasePada(next, sekarang()));
  }, [sekarang]);

  const muat = useCallback(
    (signal?: AbortSignal) => {
      Promise.all([client.me({ signal }), client.getSesi({ signal })])
        .then(([identity, status]) => {
          setMe(identity);
          pakai(status);
        })
        .catch((error: unknown) => {
          if (signal?.aborted) return;
          if (error instanceof ApiError && error.status === 401) {
            berakhir.current();
            return;
          }
          setGalat(true);
        })
        .finally(() => {
          if (!signal?.aborted) setMemuat(false);
        });
    },
    [client, pakai],
  );

  useEffect(() => {
    const controller = new AbortController();
    muat(controller.signal);
    return () => controller.abort();
  }, [muat]);

  // A 401 anywhere in the app ends the session once for the whole shell.
  useEffect(() => {
    const onUnauthorized = () => berakhir.current();
    window.addEventListener(SESI_BERAKHIR_EVENT, onUnauthorized);
    return () => window.removeEventListener(SESI_BERAKHIR_EVENT, onUnauthorized);
  }, []);

  // Re-read deadlines when the tab becomes visible again; deadlines may have moved while asleep.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState !== 'visible') return;
      client
        .getSesi()
        .then(pakai)
        .catch((error: unknown) => {
          if (error instanceof ApiError && error.status === 401) berakhir.current();
        });
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, [client, pakai]);

  // One timer per state transition; it never wakes up more often than the spec allows.
  useEffect(() => {
    if (!sesi || fase === 'berakhir') return;
    const idle = Date.parse(sesi.idle_berakhir_pada);
    const absolut = Date.parse(sesi.absolut_berakhir_pada);
    if (!Number.isFinite(idle) || !Number.isFinite(absolut)) return;
    const target =
      fase === 'aman'
        ? Math.min(idle - PERINGATAN_MS, absolut - PERINGATAN_MS)
        : fase === 'peringatan_idle'
          ? absolut - PERINGATAN_MS
          : absolut;
    const delay = Math.max(0, target - sekarang());
    const timer = setTimeout(() => {
      const next = fasePada(sesi, sekarang());
      setFase(next);
      if (next === 'berakhir') berakhir.current();
    }, delay + 20);
    return () => clearTimeout(timer);
  }, [sesi, fase, sekarang]);

  // Countdown text updates once per second only while the idle dialog is open.
  useEffect(() => {
    if (fase !== 'peringatan_idle' || !sesi) return;
    const tick = () => setDetikIdle(detikKe(sesi.idle_berakhir_pada, sekarang()));
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [fase, sesi, sekarang]);

  const perpanjang = useCallback(async () => {
    try {
      const next = await client.perpanjangSesi();
      pakai(next);
      setPerpanjangGalat(false);
      return true;
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        berakhir.current();
      } else {
        setPerpanjangGalat(true);
      }
      return false;
    }
  }, [client, pakai]);

  const keluar = useCallback(async () => {
    try {
      await client.logout();
    } catch {
      // Logout is best effort: a failed call still drops the local session.
    }
    onKeluar();
  }, [client, onKeluar]);

  return {
    me,
    sesi,
    fase,
    memuat,
    galat,
    perpanjangGalat,
    detikIdle,
    muatUlang: () => {
      setMemuat(true);
      setGalat(false);
      muat();
    },
    perpanjang,
    keluar,
  };
}
