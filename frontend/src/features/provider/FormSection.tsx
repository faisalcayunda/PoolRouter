import type { ReactNode } from 'react';
import { cx } from '../../components/m3/cx';

/**
 * Bagian kartu konfigurasi (Penpot F0 / S03a): judul + penjelasan di kiri, isian di kanan,
 * dipisah garis `outline-variant` antarbagian. Di lebar sempit (375) keduanya bertumpuk.
 */
export function FormSection({
  title,
  help,
  children,
  className,
}: {
  title: ReactNode;
  help?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cx(
        'grid gap-4 border-t border-outline-variant p-6 first:border-t-0 medium:grid-cols-[264px_1fr] medium:gap-8',
        className,
      )}
    >
      <div className="flex flex-col gap-1">
        <h2 className="typescale-title-small text-on-surface">{title}</h2>
        {help ? <p className="typescale-body-medium text-on-surface-variant">{help}</p> : null}
      </div>
      <div className="flex min-w-0 flex-col gap-4">{children}</div>
    </section>
  );
}
