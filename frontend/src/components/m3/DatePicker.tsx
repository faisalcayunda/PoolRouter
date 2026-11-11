import {
  Calendar,
  CalendarCell,
  CalendarGrid,
  DateSegment as RacDateSegment,
  DatePicker as RacDatePicker,
  DateRangePicker as RacDateRangePicker,
  DateInput as RacDateInput,
  Dialog,
  Group,
  Label,
  Popover,
  RangeCalendar,
  type DatePickerProps as RacDatePickerProps,
  type DateRangePickerProps as RacDateRangePickerProps,
} from 'react-aria-components';
import type { DateValue } from '@internationalized/date';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { IconButton } from './IconButton';
import { Icon, icons } from '../../icons';

const FIELD =
  'm3-focus typescale-body-large flex h-14 w-full items-center gap-1 rounded-extra-small border border-outline bg-surface px-2 text-on-surface data-[focus-within]:border-2 data-[focus-within]:border-primary';

// Rentang memuat dua tanggal dan tombol kalender; pada 320 px isinya lebih lebar dari kolom, jadi
// tombol turun ke baris berikutnya alih-alih melebarkan dokumen. Pada expanded kolom tetap satu baris
// setinggi 56 px, sejajar dengan TextField dan Select di sebelahnya; tombol kalender memakai margin
// kanan negatif (ikon trailing M3) dan segmen tanpa padding agar isinya muat di kolom 280 px.
const RANGE_FIELD = FIELD.replace(
  'h-14',
  'min-h-14 flex-wrap py-1 expanded:h-14 expanded:flex-nowrap expanded:py-0',
);

const POPOVER = 'rounded-medium bg-surface-container p-2 shadow-level2';

/** Shared calendar body; `cell` differs between single and range calendars. */
function CalendarGridBody() {
  return (
    <>
      <CalendarGrid>
        {(date) => (
          <CalendarCell
            date={date}
            className="typescale-body-medium m3-focus flex h-9 w-9 items-center justify-center rounded-full text-on-surface data-[selected]:bg-primary data-[selected]:text-on-primary data-[outside-month]:text-on-surface-variant data-[unavailable]:line-through"
          />
        )}
      </CalendarGrid>
    </>
  );
}

/** The calendar trigger; its accessible name comes from the catalog at the usage site. */
// Icon-only, so it gets the tooltip-with-the-same-text treatment from IconButton (§7.5).
function CalendarTrigger({ label, className }: { label: string; className?: string }) {
  return (
    <IconButton aria-label={label} className={cx('ml-auto', className)}>
      <Icon icon={icons.calendarMonth} />
    </IconButton>
  );
}

function DateSegment({ segment }: { segment: Parameters<typeof RacDateSegment>[0]['segment'] }) {
  return (
    <RacDateSegment
      segment={segment}
      className="typescale-body-large rounded-sm px-0.5 expanded:px-0 text-on-surface caret-primary outline-none data-[placeholder]:text-on-surface-variant data-[focused]:bg-secondary-container"
    />
  );
}

export interface DatePickerProps extends Omit<RacDatePickerProps<DateValue>, 'className'> {
  className?: string;
  /** Visible field label; the caller supplies catalog text (Addendum A 4). */
  label?: ReactNode;
  /** Accessible name of the calendar trigger (icon-only control, §7.5); catalog text. */
  calendarLabel: string;
}

/** M3 outlined date picker (Gregorian only; WIB is the single TZ, §7.4). */
export function DatePicker({ label, calendarLabel, className, ...props }: DatePickerProps) {
  return (
    <RacDatePicker {...props} className={cx('flex flex-col gap-1', className)}>
      {label ? <Label>{label}</Label> : null}
      <Group className={FIELD}>
        <RacDateInput>{(segment) => <DateSegment segment={segment} />}</RacDateInput>
        <CalendarTrigger label={calendarLabel} />
      </Group>
      <Popover className={POPOVER}>
        <Dialog className="outline-none">
          <Calendar className="bg-surface-container text-on-surface">
            <CalendarGridBody />
          </Calendar>
        </Dialog>
      </Popover>
    </RacDatePicker>
  );
}

export interface DateRangePickerProps extends Omit<
  RacDateRangePickerProps<DateValue>,
  'className'
> {
  className?: string;
  label?: ReactNode;
  calendarLabel: string;
  /** Visible separator between the two fields; catalog text, passed at the usage site. */
  separator?: ReactNode;
}

/** M3 outlined date range picker for log filters (PNG S17b). */
export function DateRangePicker({
  label,
  calendarLabel,
  separator,
  className,
  ...props
}: DateRangePickerProps) {
  return (
    <RacDateRangePicker {...props} className={cx('flex flex-col gap-1', className)}>
      {label ? <Label>{label}</Label> : null}
      <Group className={RANGE_FIELD} data-testid="date-range-field">
        <RacDateInput slot="start">{(segment) => <DateSegment segment={segment} />}</RacDateInput>
        {separator ? <span className="px-1 text-on-surface-variant">{separator}</span> : null}
        <RacDateInput slot="end">{(segment) => <DateSegment segment={segment} />}</RacDateInput>
        <CalendarTrigger label={calendarLabel} className="expanded:-mr-2" />
      </Group>
      <Popover className={POPOVER}>
        <Dialog className="outline-none">
          <RangeCalendar className="bg-surface-container text-on-surface">
            <CalendarGridBody />
          </RangeCalendar>
        </Dialog>
      </Popover>
    </RacDateRangePicker>
  );
}
