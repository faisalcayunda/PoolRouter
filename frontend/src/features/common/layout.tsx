import type { ReactNode } from 'react';
import { Card } from '../../components/m3/Card';
import { cx } from '../../components/m3/cx';

/**
 * Tata letak bersama layar ui-v3 (spec ARC-363 §6). Shell sudah memberi padding L1 dan lebar
 * maksimal 1280; komponen di sini menyusun isi di dalamnya.
 */

/**
 * L4: grid field 3/2/1 kolom (≥1200 / 600–1199 / <600), jarak baris 36 dan kolom 28 di ≥1200,
 * 24/20 di bawahnya. Tiap anak langsung adalah satu field dengan label di atas kontrolnya; field
 * selebar grid memakai `col-span-full`.
 */
export function FieldGrid({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      data-layout="field-grid"
      className={cx(
        'grid grid-cols-1 gap-x-5 gap-y-6 medium:grid-cols-2 large:grid-cols-3 large:gap-x-7 large:gap-y-9',
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * L6: kelas untuk `Select` di grid atau filter. Trigger selebar kolomnya dan nilai terpilih
 * dipotong dengan elipsis supaya tetap satu baris 40 px.
 */
export const SELECT_FILL = '[&>button]:w-full [&>button>span]:min-w-0 [&>button>span]:truncate';

/**
 * L3: kartu bagian (frame `2:22105`). Radius 16, padding 32 di ≥1200 dan 24 di bawahnya; judul
 * `title-large` warna `primary-text` dengan aksi di kanan.
 */
export function SectionCard({
  title,
  actions,
  children,
  className,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Card variant="container" className={cx('flex flex-col gap-6 p-6 large:p-8', className)}>
      {title || actions ? (
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          {title ? <h2 className="typescale-title-large text-primary-text">{title}</h2> : null}
          {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
        </div>
      ) : null}
      {children}
    </Card>
  );
}

/** L5: baris filter di dalam kartu, rata bawah supaya tombol sejajar dengan kotak isian. */
export function FilterBar({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cx(
        'flex flex-col gap-4 rounded-medium bg-surface-container-low p-4 expanded:flex-row expanded:flex-wrap expanded:items-end',
        className,
      )}
    >
      {children}
    </div>
  );
}
