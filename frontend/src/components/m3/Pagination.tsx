import { cx } from './cx';
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
  className?: string;
}

/** Pagination (ekstensi lokal E-4): prev/next icon buttons + range text. */
export function Pagination({
  page,
  pageCount,
  onPageChange,
  prevLabel,
  nextLabel,
  className,
}: PaginationProps) {
  return (
    <nav
      aria-label={`${prevLabel} / ${nextLabel}`}
      className={cx('flex items-center justify-end gap-1', className)}
    >
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
