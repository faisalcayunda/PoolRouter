import { Button as RacButton, type ButtonProps as RacButtonProps } from 'react-aria-components';
import { cx } from './cx';

export type ButtonVariant = 'filled' | 'filled-tonal' | 'outlined' | 'text';

const VARIANT: Record<ButtonVariant, string> = {
  filled: 'bg-primary text-on-primary data-[disabled]:bg-on-surface/12',
  'filled-tonal':
    'bg-secondary-container text-on-secondary-container data-[disabled]:bg-on-surface/12',
  outlined:
    'border border-outline-soft bg-surface-container-lowest text-on-surface data-[disabled]:border-on-surface/12',
  text: 'text-on-surface px-3',
};

export interface ButtonProps extends Omit<RacButtonProps, 'className'> {
  className?: string;
  variant?: ButtonVariant;
  /**
   * React Aria's button renders a native `<button>` but does not forward `type`, so a submit
   * button would silently never submit its form (the S11/S12 "Cari" buttons rely on this).
   * Declared explicitly and passed through as the native attribute.
   */
  type?: 'button' | 'submit' | 'reset';
  /** Use the `error` role for destructive confirmations (ADR-0003 7.2). */
  destructive?: boolean;
}

/** Tombol umum (acuan ARC-261): utama gelap, sekunder outline putih, destruktif tint merah; 13 semibold, radius 8. */
export function Button({
  variant = 'filled',
  destructive = false,
  className,
  type = 'button',
  ...props
}: ButtonProps) {
  const tone = destructive
    ? variant === 'filled'
      ? 'border border-error/25 bg-error-container text-on-error-container'
      : 'text-error'
    : VARIANT[variant];
  return (
    <RacButton
      {...props}
      type={type}
      className={cx(
        'm3-state m3-focus m3-disabled typescale-label-large inline-flex h-9 min-w-9 items-center justify-center gap-2 rounded-small px-4',
        'cursor-pointer select-none',
        variant === 'filled' && 'data-[disabled]:text-on-surface/38',
        variant === 'filled-tonal' && 'data-[disabled]:text-on-surface/38',
        variant === 'text' ? 'px-3' : null,
        tone,
        destructive && variant === 'outlined' && 'border border-error/40',
        className,
      )}
    />
  );
}
