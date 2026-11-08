import type { ReactNode } from 'react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '../common/useQuery';
import { useRole } from '../common/RoleContext';
import { PageHeader, QueryError, LoadingState, EmptyState } from '../common/page';
import { Button } from '../../components/m3/Button';
import { adminApi } from '../../api/client';

export function SecretListScreen(): ReactNode {
  const { t } = useTranslation();
  const role = useRole();
  const titleRef = useMemo(() => ({ current: null as HTMLHeadingElement | null }), []);

  const { data: secrets, isLoading, error, refetch } = useQuery(
    ['secrets'],
    () => adminApi.secret.daftar(),
    { enabled: role === 'platform_admin' },
  );

  if (!role || role !== 'platform_admin') {
    return null; // AC-8: 403 already handled
  }

  if (error) {
    return <QueryError error={error} onRetry={refetch} />;
  }

  if (isLoading) {
    return <LoadingState label={t('common.state.loading')} />;
  }

  const list = secrets || [];

  return (
    <>
      <PageHeader
        ref={titleRef}
        title={t('secret.title')}
        actions={<Button variant="filled">{t('secret.action.add')}</Button>}
      />
      {list.length === 0 ? (
        <EmptyState
          title={t('secret.empty.title')}
          body={t('secret.empty.body')}
        />
      ) : (
        <div>{/* S05 table: nama, tersamar, versi, diubah, aksi */}</div>
      )}
    </>
  );
}
