import { describe, expect, it } from 'vitest';
import { createFormatter, hourRangeLabel } from '../src/i18n/format';
import type { TFunction } from 'i18next';

/** The WIB join comes from the catalog; here it is stubbed so the test stays about formatting. */
const t = ((_key: string, opts: { tanggal: string; jam: string }) =>
  `${opts.tanggal}, ${opts.jam} WIB`) as unknown as TFunction;

describe('Intl formatting (Addendum A section 5)', () => {
  const id = createFormatter('id', t);
  const en = createFormatter('en', t);
  const instant = '2026-11-03T07:30:00.000Z';

  it('formats an instant in WIB, not in the browser zone', () => {
    // 07:30 UTC is 14:30 in Jakarta.
    expect(id.jam(instant)).toBe('14.30');
    expect(en.jam(instant)).toBe('14:30');
  });

  it('joins date and time through the catalog string', () => {
    expect(id.tanggalWaktu(instant)).toBe('3 Nov 2026, 14.30 WIB');
  });

  it('groups numbers per language', () => {
    // Indonesian uses `.` as the group separator, English `,`.
    expect(id.angka(12345)).toBe('12.345');
    expect(en.angka(12345)).toBe('12,345');
  });

  it('prints quota percentages as whole numbers', () => {
    expect(id.persen(0.42)).toBe('42%');
    expect(id.persen(0.005)).toBe('1%');
  });

  it('builds the current-hour range for the quota tracker', () => {
    const from = new Date('2026-11-03T00:00:00.000Z'); // 07.00 WIB
    const to = new Date('2026-11-03T01:00:00.000Z'); // 08.00 WIB
    expect(hourRangeLabel('id', from, to)).toBe('Sel, 07.00–08.00');
  });

  it('never formats a countdown with a negative value', () => {
    expect(id.durasi(-5)).toBe('0:00');
    expect(id.durasi(65)).toBe('1:05');
  });
});
