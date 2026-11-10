import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { LinkButton } from '../provider/LinkButton';
import { clientPath } from './access';

export type ClientBagian = 'detail' | 'kredensial' | 'webhook';

/**
 * Tautan ke dua bagian lain satu client (S08, S08b, S09/S09a). Frame Penpot tidak menggambar tab:
 * jejak breadcrumb sudah membawa kembali ke daftar, jadi bagian lain cukup tautan teks di sisi
 * aksi judul. Tautan biasa supaya bisa dibuka di tab baru.
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
    <nav aria-label={t('clients.tab.aria')} className="flex flex-wrap items-center">
      {items
        .filter((item) => item.id !== active)
        .map((item) => (
          <LinkButton key={item.id} href={item.href} variant="text">
            {item.label}
          </LinkButton>
        ))}
    </nav>
  );
}
