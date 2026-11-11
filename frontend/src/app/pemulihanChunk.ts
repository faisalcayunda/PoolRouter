/** Jendela tanpa reload ulang; mencegah loop bila chunk memang hilang (bukan sekadar deploy baru). */
const JEDA_MS = 10_000;
const KUNCI = 'poolrouter.chunk-reload';

/** Subset `Window` yang dipakai, supaya tes cukup memakai objek palsu. */
export type JendelaPemulihan = Pick<Window, 'addEventListener' | 'location' | 'sessionStorage'>;

/**
 * Setelah deploy ulang, chunk lazy berhash lama di-404 dan `import()` reject (Vite menembakkan
 * `vite:preloadError`). Dokumen yang lama terbuka tidak lagi reload per klik, jadi pulihkan dengan
 * satu reload untuk mengambil `index.html` dan hash terbaru. Reload kedua dalam 10 detik tidak
 * dilakukan: kegagalan dibiarkan merambat agar tidak berputar tanpa henti.
 */
export function pasangPemulihanChunk(win: JendelaPemulihan = window): void {
  win.addEventListener('vite:preloadError', (ev) => {
    const sekarang = Date.now();
    try {
      const terakhir = Number(win.sessionStorage.getItem(KUNCI));
      if (terakhir && sekarang - terakhir < JEDA_MS) return;
      win.sessionStorage.setItem(KUNCI, String(sekarang));
    } catch {
      // sessionStorage tidak tersedia: tanpa stempel tidak ada jaminan anti-loop, jadi jangan reload.
      return;
    }
    ev.preventDefault();
    win.location.reload();
  });
}
