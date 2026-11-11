import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type Provider } from '../../api/client';
import { Card } from '../../components/m3/Card';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { LinkButton } from './LinkButton';
import { Forbidden } from './Forbidden';

export interface ProviderListScreenProps {
  client?: ApiClient;
}

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

/**
 * S02 daftar provider (ARC-101). Hanya platform admin; peran lain yang sudah diketahui melihat
 * state 403, dan API tetap penjaga terakhir untuk peran yang belum diketahui.
 */
export function ProviderListScreen({ client = api }: ProviderListScreenProps): ReactNode {
  const { t } = useTranslation();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.provider'));
  const allowed = role === null || role === 'platform_admin';

  const providers = useQuery<Provider[]>(
    (signal) => client.listProviders({ signal }),
    'providers',
    {
      enabled: allowed,
    },
  );

  if (!allowed) return <Forbidden />;

  const rows = providers.status === 'success' ? providers.data : [];

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={t('provider.page.title')}
        subtitle={t('provider.page.subtitle')}
        titleRef={titleRef}
        actions={
          providers.status === 'success' ? (
            <LinkButton href="/provider/baru">{t('provider.page.add')}</LinkButton>
          ) : null
        }
      />

      {providers.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {providers.status === 'error' ? (
        <QueryError error={providers.error} onRetry={providers.reload} />
      ) : null}

      {providers.status === 'success' && rows.length === 0 ? (
        <Card variant="container" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large text-on-surface">{t('provider.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {t('provider.empty.body')}
          </p>
          <div className="flex flex-wrap justify-center gap-2">
            <LinkButton href="/provider/baru">{t('provider.page.add')}</LinkButton>
            <LinkButton href="/secret" variant="text">
              {t('provider.empty.goto_secrets')}
            </LinkButton>
          </div>
        </Card>
      ) : null}

      {rows.length > 0 ? (
        <Card variant="container" className="overflow-hidden">
          <ScrollArea label={t('provider.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('provider.page.title')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('provider.list.col.name')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('provider.list.col.version')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('provider.list.col.method')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('provider.list.col.status')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('provider.list.col.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr
                      key={row.provider_id}
                      className="border-b border-outline-variant text-on-surface"
                    >
                      <td data-label={t('provider.list.col.name')} className={TD}>
                        {row.nama}
                      </td>
                      <td data-label={t('provider.list.col.version')} className={TD}>
                        {row.provider_config_version}
                      </td>
                      <td data-label={t('provider.list.col.method')} className={TD}>
                        <span className="font-code">{row.metode}</span>
                      </td>
                      <td data-label={t('provider.list.col.status')} className={TD}>
                        <StatusLabel tone={row.aktif ? 'success' : 'neutral'}>
                          {row.aktif ? t('common.state.active') : t('provider.list.inactive')}
                        </StatusLabel>
                      </td>
                      <td data-label={t('provider.list.col.actions')} className={TD}>
                        <LinkButton
                          href={`/provider/${encodeURIComponent(row.provider_id)}`}
                          variant="text"
                          aria-label={t('provider.list.edit_aria', { nama: row.nama })}
                        >
                          {t('common.action.edit')}
                        </LinkButton>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        </Card>
      ) : null}
    </section>
  );
}
