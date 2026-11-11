import type { ReactNode } from 'react';
import { Icon, type IconComponent } from '../../icons';
import { Card } from '../../components/m3/Card';
import { cx } from '../../components/m3/cx';

export interface MetricCardProps {
  /** Ikon outline di kotak kecil sebelah judul; dekoratif. */
  icon: IconComponent;
  /** Judul kecil kartu; teks dari katalog di sisi pemakai. */
  title: ReactNode;
  /** Elemen judul: `p` untuk daftar kartu, `h3` bila kartu berdiri sendiri di bawah `h1`. */
  titleAs?: 'p' | 'h3';
  /** Angka besar (32 bold). */
  value: ReactNode;
  /** Teks pelengkap di samping angka, mis. "dari 5.000". */
  valueNote?: ReactNode;
  /** Pill di samping angka (delta atau status); pemakai memberi `StatusLabel`. */
  pill?: ReactNode;
  /** Sembunyikan angka dari pembaca layar (nilai placeholder). */
  valueHidden?: boolean;
  /** Isi tambahan di bawah angka, mis. bilah progres dan keterangan. */
  children?: ReactNode;
  className?: string;
}

/** Kartu metrik (acuan ARC-261): ikon + judul kecil, angka besar, pill di sebelah angka. */
export function MetricCard({
  icon,
  title,
  titleAs: Title = 'p',
  value,
  valueNote,
  pill,
  valueHidden = false,
  children,
  className,
}: MetricCardProps) {
  return (
    <Card variant="outlined" className={cx('flex w-full flex-col gap-3 p-5!', className)}>
      {/* Ikon di dalam judul: judul tetap anak langsung kartu, dan nama aksesnya hanya teks judul. */}
      <Title className="typescale-title-medium flex items-center gap-2 text-on-surface">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-small border border-outline-variant">
          <Icon icon={icon} className="h-5 w-5" />
        </span>
        {title}
      </Title>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span
          className="typescale-headline-large text-on-surface"
          {...(valueHidden ? { 'aria-hidden': true } : {})}
        >
          {value}
        </span>
        {valueNote ? (
          <span className="typescale-body-medium text-on-surface-variant">{valueNote}</span>
        ) : null}
        {pill}
      </div>
      {children}
    </Card>
  );
}
