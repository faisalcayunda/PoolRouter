import { ProgressBar as RacProgressBar, type ProgressBarProps } from 'react-aria-components';
import { cx } from './cx';

const SIZE: Record<'small' | 'medium', string> = {
  small: 'h-4 w-4',
  medium: 'h-10 w-10',
};

/**
 * M3 circular progress indicator (katalog 7.1): determinate when `value` is set,
 * indeterminate otherwise. Used while a control is saving (Addendum A 7.3).
 */
export function CircularProgress({
  className,
  size = 'small',
  isIndeterminate,
  'aria-label': ariaLabel,
  ...props
}: Omit<ProgressBarProps, 'className'> & {
  className?: string;
  size?: 'small' | 'medium';
}) {
  const indeterminate = isIndeterminate || props.value === undefined;
  return (
    <RacProgressBar
      {...props}
      isIndeterminate={indeterminate}
      aria-label={ariaLabel}
      className={cx('inline-flex items-center justify-center text-primary-text', className)}
    >
      {({ percentage }) => (
        <svg
          aria-hidden="true"
          viewBox="0 0 24 24"
          className={cx(SIZE[size], indeterminate && 'm3-spin')}
          fill="none"
          stroke="currentColor"
        >
          <circle cx="12" cy="12" r="9" strokeWidth="3" className="opacity-25" />
          <circle
            cx="12"
            cy="12"
            r="9"
            strokeWidth="3"
            strokeLinecap="round"
            strokeDasharray={indeterminate ? '14 42' : '56.5 56.5'}
            strokeDashoffset={indeterminate ? 0 : 56.5 - (56.5 * (percentage ?? 0)) / 100}
          />
        </svg>
      )}
    </RacProgressBar>
  );
}
