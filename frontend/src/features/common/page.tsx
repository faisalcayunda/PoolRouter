import type { ReactNode, Ref } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n, TFunction } from 'i18next';
import { ApiError } from '../../api/client';
import { labelOr } from '../../i18n/labelOr';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { LinearProgress } from '../../components/m3/LinearProgress';
import { Skeleton } from '../../components/m3/Skeleton';
import { cx } from '../../components/m3/cx';
import { Icon, icons } from '../../icons';

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
 * Error state for a read (Penpot S16 frame 3): an `InlineAlert` with a title and one line of cause,
 * then a filled "Coba lagi" action below it, aligned to the end. The alert is not assertive: it
 * answers a user action, so an assertive live region would announce it twice.
 */
export function QueryError({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const { t, i18n: instance } = useTranslation();
  const { message } = errorMessage(t, instance, error);
  // The generic load line is split into the frame's title and cause; a specific line stays whole.
  const body = message === t('error.network') ? t('error.load_hint') : message;
  return (
    <div className="flex flex-col gap-4">
      <InlineAlert tone="error" live="polite" heading={t('error.load_title')}>
        {body}
      </InlineAlert>
      <Button variant="filled" onPress={onRetry} className="self-end">
        {t('common.action.retry')}
      </Button>
    </div>
  );
}

/**
 * Single `h1` per screen with its subtitle; `tabIndex=-1` so navigation can move focus here.
 * Tanpa `m3-focus`: fokus ke judul hanya programatik (tidak ada di urutan Tab), jadi bingkai fokus
 * hanya muncul sebagai kotak aneh setelah klik atau navigasi, dan frame Penpot tidak menggambarnya.
 */
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
          className="typescale-headline-small text-on-surface outline-none"
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

/**
 * Empty state (Penpot S16 frames 2 and 5): a 40 px round icon box, a medium-weight title, one line
 * of cause and an optional action, all centred. `titleAs="h1"` makes it the screen heading (the 403
 * and not-found pages), focusable through `titleRef`. `tone="error"` marks a denied state.
 */
export function EmptyState({
  title,
  body,
  icon = 'search',
  tone = 'neutral',
  action,
  titleAs = 'p',
  titleRef,
}: {
  title: ReactNode;
  body: ReactNode;
  icon?: keyof typeof icons;
  tone?: 'neutral' | 'error';
  action?: ReactNode;
  titleAs?: 'p' | 'h1';
  titleRef?: Ref<HTMLHeadingElement>;
}) {
  const titleClass = 'typescale-body-large font-medium text-on-surface';
  return (
    <div className="flex flex-col items-center gap-4 px-0 py-6 text-center">
      <div className="flex flex-col items-center gap-2">
        <span
          className={cx(
            'flex h-10 w-10 items-center justify-center rounded-full',
            tone === 'error'
              ? 'bg-surface text-error'
              : 'bg-surface-container-low text-on-surface-variant',
          )}
        >
          <Icon icon={icons[icon]} className="h-6 w-6" />
        </span>
        {titleAs === 'h1' ? (
          <h1
            ref={titleRef}
            tabIndex={-1}
            className={cx(titleClass, 'm3-focus rounded-small outline-none')}
          >
            {title}
          </h1>
        ) : (
          <p className={titleClass}>{title}</p>
        )}
        <p className="typescale-body-medium max-w-[36ch] text-on-surface-variant">{body}</p>
      </div>
      {action}
    </div>
  );
}

/**
 * Loading state (Penpot S16 frame 1): four skeleton lines, a progress bar and a polite status
 * line, never a spinner-only screen. The skeleton and the bar are hidden from assistive tech; the
 * status line carries the meaning.
 */
export function LoadingState({ label }: { label: string }) {
  return (
    <div role="status" aria-live="polite" className="flex flex-col gap-3.5 p-6">
      <div aria-hidden="true" className="flex flex-col gap-3">
        <Skeleton className="h-3.5 w-[70%]" />
        <Skeleton className="h-3.5 w-full" />
        <Skeleton className="h-3.5 w-full" />
        <Skeleton className="h-3.5 w-[83%]" />
        <LinearProgress isIndeterminate aria-label={label} />
      </div>
      <p className="typescale-body-medium text-on-surface-variant">{label}</p>
    </div>
  );
}
