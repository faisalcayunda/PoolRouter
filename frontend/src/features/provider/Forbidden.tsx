import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Card } from '../../components/m3/Card';
import { useFocusOnMount } from '../common/a11y';
import { EmptyState } from '../common/page';
import { LinkButton } from './LinkButton';

/**
 * State 403 untuk layar konfigurasi (S16): peran aktif tidak mencakup layar ini. Teks memakai
 * kalimat katalog yang sama dengan shell, jadi tidak ada salinan teks 403 kedua. Bentuknya mengikuti
 * frame Penpot S16 kartu 5 di dalam kartu (spec ARC-363 §5): ikon kunci, judul, sebab, dan aksi "Ke halaman awal".
 */
export function Forbidden(): ReactNode {
  const { t } = useTranslation();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  return (
    <Card variant="container" className="mx-auto w-full max-w-[640px] p-6 large:p-8">
      <EmptyState
        icon="lock"
        tone="error"
        titleAs="h1"
        titleRef={titleRef}
        title={t('error.http.403.title')}
        body={t('error.http.403.body')}
        action={
          <LinkButton href="/" variant="outlined">
            {t('error.http.403.action')}
          </LinkButton>
        }
      />
    </Card>
  );
}
