import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

/**
 * Status label (extensions 7.1, replacing the earlier non-catalog `Badge`).
 * Short, non-interactive status text on a tinted container. The text comes from
 * the catalog (`enum.status_*`) at the usage site, never from the component.
 */
export type StatusLabelTone = 'neutral' | 'info' | 'success' | 'warning' | 'error';

/**
 * M3 baseline has no `success`/`warning` colour roles, so the tones reuse the
 * catalogued roles the way §7.1 `StatusLabel` allows: `tertiary-container` for a
 * positive state, `error-container` for warning and error. The text always names
 * the status (§7.5), so colour is never the only signal.
 */
const TONE: Record<StatusLabelTone, string> = {
  neutral: 'bg-surface-container-high text-on-surface',
  info: 'bg-secondary-container text-on-secondary-container',
  success: 'bg-tertiary-container text-on-tertiary-container',
  warning: 'bg-error-container text-on-error-container',
  error: 'bg-error-container text-on-error-container',
};

/** Tone → catalogued icon key; `Icon` resolves the component (icons stay in one place). */
const ICON: Record<StatusLabelTone, keyof typeof icons | null> = {
  neutral: null,
  info: 'info',
  success: 'checkCircle',
  warning: 'warning',
  error: 'error',
};

export function StatusLabel({
  tone = 'neutral',
  icon,
  className,
  children,
}: {
  tone?: StatusLabelTone;
  /** Defaults to the tone's icon; pass `null` to hide it. */
  icon?: keyof typeof icons | null;
  className?: string;
  children: ReactNode;
}) {
  const glyph = icon === undefined ? ICON[tone] : icon;
  return (
    <span
      className={cx(
        'typescale-label-medium inline-flex items-center gap-1 whitespace-nowrap rounded-small px-2 py-0.5',
        TONE[tone],
        className,
      )}
    >
      {glyph ? <Icon icon={icons[glyph]} className="h-4 w-4" /> : null}
      {children}
    </span>
  );
}
