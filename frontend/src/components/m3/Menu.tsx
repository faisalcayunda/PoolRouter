import {
  Menu as RacMenu,
  MenuItem as RacMenuItem,
  MenuTrigger as RacMenuTrigger,
  Popover,
  Separator,
  type MenuItemProps,
  type MenuProps as RacMenuProps,
  type MenuTriggerProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

export function MenuTrigger({ children, ...props }: MenuTriggerProps & { children: ReactNode }) {
  return <RacMenuTrigger {...props}>{children}</RacMenuTrigger>;
}

export interface MenuProps<T extends object = object>
  extends Omit<RacMenuProps<T>, 'className'> {
  className?: string;
  children: ReactNode;
}

/** M3 menu inside a popover; keyboard: arrows navigate, Enter selects, Esc closes. */
export function Menu<T extends object = object>({
  children,
  className,
  ...props
}: MenuProps<T>) {
  return (
    <Popover
      className={cx(
        'min-w-44 rounded-extra-small bg-surface-container p-2 shadow-level2 outline-none',
        className,
      )}
    >
      <RacMenu
        {...props}
        className="typescale-label-large max-h-72 overflow-auto text-on-surface outline-none"
      >
        {children}
      </RacMenu>
    </Popover>
  );
}

export function MenuItem({ className, ...props }: Omit<MenuItemProps, 'className'> & { className?: string }) {
  return (
    <RacMenuItem
      {...props}
      className={cx(
        'm3-state m3-focus typescale-label-large flex cursor-pointer items-center gap-3 rounded-small px-3 py-2.5 text-on-surface outline-none select-none',
        'data-[focused]:bg-on-surface/8 data-[disabled]:text-on-surface/38',
        className,
      )}
    />
  );
}

export function MenuSeparator() {
  return <Separator className="mx-2 my-1 h-px border-t border-outline-variant" />;
}
