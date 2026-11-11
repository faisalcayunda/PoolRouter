import { describe, expect, it, vi } from 'vitest';
import { pasangPemulihanChunk, type JendelaPemulihan } from '../src/app/pemulihanChunk';

/** Window palsu: EventTarget, sessionStorage di Map, dan `location.reload` yang bisa dihitung. */
function jendelaPalsu() {
  const target = new EventTarget();
  const simpan = new Map<string, string>();
  const reload = vi.fn();
  const win = {
    addEventListener: target.addEventListener.bind(target),
    location: { reload },
    sessionStorage: {
      getItem: (k: string) => simpan.get(k) ?? null,
      setItem: (k: string, v: string) => void simpan.set(k, v),
    },
  } as unknown as JendelaPemulihan;
  const gagal = () => {
    const ev = new Event('vite:preloadError', { cancelable: true });
    target.dispatchEvent(ev);
    return ev;
  };
  return { win, reload, gagal };
}

describe('pasangPemulihanChunk', () => {
  it('kegagalan pertama reload sekali dan mencegah error default', () => {
    const { win, reload, gagal } = jendelaPalsu();
    pasangPemulihanChunk(win);
    expect(gagal().defaultPrevented).toBe(true);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('kegagalan kedua dalam 10 detik tidak reload dan tidak mencegah error', () => {
    const { win, reload, gagal } = jendelaPalsu();
    pasangPemulihanChunk(win);
    gagal();
    const kedua = gagal();
    expect(kedua.defaultPrevented).toBe(false);
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
