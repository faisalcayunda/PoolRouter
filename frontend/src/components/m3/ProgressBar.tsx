import {
  ProgressBar as RacProgressBar,
  type ProgressBarProps,
} from 'react-aria-components';
import { cx } from './cx';

/** M3 linear progress. Value is optional for indeterminate use. */
export function ProgressBar({ className, ...props }: Omit<ProgressBarProps,'className'> & { className?: string }) {
  return (
    <RacProgressBar
      {...props}
      className={cx('typescale-body-small flex flex-col gap-1 text-on-surface-variant', className)}
    >
      {({ percentage, valueText }) => (
        <>
          {valueText ? <span>{valueText}</span> : null}
          <div className="h-1 w-full overflow-hidden rounded-full bg-secondary-container">
            <div
              className="h-full bg-primary"
              style={{ width: `${percentage ?? 0}%` }}
              data-indeterminate={percentage === undefined || undefined}
            />
          </div>
        </>
      )}
    </RacProgressBar>
  );
}
