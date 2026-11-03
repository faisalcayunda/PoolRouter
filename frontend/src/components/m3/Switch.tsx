import { Switch as RacSwitch, type SwitchProps as RacSwitchProps } from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

export interface SwitchProps extends Omit<RacSwitchProps, 'className'> {
  className?: string;
  children?: ReactNode;
}

/** M3 switch. Keyboard: Space toggles. RAC exposes data-selected and data-readonly. */
export function Switch({ className, children, ...props }: SwitchProps) {
  return (
    <RacSwitch
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
          'relative inline-flex h-8 w-13 shrink-0 items-center rounded-full border-2 px-0.5 transition-colors',
          'border-on-surface-variant bg-surface-container-highest',
          'group-data-[selected]:border-primary group-data-[selected]:bg-primary',
          'group-data-[disabled]:border-on-surface/12 group-data-[disabled]:bg-on-surface/12',
          'group-data-[focus-visible]:outline-2 group-data-[focus-visible]:outline-secondary group-data-[focus-visible]:outline-offset-2',
        )}
      >
        <span
          className={cx(
            'h-6 w-6 rounded-full bg-outline transition-transform',
            'group-data-[selected]:translate-x-5 group-data-[selected]:bg-on-primary',
            'group-data-[disabled]:bg-on-surface/38',
          )}
        />
      </span>
      {children}
    </RacSwitch>
  );
}
