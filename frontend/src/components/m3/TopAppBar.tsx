import type { ReactNode } from 'react';
import { cx } from './cx';

/**
 * M3 top app bar (katalog 7.1).
 *
 * Slot title berupa blok (`div`) supaya bisa memuat breadcrumb pada UI v2 (bar Penpot berisi
 * breadcrumb, bukan judul halaman). The title is **text, not a heading** (handoff spec section 13.1): each screen's
 * `<main>` opens with its own `h1`, so a heading here would give the document two level-1
 * headings. Leading controls get their accessible label at the usage site.
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
      <div className="typescale-title-large min-w-0 flex-1">{title}</div>
      {trailing}
    </header>
  );
}
