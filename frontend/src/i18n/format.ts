import type { TFunction } from 'i18next';
import { LOCALES, type BahasaUi } from './locales';

/** Addendum A section 5: all formatting is Intl with the fixed Asia/Jakarta zone. */
const TIME_ZONE = 'Asia/Jakarta';

type DateInput = Date | string | number;

function toDate(value: DateInput): Date {
  return value instanceof Date ? value : new Date(value);
}

export interface Formatter {
  tanggal(value: DateInput): string;
  jam(value: DateInput): string;
  /** "3 Nov 2026, 14.30 WIB": date and time are formatted apart, the WIB join comes from the catalog. */
  tanggalWaktu(value: DateInput): string;
  angka(value: number): string;
  persen(fraction: number, fractionDigits?: number): string;
  /** `m:ss` countdown from seconds. */
  durasi(totalSeconds: number): string;
  relatif(value: DateInput, now?: DateInput): string;
}

export function createFormatter(lng: BahasaUi, t: TFunction): Formatter {
  const locale = LOCALES[lng].formatLocale;
  const dateFmt = new Intl.DateTimeFormat(locale, {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    timeZone: TIME_ZONE,
  });
  const timeFmt = new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
    timeZone: TIME_ZONE,
  });
  const numFmt = new Intl.NumberFormat(locale);
  const two = new Intl.NumberFormat(locale, { minimumIntegerDigits: 2, useGrouping: false });
  const rel = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const normalize = (s: string) => s.replace(/ | /g, ' ');

  const tanggal = (v: DateInput) => normalize(dateFmt.format(toDate(v)));
  const jam = (v: DateInput) => normalize(timeFmt.format(toDate(v)));
  return {
    tanggal,
    jam,
    tanggalWaktu: (v) => t('common.datetime.wib', { tanggal: tanggal(v), jam: jam(v) }),
    angka: (n) => numFmt.format(n),
    persen: (f, digits = 0) =>
      new Intl.NumberFormat(locale, {
        style: 'percent',
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
      }).format(f),
    durasi: (total) => {
      const s = Math.max(0, Math.floor(total));
      return `${numFmt.format(Math.floor(s / 60))}:${two.format(s % 60)}`;
    },
    relatif: (v, now = Date.now()) => {
      const diff = (toDate(v).getTime() - toDate(now).getTime()) / 1000;
      const abs = Math.abs(diff);
      if (abs < 60) return rel.format(Math.round(diff), 'second');
      if (abs < 3600) return rel.format(Math.round(diff / 60), 'minute');
      if (abs < 86400) return rel.format(Math.round(diff / 3600), 'hour');
      return rel.format(Math.round(diff / 86400), 'day');
    },
  };
}
