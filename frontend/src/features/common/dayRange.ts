import { today, type DateValue } from '@internationalized/date';

/** Satu-satunya zona waktu layar filter (WIB); rentang tanggal dikirim sebagai hari penuh WIB. */
const ZONE = 'Asia/Jakarta';

/** Rentang dua tanggal; bentuknya sama dengan `RangeValue` milik React Aria. */
export interface DayRange {
  start: DateValue;
  end: DateValue;
}

/** Default kunjungan pertama: hari ini (WIB), seperti rentang di frame Penpot. */
export function todayRange(): DayRange {
  const day = today(ZONE);
  return { start: day, end: day };
}

/** Awal hari pertama dan akhir hari terakhir rentang, dalam ISO 8601 dengan offset WIB. */
export function dayRangeBounds(range: DayRange): { dari: string; sampai: string } {
  return {
    dari: `${range.start.toString()}T00:00:00+07:00`,
    sampai: `${range.end.toString()}T23:59:59+07:00`,
  };
}
