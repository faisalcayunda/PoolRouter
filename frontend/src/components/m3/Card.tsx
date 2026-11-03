import type { ReactNode } from 'react';
import { cx } from './cx';

export type CardVariant = 'elevated' | 'filled' | 'outlined';

const VARIANT: Record<CardVariant, string> = {
  elevated: 'bg-surface-container-low shadow-level1',
  filled: 'bg-surface-container-highest',
  outlined: 'border border-outline-variant bg-surface',
};

export interface CardProps {
  variant?: CardVariant;
  className?: string;
  children: ReactNode;
}

/** M3 card container. Interactive behaviour is composed with Button/IconButton at usage sites. */
export function Card({ variant = 'filled', className, children }: CardProps) {
  return (
    <div className={cx('rounded-medium p-4', VARIANT[variant], className)}>{children}</div>
  );
}
