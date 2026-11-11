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
 * Pill acuan ARC-261: latar tint, border tipis dari warna status, teks gelap supaya kontras AA.
 * Teks selalu menyebut statusnya (§7.5), jadi warna bukan satu-satunya penanda.
 */
const TONE: Record<StatusLabelTone, string> = {
  neutral: 'border border-outline-variant bg-surface-container text-on-surface',
  info: 'border border-info/30 bg-info-container text-on-info-container',
  success: 'border border-success/40 bg-success-container text-on-success-container',
  warning: 'border border-warning/40 bg-warning-container text-on-warning-container',
  error: 'border border-error/30 bg-error-container text-on-error-container',
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
        'typescale-label-medium inline-flex h-6 items-center gap-1 whitespace-nowrap rounded-full px-2.5',
        TONE[tone],
        className,
      )}
    >
      {glyph ? <Icon icon={icons[glyph]} className="h-3.5 w-3.5" /> : null}
      {children}
    </span>
  );
}
