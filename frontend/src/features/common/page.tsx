import type { ReactNode, Ref } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n, TFunction } from 'i18next';
import { ApiError } from '../../api/client';
import { labelOr } from '../../i18n/labelOr';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';

/**
 * Copy for a failed read, keyed by the server's stable `kode` (section 7: "Error + coba lagi").
 * A specific line wins when the catalog has one; otherwise the generic load line. A transport
 * failure always reads as "check the connection".
 */
export function errorMessage(
  t: TFunction,
  instance: i18n,
  error: unknown,
): { message: string; kode: string } {
  const kode = error instanceof ApiError ? error.kode : 'jaringan';
  // A transport failure always reads as "check the connection".
  if (kode === 'jaringan') return { message: t('error.network'), kode };
  // A 403 is the one HTTP status the catalog words per se.
  if (error instanceof ApiError && error.status === 403) {
    return { message: t('error.http.403.title'), kode };
  }
  // A catalogued `error.<kode>` line wins when one exists; otherwise the generic load line.
  const message = labelOr(instance, `error.${kode}`, t('error.network'));
  return { message, kode };
}

/**
 * Error state for a read (`InlineAlert` tone=error plus a "Coba lagi" action, section 7). The alert
 * is not assertive: it answers a user action, so an assertive live region would announce it twice.
 */
export function QueryError({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const { t, i18n: instance } = useTranslation();
  const { message } = errorMessage(t, instance, error);
  return (
    <InlineAlert
      tone="error"
      live="polite"
      action={
        <Button variant="text" onPress={onRetry}>
          {t('common.action.retry')}
        </Button>
      }
    >
      {message}
    </InlineAlert>
  );
}

/** Single `h1` per screen with its subtitle; `tabIndex=-1` so navigation can move focus here. */
export function PageHeader({
  title,
  subtitle,
  titleRef,
  children,
  actions,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  titleRef?: Ref<HTMLHeadingElement>;
  children?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-4 flex flex-col gap-2">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h1
          ref={titleRef}
          tabIndex={-1}
          className="typescale-headline-small m3-focus rounded-small text-on-surface outline-none"
        >
          {title}
        </h1>
        {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
      </div>
      {subtitle ? (
        <p className="typescale-body-medium text-on-surface-variant">{subtitle}</p>
      ) : null}
      {children}
    </div>
  );
}

/** Empty state block: a heading and one line of guidance, never a bare illustration. */
export function EmptyState({ title, body }: { title: ReactNode; body: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-1 p-6">
      <p className="typescale-title-medium text-on-surface">{title}</p>
      <p className="typescale-body-medium text-on-surface-variant">{body}</p>
    </div>
  );
}

/** Loading line for a table: a polite status plus a skeleton bar, no spinner-only screen. */
export function LoadingState({ label }: { label: string }) {
  return (
    <p
      role="status"
      aria-live="polite"
      className="typescale-body-medium p-6 text-on-surface-variant"
    >
      {label}
    </p>
  );
}
