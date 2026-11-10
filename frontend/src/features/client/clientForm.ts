/** Batas limiter di server (L-04): burst 1..=1000, isi ulang 0,10..=100,00 per detik. */
export const BURST_MIN = 1;
export const BURST_MAX = 1000;
export const REFILL_MIN = 0.1;
export const REFILL_MAX = 100;
/** Nilai bawaan server bila kolom dikosongkan saat membuat client. */
export const BURST_DEFAULT = 20;
export const REFILL_DEFAULT = 1;

export const NAMA_MAKS = 200;

export type LimiterErrors = { nama?: 'required' | 'too_long'; burst?: 'range'; refill?: 'range' };

export interface ClientFormValues {
  nama: string;
  burst: string;
  refill: string;
}

/** Angka dari kolom teks: koma desimal Indonesia diterima, kosong = tidak diisi. */
export function parseNumber(raw: string): number | null | undefined {
  const text = raw.trim().replace(',', '.');
  if (text === '') return null;
  if (!/^\d+(\.\d+)?$/.test(text)) return undefined;
  return Number(text);
}

export function validateClientForm(values: ClientFormValues): LimiterErrors {
  const errors: LimiterErrors = {};
  const nama = values.nama.trim();
  if (nama === '') errors.nama = 'required';
  else if (nama.length > NAMA_MAKS) errors.nama = 'too_long';
  const burst = parseNumber(values.burst);
  if (
    burst === undefined ||
    (burst !== null && (!Number.isInteger(burst) || burst < BURST_MIN || burst > BURST_MAX))
  ) {
    errors.burst = 'range';
  }
  const refill = parseNumber(values.refill);
  if (refill === undefined || (refill !== null && (refill < REFILL_MIN || refill > REFILL_MAX))) {
    errors.refill = 'range';
  }
  return errors;
}

/** Isi ulang tampil dengan satu atau dua desimal, tanpa nol berlebih ("1", "0,5"). */
export function formatRefill(value: number, lng: string): string {
  return new Intl.NumberFormat(lng, { maximumFractionDigits: 2 }).format(value);
}
