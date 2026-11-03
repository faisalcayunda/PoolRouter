import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, type IconComponent } from '../../icons';

/**
 * Empty state (ekstensi lokal E-3): icon + title + body + optional retry action.
 * Text is supplied from the catalog at the usage site.
 */
export function EmptyState({
  icon,
  title,
  body,
  action,
  className,
}: {
  icon?: IconComponent;
  title: ReactNode;
  body?: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cx(
        'typescale-body-medium flex flex-col items-center gap-2 px-6 py-10 text-center text-on-surface-variant',
        className,
      )}
    >
      {icon ? <Icon icon={icon} className="h-10 w-10 text-on-surface-variant" /> : null}
      <p className="typescale-title-medium text-on-surface">{title}</p>
      {body ? <p>{body}</p> : null}
      {action ? <div className="pt-2">{action}</div> : null}
    </div>
  );
}
