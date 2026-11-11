import { Button as RacButton, type ButtonProps as RacButtonProps } from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { Tooltip, TooltipTrigger } from './Tooltip';

export type IconButtonVariant = 'standard' | 'filled' | 'filled-tonal' | 'outlined';

const VARIANT: Record<IconButtonVariant, string> = {
  standard: 'text-on-surface-variant',
  filled:
    'bg-primary text-on-primary group-data-[disabled]:bg-on-surface/12 group-data-[disabled]:text-on-surface/38',
  'filled-tonal':
    'bg-secondary-container text-on-secondary-container group-data-[disabled]:bg-on-surface/12 group-data-[disabled]:text-on-surface/38',
  outlined:
    'border border-outline-soft text-on-surface-variant group-data-[disabled]:border-on-surface/12 group-data-[disabled]:text-on-surface/38',
};

export interface IconButtonProps extends Omit<RacButtonProps, 'className' | 'children'> {
  /** The icon; the accessible name comes from aria-label, not from this node. */
  children?: ReactNode;
  className?: string;
  variant?: IconButtonVariant;
  /**
   * Icon-only control: the accessible name is mandatory and doubles as the
   * tooltip text, so the two can never disagree (ADR-0003 7.5).
   */
  'aria-label': string;
  /** Ukuran visual; target sentuh 48 px, kecuali 32 (dipakai di dalam kolom 40 px) yang memakai target 32 px. */
  size?: 32 | 40 | 48;
}

/**
 * M3 icon button, four F0 variants. The label is required because there is no
 * text, and it is also shown as a plain tooltip: §7.5 asks every icon-only
 * control to have both, with the same words. Building the tooltip here means a
 * call site cannot ship the label without it.
 */
export function IconButton({
  variant = 'standard',
  size = 40,
  className,
  'aria-label': ariaLabel,
  children,
  ...props
}: IconButtonProps) {
  return (
    <TooltipTrigger>
      <RacButton
        {...props}
        aria-label={ariaLabel}
        className={cx(
          // 48px touch target around a 40px (or 48px) visual circle, per §7.5.
          'm3-state-host m3-focus m3-disabled group inline-flex shrink-0 cursor-pointer items-center justify-center rounded-small select-none',
          size === 32 ? 'h-8 w-8' : 'h-12 w-12',
          className,
        )}
      >
        <span
          className={cx(
            'm3-state typescale-label-large inline-flex items-center justify-center rounded-small',
            size === 32 ? 'h-8 w-8' : size === 40 ? 'h-10 w-10' : 'h-12 w-12',
            VARIANT[variant],
          )}
        >
          {children}
        </span>
      </RacButton>
      <Tooltip>{ariaLabel}</Tooltip>
    </TooltipTrigger>
  );
}
