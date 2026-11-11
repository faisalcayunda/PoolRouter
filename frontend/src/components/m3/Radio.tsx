import {
  Label,
  Radio as RacRadio,
  RadioGroup as RacRadioGroup,
  type RadioGroupProps as RacRadioGroupProps,
  type RadioProps as RacRadioProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

export interface RadioProps extends Omit<RacRadioProps, 'className'> {
  className?: string;
  children?: ReactNode;
}

/** M3 radio button. Arrow keys move selection inside the group (§3.7). */
export function Radio({ className, children, ...props }: RadioProps) {
  return (
    <RacRadio
      {...props}
      className={cx(
        'm3-state m3-focus m3-disabled typescale-body-medium group flex cursor-pointer items-center gap-3 rounded-full py-1 text-on-surface',
        'data-[disabled]:text-on-surface/38',
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cx(
          'flex h-5 w-5 shrink-0 items-center justify-center rounded-full border-2 transition-colors',
          'border-on-surface-variant',
          'group-data-[selected]:border-primary-text',
          'group-data-[disabled]:border-on-surface/38',
        )}
      >
        <span
          className={cx(
            'h-2.5 w-2.5 rounded-full bg-primary-text opacity-0 transition-opacity',
            'group-data-[selected]:opacity-100',
          )}
        />
      </span>
      {children}
    </RacRadio>
  );
}

export interface RadioGroupProps extends Omit<RacRadioGroupProps, 'className'> {
  className?: string;
  label?: ReactNode;
  children: ReactNode;
}

/** M3 radio group with an associated group label. */
export function RadioGroup({ label, children, className, ...props }: RadioGroupProps) {
  return (
    <RacRadioGroup {...props} className={cx('flex flex-col gap-2', className)}>
      {label ? <Label className="typescale-body-medium text-on-surface">{label}</Label> : null}
      {children}
    </RacRadioGroup>
  );
}
