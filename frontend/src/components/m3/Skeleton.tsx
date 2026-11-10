import { cx } from './cx';

/** Skeleton placeholder for loading rows and cards (ADR-0003 7.6); Penpot S16 uses radius 4 and surface-container-high. */
export function Skeleton({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cx('m3-skeleton rounded-extra-small bg-surface-container-high', className)}
    />
  );
}
