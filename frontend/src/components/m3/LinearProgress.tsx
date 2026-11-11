import { ProgressBar as RacProgressBar, type ProgressBarProps } from 'react-aria-components';
import { cx } from './cx';

/**
 * M3 linear progress indicator (katalog 7.1).
 * Indeterminate when `isIndeterminate` is set or no `value` is given.
 * Kedua ukuran mengikuti Figma 2:21304: trek `surface-container`, isi `accent`. Standard 4 px dengan
 * angka di atas bilah; `size="large"` 8 px untuk kartu metrik, tanpa angka (angkanya di teks kartu).
 */
export function LinearProgress({
  className,
  isIndeterminate,
  size = 'standard',
  ...props
}: Omit<ProgressBarProps, 'className'> & { className?: string; size?: 'standard' | 'large' }) {
  const large = size === 'large';
  const indeterminate = isIndeterminate || props.value === undefined;
  return (
    <RacProgressBar
      {...props}
      isIndeterminate={indeterminate}
      className={cx('typescale-body-small flex flex-col gap-1 text-on-surface-variant', className)}
    >
      {({ percentage, valueText }) => (
        <>
          {!indeterminate && !large && valueText ? <span>{valueText}</span> : null}
          <div
            className={cx(
              'w-full overflow-hidden rounded-full',
              large ? 'h-2 bg-surface-container' : 'h-1 bg-surface-container',
            )}
            data-indeterminate={indeterminate || undefined}
          >
            <div
              className={cx('h-full', 'bg-accent', indeterminate && 'm3-indeterminate-bar')}
              style={{ width: `${percentage ?? 0}%` }}
            />
          </div>
        </>
      )}
    </RacProgressBar>
  );
}
