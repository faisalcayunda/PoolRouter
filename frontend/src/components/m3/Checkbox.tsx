import {
  Checkbox as RacCheckbox,
  CheckboxGroup as RacCheckboxGroup,
  Label,
  type CheckboxGroupProps as RacCheckboxGroupProps,
  type CheckboxProps as RacCheckboxProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

export interface CheckboxProps extends Omit<RacCheckboxProps, 'className'> {
  className?: string;
  children?: ReactNode;
}

/** M3 checkbox. Keyboard: Space toggles (§3.7). RAC exposes data-selected/data-indeterminate. */
export function Checkbox({ className, children, ...props }: CheckboxProps) {
  return (
    <RacCheckbox
      {...props}
      className={cx('m3-state m3-focus m3-disabled typescale-body-medium group flex cursor-pointer items-center gap-3 rounded-extra-small py-1 text-on-surface',
          'data-[disabled]:text-on-surface/38',
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cx(
          'flex h-4.5 w-4.5 shrink-0 items-center justify-center rounded-[2px] border-2 transition-colors',
          'border-on-surface-variant bg-transparent',
          'group-data-[selected]:border-primary group-data-[selected]:bg-primary',
          'group-data-[indeterminate]:border-primary group-data-[indeterminate]:bg-primary',
          'group-data-[disabled]:border-on-surface/38 group-data-[disabled]:bg-transparent',
        )}
      >
        <Icon
          icon={icons.check}
          className={cx(
            'h-3.5 w-3.5 text-on-primary opacity-0 transition-opacity',
            'group-data-[selected]:opacity-100',
          )}
        />
      </span>
      {children}
    </RacCheckbox>
  );
}

export interface CheckboxGroupProps extends Omit<RacCheckboxGroupProps, 'className'> {
  className?: string;
  label?: ReactNode;
  children: ReactNode;
}

/** M3 checkbox group with an associated group label. */
export function CheckboxGroup({ label, children, className, ...props }: CheckboxGroupProps) {
  return (
    <RacCheckboxGroup
      {...props}
      className={cx('flex flex-col gap-2',
        className,
      )}
    >
      {label ? <Label className="typescale-body-medium text-on-surface">{label}</Label> : null}
      {children}
    </RacCheckboxGroup>
  );
}
