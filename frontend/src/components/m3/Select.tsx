import {
  Button as RacButton,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select as RacSelect,
  SelectValue,
  type SelectProps as RacSelectProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

export interface SelectProps<T extends object>
  extends Omit<RacSelectProps<T>, 'className'> {
  className?: string;
  label?: ReactNode;
  /** Items for the underlying ListBox (RAC dynamic collection). */
  items?: T[] | Iterable<T>;
  children: ReactNode;
}

/** M3 outlined select: value button + popover listbox. Keyboard: Enter opens, arrows navigate. */
export function Select<T extends object>({
  label,
  items,
  children,
  className,
  ...props
}: SelectProps<T>) {
  return (
    <RacSelect
      {...props}
      className={cx('flex flex-col gap-1',
        className,
      )}
    >
      {label ? <Label>{label}</Label> : null}
      <RacButton className="m3-state m3-focus typescale-body-large flex h-14 w-64 items-center justify-between rounded-extra-small border border-outline bg-surface px-4 text-on-surface data-[focused]:border-2 data-[focused]:border-primary data-[pressed]:border-2 data-[pressed]:border-primary">
        <SelectValue className="data-[placeholder]:text-on-surface-variant" />
        <Icon icon={icons.arrowDropDown} className="text-on-surface-variant" />
      </RacButton>
      <Popover className="min-w-(--trigger-width) rounded-medium bg-surface-container shadow-level2">
        <ListBox
          items={items}
          className="typescale-body-large max-h-72 overflow-auto p-2 text-on-surface outline-none"
        >
          {children}
        </ListBox>
      </Popover>
    </RacSelect>
  );
}

export function SelectItem({
  children,
  id,
  textValue,
}: {
  children: ReactNode;
  id: string | number;
  textValue?: string;
}) {
  return (
    <ListBoxItem
      id={id}
      textValue={textValue}
      className={cx(
        'm3-state m3-focus typescale-body-large flex cursor-pointer items-center justify-between rounded-small px-3 py-2.5 text-on-surface outline-none',
        'data-[focused]:bg-on-surface/8 data-[selected]:text-primary',
      )}
    >
      {children}
    </ListBoxItem>
  );
}
