import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-aria-components';
import { cx } from '../../components/m3/cx';
import { clientPath } from './access';

export type ClientBagian = 'detail' | 'kredensial' | 'webhook';

/** Tiga bagian satu client (S08, S08b, S09/S09a); tautan biasa supaya bisa dibuka di tab baru. */
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
    <nav aria-label={t('clients.tab.aria')} className="flex flex-wrap gap-2">
      <Link
        href="/client"
        className="m3-state m3-focus typescale-label-large inline-flex h-10 items-center rounded-full px-3 text-primary no-underline"
      >
        {t('clients.back')}
      </Link>
      {items.map((item) => (
        <Link
          key={item.id}
          href={item.href}
          aria-current={item.id === active ? 'page' : undefined}
          className={cx(
            'm3-state m3-focus typescale-label-large inline-flex h-10 items-center rounded-full px-4 no-underline',
            item.id === active
              ? 'bg-secondary-container text-on-secondary-container'
              : 'border border-outline text-on-surface',
          )}
        >
          {item.label}
        </Link>
      ))}
    </nav>
  );
}
