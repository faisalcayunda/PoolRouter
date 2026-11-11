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
 * Acuan ARC-261: latar tint, border tipis, radius 12, padding 16. Ikon dan teks pesan ikut
 * membawa nada, jadi warna bukan satu-satunya penanda.
 */
const TONE: Record<
  NonNullable<InlineAlertProps['tone']>,
  { box: string; icon: keyof typeof icons }
> = {
  info: { box: 'border border-info/25 bg-info-container text-on-info-container', icon: 'info' },
  success: {
    box: 'border border-success/30 bg-success-container text-on-success-container',
    icon: 'checkCircle',
  },
  warning: {
    box: 'border border-warning bg-warning-container text-on-warning-container',
    icon: 'warning',
  },
  error: {
    box: 'border border-error/25 bg-error-container text-on-error-container',
    icon: 'error',
  },
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
