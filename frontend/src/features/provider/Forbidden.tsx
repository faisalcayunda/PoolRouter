import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { useFocusOnMount } from '../common/a11y';
import { PageHeader } from '../common/page';

/**
 * State 403 untuk layar konfigurasi (S16): peran aktif tidak mencakup layar ini. Judul memakai
 * kalimat katalog yang sama dengan shell, jadi tidak ada salinan teks 403 kedua.
 */
export function Forbidden(): ReactNode {
  const { t } = useTranslation();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  return (
    <section className="flex flex-col gap-2">
      <PageHeader
        title={t('error.http.403.title')}
        subtitle={t('error.http.403.body')}
        titleRef={titleRef}
      />
    </section>
  );
}
