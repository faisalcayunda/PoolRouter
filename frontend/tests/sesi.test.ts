import { describe, expect, it } from 'vitest';
import type { SesiStatus } from '../src/api/client';
import { detikKe, fasePada, offsetServer, PERINGATAN_MS } from '../src/features/auth/sesi';

const SESI: SesiStatus = {
  idle_berakhir_pada: '2026-11-03T08:00:00.000Z',
  absolut_berakhir_pada: '2026-11-03T15:00:00.000Z',
  waktu_server: '2026-11-03T07:30:00.000Z',
};

const at = (iso: string) => Date.parse(iso);

describe('fase sesi', () => {
  it('is calm until the idle warning window opens', () => {
    expect(fasePada(SESI, at('2026-11-03T07:57:59.000Z'))).toBe('aman');
    expect(fasePada(SESI, at('2026-11-03T07:58:00.000Z'))).toBe('peringatan_idle');
  });

  it('moves to the absolute warning when extending is pointless', () => {
    expect(fasePada(SESI, at('2026-11-03T14:58:00.000Z'))).toBe('peringatan_absolut');
  });

  it('is over at the absolute deadline even if the idle timer was extended', () => {
    expect(fasePada(SESI, at('2026-11-03T15:00:00.000Z'))).toBe('berakhir');
  });

  it('counts down whole seconds and never shows a negative number', () => {
    expect(detikKe(SESI.idle_berakhir_pada, at('2026-11-03T07:58:30.500Z'))).toBe(90);
    expect(detikKe(SESI.idle_berakhir_pada, at('2026-11-03T09:00:00.000Z'))).toBe(0);
  });

  it('uses the server clock, not the laptop clock', () => {
    // The server says 07:30; the browser thinks it is 07:00, so it is 30 minutes behind.
    expect(offsetServer(SESI, at('2026-11-03T07:00:00.000Z'))).toBe(30 * 60 * 1000);
    expect(PERINGATAN_MS).toBe(120_000);
  });
});
