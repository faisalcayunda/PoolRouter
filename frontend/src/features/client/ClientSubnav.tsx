import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { cx } from '../../components/m3/cx';
import { clientPath } from './access';

const TAB =
  'typescale-title-small -mb-px inline-flex min-h-12 shrink-0 items-center border-b-2 px-4 whitespace-nowrap';

export type ClientBagian = 'detail' | 'kredensial' | 'webhook';

/**
 * Subnav satu client (S08, S08b, S09/S09a) sebagai Line tab di bawah judul (spec ARC-363 §5, frame
 * `2:22105`): ketiga bagian tampil dengan jarak tetap dan bagian aktif ditandai `aria-current`.
 * Tautan biasa supaya bisa dibuka di tab baru.
 */
export function ClientSubnav({
  clientId,
  active,
}: {
  clientId: string;
  active: ClientBagian;
}): ReactNode {
  const { t } = useTranslation();
  const items: { id: ClientBagian; href: string; label: string }[] = [
    { id: 'detail', href: clientPath(clientId), label: t('clients.tab.detail') },
    {
      id: 'kredensial',
      href: clientPath(clientId, 'kredensial'),
      label: t('clients.tab.credentials'),
    },
    { id: 'webhook', href: clientPath(clientId, 'webhook'), label: t('clients.tab.webhook') },
  ];
  return (
    <nav
      aria-label={t('clients.tab.aria')}
      className="mt-2 flex gap-4 overflow-x-auto border-b border-outline-variant"
    >
      {items.map((item) =>
        // Bagian aktif bukan tautan (tidak ada tujuan baru); tetap tampil sebagai tab terpilih.
        item.id === active ? (
          <span
            key={item.id}
            aria-current="page"
            className={cx(TAB, 'border-secondary text-primary-text')}
          >
            {item.label}
          </span>
        ) : (
          <a
            key={item.id}
            href={item.href}
            className={cx(TAB, 'm3-state m3-focus border-transparent text-on-surface-variant')}
          >
            {item.label}
          </a>
        ),
      )}
    </nav>
  );
}
