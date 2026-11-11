import type { ReactNode } from 'react';
import { cx } from './cx';

export type CardVariant = 'elevated' | 'filled' | 'outlined' | 'container';

const VARIANT: Record<CardVariant, string> = {
  elevated: 'border border-outline-variant bg-surface shadow-level1',
  filled: 'bg-surface-container-highest',
  outlined: 'border border-outline-variant bg-surface-container-lowest',
  /** Kartu isi layar (acuan ARC-261): putih, border tipis, sudut 16 px (Figma 2:22107), shadow level1, tanpa padding. */
  container:
    'rounded-large border border-outline-variant bg-surface-container-lowest shadow-level1',
};

export interface CardProps {
  variant?: CardVariant;
  className?: string;
  children: ReactNode;
}

/** M3 card container. Interactive behaviour is composed with Button/IconButton at usage sites. */
export function Card({ variant = 'filled', className, children }: CardProps) {
  return (
    <div
      className={cx(
        variant === 'container' ? null : 'rounded-medium p-4',
        VARIANT[variant],
        className,
      )}
    >
      {children}
    </div>
  );
}
