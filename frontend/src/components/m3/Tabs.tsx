import {
  Tab as RacTab,
  TabList as RacTabList,
  TabPanel as RacTabPanel,
  Tabs as RacTabs,
  type TabListProps,
  type TabPanelProps,
  type TabProps,
  type TabsProps,
} from 'react-aria-components';
import { cx } from './cx';

export function Tabs({
  className,
  ...props
}: Omit<TabsProps, 'className'> & { className?: string }) {
  return <RacTabs {...props} className={cx('flex flex-col gap-2', className)} />;
}

/** Primary tabs: full-width items with the active indicator under the label. */
export function TabList({
  className,
  ...props
}: Omit<TabListProps<object>, 'className'> & { className?: string }) {
  return (
    <RacTabList
      {...props}
      className={cx(
        'flex items-stretch gap-2 overflow-x-auto border-b border-outline-variant',
        className,
      )}
    />
  );
}

export function Tab({ className, ...props }: Omit<TabProps, 'className'> & { className?: string }) {
  return (
    <RacTab
      {...props}
      className={cx(
        'm3-state m3-focus typescale-label-large relative cursor-pointer px-4 pt-1 pb-2.5 text-on-surface-variant outline-none select-none',
        'data-[selected]:text-on-surface',
        'after:absolute after:inset-x-4 after:bottom-0 after:h-0.5 after:rounded-full after:bg-transparent',
        'data-[selected]:after:bg-primary-text',
        className,
      )}
    />
  );
}

export function TabPanel({
  className,
  ...props
}: Omit<TabPanelProps, 'className'> & { className?: string }) {
  return (
    <RacTabPanel
      {...props}
      className={cx(
        'typescale-body-medium py-2 text-on-surface outline-none',
        // `shouldForceMount` menyisakan panel tak aktif di DOM (hanya `inert`); sembunyikan dari layar.
        'data-[inert]:hidden',
        className,
      )}
    />
  );
}
