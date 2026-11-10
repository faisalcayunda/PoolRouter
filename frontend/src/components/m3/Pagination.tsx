import { cx } from './cx';
import { Button } from './Button';
import { IconButton } from './IconButton';
import { Icon, icons } from '../../icons';

export interface PaginationProps {
  /** 1-based current page. */
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
  /** Catalog text for prev/next labels (common.pagination). */
  prevLabel: string;
  nextLabel: string;
  /**
   * Accessible name of the pagination landmark. Section 13.2 requires it to name the table it
   * paginates ("Paginasi log"), so two tables on one page are told apart.
   */
  label?: string;
  /** Text "1–25 dari 240", announced politely after a page change (section 13.2). */
  rangeText?: string;
  className?: string;
  /**
   * `icon` (default) is the compact chevron pair; `text` is the Penpot S11 footer: range text on
   * the left and two outlined buttons that show `prevLabel` / `nextLabel` as visible text.
   */
  variant?: 'icon' | 'text';
}

/** Pagination (ekstensi lokal E-4): prev/next icon buttons + range text in a named landmark. */
export function Pagination({
  page,
  pageCount,
  onPageChange,
  prevLabel,
  nextLabel,
  label,
  rangeText,
  className,
  variant = 'icon',
}: PaginationProps) {
  if (variant === 'text') {
    return (
      <nav
        aria-label={label ?? `${prevLabel} / ${nextLabel}`}
        className={cx('flex items-center justify-between gap-4', className)}
      >
        <p role="status" className="typescale-body-medium text-on-surface-variant">
          {rangeText}
        </p>
        <div className="flex items-center gap-2">
          <Button variant="outlined" onPress={() => onPageChange(page - 1)} isDisabled={page <= 1}>
            {prevLabel}
          </Button>
          <Button
            variant="outlined"
            onPress={() => onPageChange(page + 1)}
            isDisabled={page >= pageCount}
          >
            {nextLabel}
          </Button>
        </div>
      </nav>
    );
  }
  return (
    <nav
      aria-label={label ?? `${prevLabel} / ${nextLabel}`}
      className={cx('flex items-center justify-end gap-1', className)}
    >
      {rangeText ? (
        <p role="status" className="typescale-body-small text-on-surface-variant">
          {rangeText}
        </p>
      ) : null}
      <IconButton
        aria-label={prevLabel}
        onPress={() => onPageChange(page - 1)}
        isDisabled={page <= 1}
      >
        <Icon icon={icons.chevronLeft} />
      </IconButton>
      <IconButton
        aria-label={nextLabel}
        onPress={() => onPageChange(page + 1)}
        isDisabled={page >= pageCount}
      >
        <Icon icon={icons.chevronRight} />
      </IconButton>
    </nav>
  );
}
