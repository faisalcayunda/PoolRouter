import type { ReactNode } from 'react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '../common/useQuery';
import { useRole } from '../common/RoleContext';
import { PageHeader, QueryError, LoadingState, EmptyState } from '../common/page';
import { Button } from '../../components/m3/Button';
import { adminApi } from '../../api/client';

export function ProviderListScreen(): ReactNode {
  const { t } = useTranslation();
  const role = useRole();
  const titleRef = useMemo(() => ({ current: null as HTMLHeadingElement | null }), []);

  const { data: providers, isLoading, error, refetch } = useQuery(
    ['providers'],
    () => adminApi.provider.daftar(),
    { enabled: role === 'platform_admin' },
  );

  if (!role || role !== 'platform_admin') {
    return null; // AC-8: 403 already handled by router
  }

  if (error) {
    return <QueryError error={error} onRetry={refetch} />;
  }

  if (isLoading) {
    return <LoadingState label={t('common.state.loading')} />;
  }

  const list = providers || [];

  return (
    <>
      <PageHeader
        ref={titleRef}
        title={t('provider.title')}
        actions={list.length > 0 && <Button variant="filled">{t('provider.action.add')}</Button>}
      />
      {list.length === 0 ? (
        <EmptyState
          title={t('provider.empty.title')}
          body={t('provider.empty.body')}
        >
          <div className="mt-3 flex gap-2">
            <Button variant="filled">{t('provider.action.add')}</Button>
            <Button variant="outlined">{t('provider.action.view_secrets')}</Button>
          </div>
        </EmptyState>
      ) : (
        <div>{/* S03–S04 table akan di-render oleh K3 */}</div>
      )}
    </>
  );
}
