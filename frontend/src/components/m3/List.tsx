import {
  ListBox as RacListBox,
  ListBoxItem as RacListBoxItem,
  type ListBoxItemProps,
  type ListBoxProps,
} from 'react-aria-components';
import { cx } from './cx';

/** M3 list built on RAC ListBox (semantic listbox; used for settings lists and audit rows). */
export function List<T extends object>({ className, ...props }: Omit<ListBoxProps<T>, 'className'> & { className?: string }) {
  return <RacListBox {...props} className={cx('flex flex-col py-2 outline-none', className)} />;
}

export function ListItem({ className, ...props }: Omit<ListBoxItemProps, 'className'> & { className?: string }) {
  return (
    <RacListBoxItem
      {...props}
      className={cx(
        'm3-state m3-focus typescale-body-large cursor-pointer rounded-extra-small px-4 py-3 text-on-surface outline-none',
        'data-[disabled]:text-on-surface/38 data-[disabled]:cursor-default',
        className,
      )}
    />
  );
}

export function ListHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      {...props}
      className={cx('typescale-title-small px-4 py-2 text-on-surface-variant', className)}
    />
  );
}

export type { ListBoxItemProps };
