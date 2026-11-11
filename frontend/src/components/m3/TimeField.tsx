import {
  TimeField as RacTimeField,
  DateInput,
  DateSegment as RacDateSegment,
  Group,
  Label,
  type TimeFieldProps,
  type TimeValue,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

const FIELD =
  'm3-focus typescale-body-large flex h-14 w-full items-center gap-1 rounded-extra-small border border-outline bg-surface px-4 text-on-surface data-[focus-within]:border-2 data-[focus-within]:border-secondary';

/**
 * M3 outlined time field (katalog 7.1). Carries time segments only; the zone is
 * WIB by policy (ADR-0003 7.4), so no offset is attached to the value.
 */
export function TimeField({
  label,
  className,
  ...props
}: Omit<TimeFieldProps<TimeValue>, 'className'> & { className?: string; label?: ReactNode }) {
  return (
    <RacTimeField {...props} className={cx('flex flex-col gap-1', className)}>
      {label ? <Label>{label}</Label> : null}
      <Group className={FIELD}>
        <DateInput>{(segment) => <DateSegment segment={segment} />}</DateInput>
      </Group>
    </RacTimeField>
  );
}

function DateSegment({ segment }: { segment: Parameters<typeof RacDateSegment>[0]['segment'] }) {
  return (
    <RacDateSegment
      segment={segment}
      className="typescale-body-large rounded-sm px-0.5 text-on-surface caret-primary-text outline-none data-[placeholder]:text-on-surface-variant data-[focused]:bg-secondary-container"
    />
  );
}
