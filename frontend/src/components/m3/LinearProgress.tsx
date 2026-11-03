import { ProgressBar as RacProgressBar, type ProgressBarProps } from 'react-aria-components';
import { cx } from './cx';

/**
 * M3 linear progress indicator (katalog 7.1).
 * Indeterminate when `isIndeterminate` is set or no `value` is given.
 */
export function LinearProgress({
  className,
  isIndeterminate,
  ...props
}: Omit<ProgressBarProps, 'className'> & { className?: string }) {
  const indeterminate = isIndeterminate || props.value === undefined;
  return (
    <RacProgressBar
      {...props}
      isIndeterminate={indeterminate}
      className={cx('typescale-body-small flex flex-col gap-1 text-on-surface-variant', className)}
    >
      {({ percentage, valueText }) => (
        <>
          {!indeterminate && valueText ? <span>{valueText}</span> : null}
          <div
            className="h-1 w-full overflow-hidden rounded-full bg-secondary-container"
            data-indeterminate={indeterminate || undefined}
          >
            <div
              className={cx('h-full bg-primary', indeterminate && 'm3-indeterminate-bar')}
              style={{ width: `${percentage ?? 0}%` }}
            />
          </div>
        </>
      )}
    </RacProgressBar>
  );
}
