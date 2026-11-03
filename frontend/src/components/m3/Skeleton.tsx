import { cx } from './cx';

/** Skeleton placeholder for loading rows and cards (ADR-0003 7.6). */
export function Skeleton({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cx('m3-skeleton rounded-medium bg-on-surface/12', className)}
    />
  );
}
