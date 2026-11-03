import { useId, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { parseDate, Time, type AnyTime } from '@internationalized/date';
import { DatePicker } from '../../components/m3/DatePicker';
import { TimeField as M3TimeField } from '../../components/m3/TimeField';
import { InlineAlert } from '../../components/m3/InlineAlert';

/** A closed interval in WIB, already serialised for `LogCariRequest.dari/sampai`. */
export interface DateRange {
  from: string;
  to: string;
  /** The picker's own values so the fields keep showing what the user typed. */
  startDate: string;
  startTime: string;
  endDate: string;
  endTime: string;
}

/**
 * Merge a bare edit into the current range and serialise it for the API.
 *
 * Returns `null` while any of the four parts is missing, and for an inverted pair: a half-filled
 * range must never reach a request body. Kept pure so the rule is testable without driving React
 * Aria's segmented date inputs.
 */
export function mergeRange(current: DateRange | null, next: Partial<DateRange>): DateRange | null {
  const merged = {
    startDate: next.startDate ?? current?.startDate ?? '',
    startTime: next.startTime ?? current?.startTime ?? '',
    endDate: next.endDate ?? current?.endDate ?? '',
    endTime: next.endTime ?? current?.endTime ?? '',
  };
  if (!merged.startDate || !merged.startTime || !merged.endDate || !merged.endTime) return null;
  const from = `${merged.startDate}T${merged.startTime}:00+07:00`;
  const to = `${merged.endDate}T${merged.endTime}:00+07:00`;
  return from <= to ? { ...merged, from, to } : null;
}

/** RAC hands back an `AnyTime`; only hour and minute matter, and both ends are sent as WIB. */
function timeText(time: AnyTime | null): string {
  if (!time) return '';
  const hour = 'hour' in time ? time.hour : 0;
  const minute = 'minute' in time ? time.minute : 0;
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`;
}

/** `00:00` → `Time`; anything else (a blank or half-typed value) stays empty. */
function toTime(value: string): Time | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value);
  if (!match) return null;
  return new Time(Number(match[1]), Number(match[2]));
}

export interface DateRangeFieldProps {
  value: DateRange | null;
  onChange: (range: DateRange | null) => void;
  legend: string;
  description?: string;
  errorMessage?: string | null;
}

/**
 * Two date + time pairs forming one WIB interval (spec section 15).
 *
 * The labels reuse the `log.filter.range` wording rather than adding keys: the v0.13 catalog lists
 * the group heading but not the four segment labels, so the segments are titled "Rentang waktu"
 * with the start/end distinguished by position and by the pickers' own accessible names.
 */
export function DateRangeField({
  value,
  onChange,
  legend,
  description,
  errorMessage,
}: DateRangeFieldProps): ReactNode {
  const { t } = useTranslation();
  const id = useId();

  const startDate = value?.startDate ?? '';
  const startTime = value?.startTime ?? '';
  const endDate = value?.endDate ?? '';
  const endTime = value?.endTime ?? '';

  function emit(next: Partial<DateRange>) {
    onChange(mergeRange(value, next));
  }

  const describedBy =
    [description ? `${id}-desc` : null, errorMessage ? `${id}-error` : null]
      .filter(Boolean)
      .join(' ') || undefined;

  return (
    <fieldset
      className="flex flex-col gap-2"
      aria-describedby={describedBy}
      aria-invalid={errorMessage ? true : undefined}
    >
      <legend className="typescale-label-large mb-1">{legend}</legend>
      {description ? (
        <p id={`${id}-desc`} className="typescale-body-small text-on-surface-variant">
          {description}
        </p>
      ) : null}
      <div className="flex flex-wrap items-end gap-2">
        <DatePicker
          label={t('log.filter.range')}
          calendarLabel={t('log.filter.range')}
          value={startDate ? parseDate(startDate) : null}
          onChange={(date) => emit({ startDate: date ? date.toString() : '' })}
        />
        <M3TimeField
          label={t('log.filter.range')}
          value={toTime(startTime)}
          onChange={(time) => emit({ startTime: timeText(time) })}
        />
      </div>
      <div className="flex flex-wrap items-end gap-2">
        <DatePicker
          label={t('log.filter.range')}
          calendarLabel={t('log.filter.range')}
          value={endDate ? parseDate(endDate) : null}
          onChange={(date) => emit({ endDate: date ? date.toString() : '' })}
        />
        <M3TimeField
          label={t('log.filter.range')}
          value={toTime(endTime)}
          onChange={(time) => emit({ endTime: timeText(time) })}
        />
      </div>
      {errorMessage ? (
        <div id={`${id}-error`}>
          <InlineAlert tone="error" live="polite">
            {errorMessage}
          </InlineAlert>
        </div>
      ) : null}
    </fieldset>
  );
}
