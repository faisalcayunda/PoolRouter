import type { ReactNode } from 'react';
import { cx } from './cx';

/**
 * M3 top app bar (katalog 7.1). Renders a heading at level 1 so every screen has
 * exactly one; leading controls get their accessible label at the usage site.
 */
export interface TopAppBarProps {
  /** Screen title. Supplied from the catalog (`*.page.title`) by the usage site. */
  title: ReactNode;
  /** Leading control, usually the modal-drawer `IconButton`. */
  leading?: ReactNode;
  /** Trailing controls, e.g. the account menu. */
  trailing?: ReactNode;
  className?: string;
}

export function TopAppBar({ title, leading, trailing, className }: TopAppBarProps) {
  return (
    <header
      className={cx('flex min-h-16 items-center gap-2 bg-surface px-4 text-on-surface', className)}
    >
      {leading}
      <h1 className="typescale-title-large flex-1 truncate">{title}</h1>
      {trailing}
    </header>
  );
}
