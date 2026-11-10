import type { ReactNode } from 'react';
import { cx } from './cx';

export type CardVariant = 'elevated' | 'filled' | 'outlined' | 'container';

const VARIANT: Record<CardVariant, string> = {
  elevated: 'bg-surface-container-low shadow-level1',
  filled: 'bg-surface-container-highest',
  outlined: 'border border-outline-variant bg-surface',
  /** Kartu isi layar Penpot (F0 / S03*, S04*): latar `surface-container-lowest`, sudut 16 px, tanpa padding. */
  container: 'rounded-large border border-outline-variant bg-surface-container-lowest',
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
