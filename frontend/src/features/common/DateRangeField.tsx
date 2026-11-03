import { useId, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { parseDate, Time } from '@internationalized/date';
import { DatePicker } from '../../components/m3/DatePicker';
import { TimeField as M3TimeField } from '../../components/m3/TimeField';
import { TextField } from '../../components/m3/TextField';

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

/** Which end of the range a message belongs to. */
export type DateRangeFieldErrors = Partial<Record<'startDate' | 'endDate', string>>;

/** `yyyy-mm-dd` → the `dd/mm/yyyy` the user types and reads (section 15.1). */
export function formatTypedDate(iso: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  return match ? `${match[3]}/${match[2]}/${match[1]}` : iso;
}

/**
 * Parse one typed date. Returns ISO `yyyy-mm-dd` for a real calendar day, or `null` when the text
 * is not `dd/mm/yyyy` or names a day that does not exist (31/02/2026).
 */
export function parseTypedDate(text: string): string | null {
  const match = /^(\d{2})\/(\d{2})\/(\d{4})$/.exec(text.trim());
  if (!match) return null;
  const [, dd, mm, yyyy] = match;
  const date = new Date(Number(yyyy), Number(mm) - 1, Number(dd));
  if (
    date.getFullYear() !== Number(yyyy) ||
    date.getMonth() !== Number(mm) - 1 ||
    date.getDate() !== Number(dd)
  ) {
    return null;
  }
  return `${yyyy}-${mm}-${dd}`;
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
  const { startDate, startTime, endDate, endTime } = merged;
  if (!isIsoDate(startDate) || !isIsoDate(endDate) || !startTime || !endTime) return null;
  const from = `${startDate}T${startTime}:00+07:00`;
  const to = `${endDate}T${endTime}:00+07:00`;
  return from <= to ? { ...merged, from, to } : null;
}

function isIsoDate(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(value);
}

/** RAC hands back a `TimeValue`; only hour and minute matter, and both ends are sent as WIB. */
function timeText(time: { hour: number; minute: number } | null): string {
  if (!time) return '';
  return `${String(time.hour).padStart(2, '0')}:${String(time.minute).padStart(2, '0')}`;
}

/** `00:00` → `Time`; anything else (a blank or half-typed value) stays empty. */
function toTime(value: string): Time | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value);
  if (!match) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  if (hour > 23 || minute > 59) return null;
  return new Time(hour, minute);
}

export interface DateRangeFieldProps {
  value: DateRange | null;
  /**
   * Raw `dd/mm/yyyy` text per end, so a half-typed date stays visible. Defaults to `value`; callers
   * that hold the text in their own state pass it here.
   */
  typed?: { start: string; end: string };
  onTypedChange?: (typed: { start: string; end: string }) => void;
  onChange: (range: DateRange | null) => void;
  legend: string;
  description?: string;
  /** One message per field: a field with a message is marked `aria-invalid` (section 15.1). */
  errors?: DateRangeFieldErrors;
  /** A message about the pair as a whole ("Sampai lebih awal dari Dari"). */
  errorMessage?: string | null;
}

/**
 * Two date + time pairs forming one WIB interval (spec section 15.1).
 *
 * The primary path is the manual text field: "Dari" and "Sampai", `dd/mm/yyyy`, the format in the
 * supporting text, and one error per field via `aria-invalid`. The calendar and clock pickers sit
 * beside them as a shortcut for the same value.
 */
export function DateRangeField({
  value,
  typed,
  onTypedChange,
  onChange,
  legend,
  description,
  errors,
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

  const startText = typed?.start ?? (startDate ? formatTypedDate(startDate) : '');
  const endText = typed?.end ?? (endDate ? formatTypedDate(endDate) : '');

  /**
   * Typing keeps the raw text so a half-typed date is not silently dropped, and only a complete
   * calendar day moves the value that is sent to the API.
   */
  function typeInto(which: 'start' | 'end', text: string) {
    const next = { start: startText, end: endText, [which]: text };
    onTypedChange?.(next);
    const iso = parseTypedDate(text);
    emit(which === 'start' ? { startDate: iso ?? '' } : { endDate: iso ?? '' });
  }

  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="typescale-label-large mb-1">{legend}</legend>
      {description ? (
        <p className="typescale-body-small text-on-surface-variant">{description}</p>
      ) : null}
      <div className="flex flex-wrap items-start gap-6">
        <div className="flex flex-wrap items-start gap-3">
          <TextField
            label={t('common.date.from')}
            description={t('common.date.format_hint')}
            inputMode="numeric"
            value={startText}
            onChange={(text) => typeInto('start', text)}
            isInvalid={Boolean(errors?.startDate)}
            errorMessage={errors?.startDate}
            className="w-40"
          />
          <M3TimeField
            label={t('common.date.from_time')}
            value={startTime ? toTime(startTime) : null}
            onChange={(time) => emit({ startTime: timeText(time) })}
            hourCycle={24}
            className="w-28"
          />
          <DatePicker
            label={t('common.date.from_picker')}
            calendarLabel={t('common.date.from')}
            value={isIsoDate(startDate) ? parseDate(startDate) : null}
            onChange={(date) => emit({ startDate: date ? date.toString() : '' })}
            className="w-44"
          />
        </div>
        <div className="flex flex-wrap items-start gap-3">
          <TextField
            label={t('common.date.to')}
            description={t('common.date.format_hint')}
            inputMode="numeric"
            value={endText}
            onChange={(text) => typeInto('end', text)}
            isInvalid={Boolean(errors?.endDate)}
            errorMessage={errors?.endDate}
            className="w-40"
          />
          <M3TimeField
            label={t('common.date.to_time')}
            value={endTime ? toTime(endTime) : null}
            onChange={(time) => emit({ endTime: timeText(time) })}
            hourCycle={24}
            className="w-28"
          />
          <DatePicker
            label={t('common.date.to_picker')}
            calendarLabel={t('common.date.to')}
            value={isIsoDate(endDate) ? parseDate(endDate) : null}
            onChange={(date) => emit({ endDate: date ? date.toString() : '' })}
            className="w-44"
          />
        </div>
      </div>
      {errorMessage ? (
        <p id={`${id}-error`} role="alert" className="typescale-body-small text-error">
          {errorMessage}
        </p>
      ) : null}
    </fieldset>
  );
}
