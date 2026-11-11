import type { ReactNode } from 'react';
import { Link } from 'react-aria-components';
import { cx } from '../../components/m3/cx';

/**
 * Tautan bergaya `Button` M3. Pindah layar adalah navigasi (tautan), bukan aksi, jadi elemen tetap
 * `<a href>` supaya bisa dibuka di tab baru dan dibaca screen reader sebagai tautan.
 */
const VARIANT = {
  filled: 'bg-primary px-6 text-on-primary',
  outlined: 'border border-outline px-6 text-primary',
  text: 'px-3 text-primary',
} as const;

export function LinkButton({
  href,
  variant = 'filled',
  className,
  'aria-label': ariaLabel,
  newTab = false,
  children,
}: {
  href: string;
  variant?: keyof typeof VARIANT;
  className?: string;
  'aria-label'?: string;
  /** Buka di tab baru, mis. supaya isian form yang belum disimpan tidak hilang. */
  newTab?: boolean;
  children: ReactNode;
}) {
  return (
    <Link
      href={href}
      aria-label={ariaLabel}
      target={newTab ? '_blank' : undefined}
      rel={newTab ? 'noopener noreferrer' : undefined}
      className={cx(
        'm3-state m3-focus typescale-label-large inline-flex h-10 min-w-12 items-center justify-center gap-2 rounded-full no-underline',
        VARIANT[variant],
        className,
      )}
    >
      {children}
    </Link>
  );
}
