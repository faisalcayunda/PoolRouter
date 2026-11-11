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

export interface SelectProps<T extends object> extends Omit<RacSelectProps<T>, 'className'> {
  className?: string;
  label?: ReactNode;
  /** Items for the underlying ListBox (RAC dynamic collection). */
  items?: T[] | Iterable<T>;
  children: ReactNode;
}

/** Dropdown (acuan ARC-261): tombol nilai outline + popover kartu putih. Keyboard: Enter membuka, panah memilih. */
export function Select<T extends object>({
  label,
  items,
  children,
  className,
  ...props
}: SelectProps<T>) {
  return (
    <RacSelect {...props} className={cx('flex flex-col gap-1', className)}>
      {label ? <Label className="typescale-label-large text-on-surface">{label}</Label> : null}
      <RacButton className="typescale-body-medium flex h-10 w-64 cursor-pointer items-center justify-between rounded-small border border-outline bg-surface-container-lowest pr-2 pl-3 text-on-surface outline-none data-[focused]:border-secondary data-[focused]:ring-3 data-[focused]:ring-accent-container data-[pressed]:border-secondary">
        <SelectValue className="data-[placeholder]:text-on-surface-variant" />
        <Icon icon={icons.arrowDropDown} className="text-on-surface-variant" />
      </RacButton>
      <Popover className="min-w-(--trigger-width) rounded-medium border border-outline-variant bg-surface-container-lowest shadow-level2">
        <ListBox
          items={items}
          className="typescale-body-medium max-h-72 overflow-auto p-1 text-on-surface outline-none"
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
        'typescale-body-medium flex cursor-pointer items-center justify-between rounded-small px-3 py-2 text-on-surface outline-none',
        'data-[focused]:bg-surface-container data-[selected]:font-semibold data-[focus-visible]:outline-solid data-[focus-visible]:outline-3 data-[focus-visible]:-outline-offset-3 data-[focus-visible]:outline-secondary',
      )}
    >
      {children}
    </ListBoxItem>
  );
}
