import { Button as RacButton, type ButtonProps as RacButtonProps } from 'react-aria-components';
import { cx } from './cx';

export type IconButtonVariant = 'standard' | 'filled' | 'filled-tonal' | 'outlined';

const VARIANT: Record<IconButtonVariant, string> = {
  standard: 'text-on-surface-variant',
  filled: 'bg-primary text-on-primary data-[disabled]:bg-on-surface/12 data-[disabled]:text-on-surface/38',
  'filled-tonal':
    'bg-secondary-container text-on-secondary-container data-[disabled]:bg-on-surface/12 data-[disabled]:text-on-surface/38',
  outlined:
    'border border-outline text-on-surface-variant data-[disabled]:border-on-surface/12 data-[disabled]:text-on-surface/38',
};

export interface IconButtonProps extends Omit<RacButtonProps, 'className'> {
  className?: string;
  variant?: IconButtonVariant;
  size?: 40 | 48;
  /** Icon-only control: the accessible name is mandatory (M3 accessibility). */
  'aria-label': string;
}

/** M3 icon button, four F0 variants; label is required because there is no text. */
export function IconButton({
  variant = 'standard',
  size = 40,
  className,
  ...props
}: IconButtonProps) {
  return (
    <RacButton
      {...props}
      className={cx(
        'm3-state m3-focus m3-disabled typescale-label-large inline-flex shrink-0 cursor-pointer items-center justify-center rounded-full select-none',
        size === 40 ? 'h-10 w-10' : 'h-12 w-12',
        VARIANT[variant],
        className,
      )}
    />
  );
}
