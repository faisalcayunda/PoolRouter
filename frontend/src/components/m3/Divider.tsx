import { cx } from './cx';

/** M3 full-width divider (§7.1). */
export function Divider({
  inset = false,
  className,
}: {
  inset?: boolean;
  className?: string;
}) {
  return (
    <hr
      className={cx(
        'border-t border-outline-variant',
        inset && 'ml-16',
        className,
      )}
    />
  );
}
