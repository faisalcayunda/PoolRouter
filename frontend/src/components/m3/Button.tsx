import { Button as RacButton, type ButtonProps as RacButtonProps } from 'react-aria-components';
import { cx } from './cx';

export type ButtonVariant = 'filled' | 'filled-tonal' | 'outlined' | 'text';

const VARIANT: Record<ButtonVariant, string> = {
  filled: 'bg-primary text-on-primary data-[disabled]:bg-on-surface/12',
  'filled-tonal':
    'bg-secondary-container text-on-secondary-container data-[disabled]:bg-on-surface/12',
  outlined: 'border border-outline text-primary data-[disabled]:border-on-surface/12',
  text: 'text-primary px-3',
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

/** M3 common button. One `filled` per area; `elevated` is not in the F0 catalog. */
export function Button({
  variant = 'filled',
  destructive = false,
  className,
  type = 'button',
  ...props
}: ButtonProps) {
  const tone = destructive
    ? variant === 'filled'
      ? 'bg-error text-on-error'
      : 'text-error'
    : VARIANT[variant];
  return (
    <RacButton
      {...props}
      type={type}
      className={cx(
        'm3-state m3-focus m3-disabled typescale-label-large inline-flex h-10 min-w-12 items-center justify-center gap-2 rounded-full px-6',
        'cursor-pointer select-none',
        variant === 'filled' && 'data-[disabled]:text-on-surface/38',
        variant === 'filled-tonal' && 'data-[disabled]:text-on-surface/38',
        variant === 'text' ? 'px-3' : null,
        tone,
        destructive && variant === 'outlined' && 'border border-error',
        className,
      )}
    />
  );
}
