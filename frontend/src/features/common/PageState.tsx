import type { ReactNode } from 'react';

export function PageState({
  loading,
  error,
  empty,
  children,
}: {
  loading?: boolean;
  error?: ReactNode;
  empty?: ReactNode;
  children: ReactNode;
}) {
  if (loading)
    return (
      <div role="status" aria-live="polite">
        Memuat…
      </div>
    );
  if (error) return <>{error}</>;
  if (empty) return <>{empty}</>;
  return <>{children}</>;
}
