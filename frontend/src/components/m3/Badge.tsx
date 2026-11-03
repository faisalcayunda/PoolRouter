import type { ReactNode } from 'react';
import { cx } from './cx';

/** Small count badge (ekstensi lokal E-2): shown on navigation items. */
export function Badge({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cx(
        'typescale-label-medium inline-flex h-4.5 min-w-4.5 items-center justify-center rounded-full bg-error px-1 text-on-error',
        className,
      )}
    >
      {children}
    </span>
  );
}
