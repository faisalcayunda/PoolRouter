import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

/**
 * Inline alert (extensions 7.1): a message that stays in the flow of the page,
 * used for a failed save or reload (Addendum A 7.3 and 8.2).
 *
 * `live="assertive"` is for failures the user must act on now; the default
 * `polite` announces on the next opportunity. The heading is optional because
 * short field-level messages are often one line.
 */
export interface InlineAlertProps {
  tone?: 'info' | 'success' | 'warning' | 'error';
  /** Optional bold first line; the message itself goes in `children`. */
  heading?: ReactNode;
  /** Optional action, e.g. a retry `Button`. */
  action?: ReactNode;
  live?: 'polite' | 'assertive';
  className?: string;
  children: ReactNode;
}

/**
 * Penpot S16 fills `info` and `success` with the `info-container` and `success-container`
 * tokens (radius 12, padding 16). There is no `warning-container` token, so warning reuses
 * `error-container`. The icon and the message text carry the tone as well.
 */
const TONE: Record<
  NonNullable<InlineAlertProps['tone']>,
  { box: string; icon: keyof typeof icons }
> = {
  info: { box: 'bg-info-container text-on-info-container', icon: 'info' },
  success: { box: 'bg-success-container text-on-success-container', icon: 'checkCircle' },
  warning: { box: 'bg-error-container text-on-error-container', icon: 'warning' },
  error: { box: 'bg-error-container text-on-error-container', icon: 'error' },
};

export function InlineAlert({
  tone = 'error',
  heading,
  action,
  live = 'polite',
  className,
  children,
}: InlineAlertProps) {
  const { box, icon } = TONE[tone];
  return (
    <div
      role="alert"
      aria-live={live}
      className={cx(
        'typescale-body-medium flex items-start gap-2 rounded-medium p-4',
        box,
        className,
      )}
    >
      <Icon icon={icons[icon]} className="mt-0.5 h-5 w-5 shrink-0" />
      <div className="flex flex-1 flex-col gap-1">
        {heading ? <p className="typescale-title-small">{heading}</p> : null}
        <div>{children}</div>
        {action ? <div className="pt-1">{action}</div> : null}
      </div>
    </div>
  );
}
