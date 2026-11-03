import {
  Calendar,
  CalendarCell,
  CalendarGrid,
  DateSegment as RacDateSegment,
  DatePicker as RacDatePicker,
  DateRangePicker as RacDateRangePicker,
  DateInput,
  Dialog,
  Group,
  Label,
  Popover,
  type DatePickerProps as RacDatePickerProps,
  type DateRangePickerProps as RacDateRangePickerProps,
} from 'react-aria-components';
import type { DateValue } from '@internationalized/date';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

const FIELD =
  'm3-focus typescale-body-large flex h-14 w-full items-center rounded-extra-small border border-outline bg-surface px-4 text-on-surface group-data-[focused]:border-2 group-data-[focused]:border-primary';

/** M3 outlined date picker (Gregorian only; WIB is the single TZ, §7.4). */
export function DatePicker({
  label,
  className,
  ...props
}: Omit<RacDatePickerProps<DateValue>, 'className'> & { className?: string; label?: ReactNode }) {
  return (
    <RacDatePicker
      {...props}
      className={cx('flex flex-col gap-1', className)}
    >
      {label ? <Label>{label}</Label> : null}
      <Group className={FIELD}>
        <DateInput>{(segment) => <DateSegment key={segment.type} segment={segment} />}</DateInput>
      </Group>
      <Popover className="rounded-medium bg-surface-container p-2 shadow-level2">
        <Dialog className="outline-none">
          <Calendar className="bg-surface-container text-on-surface">
            <CalendarGrid>{(date) => <CalendarCell date={date} />}</CalendarGrid>
          </Calendar>
        </Dialog>
      </Popover>
    </RacDatePicker>
  );
}

function DateSegment({ segment }: { segment: Parameters<typeof RacDateSegment>[0]['segment'] }) {
  return (
    <RacDateSegment
      segment={segment}
      className="typescale-body-large rounded-sm px-0.5 text-on-surface caret-primary outline-none data-[placeholder]:text-on-surface-variant data-[focused]:bg-secondary-container"
    />
  );
}

/** M3 outlined date range picker for log filters (PNG S17b). */
export function DateRangePicker({
  label,
  className,
  ...props
}: Omit<RacDateRangePickerProps<DateValue>, 'className'> & { className?: string; label?: ReactNode }) {
  return (
    <RacDateRangePicker
      {...props}
      className={cx('flex flex-col gap-1', className)}
    >
      {label ? <Label>{label}</Label> : null}
      <Group className={FIELD}>
        <DateInput>{(segment) => <DateSegment key={segment.type} segment={segment} />}</DateInput>
      </Group>
      <Popover className="rounded-medium bg-surface-container p-2 shadow-level2">
        <Dialog className="outline-none">
          <Calendar className="bg-surface-container text-on-surface">
            <CalendarGrid>{(date) => <CalendarCell date={date} />}</CalendarGrid>
          </Calendar>
        </Dialog>
      </Popover>
    </RacDateRangePicker>
  );
}

// Icon kept for the picker trigger composition at usage sites.
export function DatePickerIcon() {
  return <Icon icon={icons.calendarMonth} className="text-on-surface-variant" />;
}
