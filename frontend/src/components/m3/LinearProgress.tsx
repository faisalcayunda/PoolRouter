import { ProgressBar as RacProgressBar, type ProgressBarProps } from 'react-aria-components';
import { cx } from './cx';

/**
 * M3 linear progress indicator (katalog 7.1).
 * Indeterminate when `isIndeterminate` is set or no `value` is given.
 * `size="large"` mengikuti bilah kartu metrik Penpot: trek 8 px `surface-container-low`, isi
 * `primary-container`, tanpa angka di atas bilah (angkanya tertulis di teks kartu).
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
              large ? 'h-2 bg-surface-container-low' : 'h-1 bg-secondary-container',
            )}
            data-indeterminate={indeterminate || undefined}
          >
            <div
              className={cx(
                'h-full',
                large ? 'bg-primary-container' : 'bg-primary',
                indeterminate && 'm3-indeterminate-bar',
              )}
              style={{ width: `${percentage ?? 0}%` }}
            />
          </div>
        </>
      )}
    </RacProgressBar>
  );
}
