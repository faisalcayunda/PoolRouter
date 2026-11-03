import { describe, expect, it } from 'vitest';
import { mergeRange, type DateRange } from '../src/features/common/DateRangeField';

const full: DateRange = {
  startDate: '2026-11-02',
  startTime: '00:00',
  endDate: '2026-11-03',
  endTime: '00:00',
  from: '2026-11-02T00:00:00+07:00',
  to: '2026-11-03T00:00:00+07:00',
};

describe('date range serialisation (section 15)', () => {
  it('emits WIB ISO 8601 with the +07:00 offset the API expects', () => {
    expect(mergeRange(null, { ...full })).toEqual({
      startDate: '2026-11-02',
      startTime: '00:00',
      endDate: '2026-11-03',
      endTime: '00:00',
      from: '2026-11-02T00:00:00+07:00',
      to: '2026-11-03T00:00:00+07:00',
    });
  });

  it('drops the range as soon as one of the four parts is missing', () => {
    // Both ends are required: a half-filled range must never reach a request body.
    expect(mergeRange(full, { endDate: '' })).toBeNull();
    expect(mergeRange(full, { endTime: '' })).toBeNull();
    expect(mergeRange(full, { startDate: '' })).toBeNull();
    expect(mergeRange(full, { startTime: '' })).toBeNull();
  });

  it('rejects an inverted pair instead of sending it', () => {
    expect(mergeRange(full, { endDate: '2026-11-01' })).toBeNull();
    expect(mergeRange(full, { startDate: '2026-11-04' })).toBeNull();
  });

  it('accepts a zero-width range (start equal to end)', () => {
    const same = mergeRange(full, { endDate: '2026-11-02', endTime: '00:00' });
    expect(same?.from).toBe('2026-11-02T00:00:00+07:00');
    expect(same?.to).toBe('2026-11-02T00:00:00+07:00');
  });

  it('keeps a partial edit visible while reporting no range yet', () => {
    // The user's typed values must stay on screen; only the emitted range is null.
    const partial = mergeRange(full, { endDate: '' });
    expect(partial).toBeNull();
    expect(mergeRange({ ...full, endDate: '' }, { endDate: '2026-11-05' })?.to).toBe(
      '2026-11-05T00:00:00+07:00',
    );
  });
});
