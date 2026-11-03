import {
  UNSTABLE_Toast as Toast,
  UNSTABLE_ToastContent as ToastContent,
  UNSTABLE_ToastList as ToastList,
  UNSTABLE_ToastRegion as ToastRegionRoot,
  UNSTABLE_ToastQueue,
  type QueuedToast,
} from 'react-aria-components';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

/**
 * Snackbar region (ADR-0003 7.1): renders the RAC toast queue at the bottom center.
 * Callers create the queue with `useToastQueue(new ToastQueue(...))` and pass it here;
 * the close label is a catalog string (Addendum A 5.4).
 */
export function Snackbar<T>({
  queue,
  className,
  renderContent,
  closeLabel,
}: {
  queue: InstanceType<typeof UNSTABLE_ToastQueue<T>>;
  className?: string;
  renderContent: (content: T) => React.ReactNode;
  closeLabel: string;
}) {
  return (
    <ToastRegionRoot
      queue={queue}
      className={cx(
        // Full-width strip for centring, but click-through: only the toasts take pointer events,
        // otherwise the empty strip silently swallows clicks on whatever sits behind it.
        'pointer-events-none fixed inset-x-0 bottom-4 z-50 flex flex-col items-center gap-2 outline-none',
        className,
      )}
    >
      <ToastList>
        {({ toast }: { toast: QueuedToast<unknown> }) => (
          <Toast
            toast={toast as QueuedToast<T>}
            className="pointer-events-auto m3-focus typescale-body-medium flex min-h-12 w-full max-w-sm items-center gap-2 rounded-extra-small bg-inverse-surface py-2 pl-4 pr-1 text-inverse-on-surface shadow-level3 outline-none"
          >
            <ToastContent className="flex-1">{renderContent(toast.content as T)}</ToastContent>
            <button
              type="button"
              aria-label={closeLabel}
              onClick={() => queue.close(toast.key)}
              className="m3-state m3-focus flex h-10 w-10 shrink-0 cursor-pointer items-center justify-center rounded-full text-inverse-on-surface"
            >
              <Icon icon={icons.close} />
            </button>
          </Toast>
        )}
      </ToastList>
    </ToastRegionRoot>
  );
}
