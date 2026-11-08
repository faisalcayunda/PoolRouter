import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { UNSTABLE_ToastQueue } from 'react-aria-components';
import { Snackbar } from '../../components/m3/Snackbar';

/**
 * One toast queue per screen (section 7: success confirmations are announced with a Snackbar,
 * 400 px, bottom centre). RAC requires a single toast region, so the screen owns the queue and
 * renders `<SnackbarRegion queue={queue} />` once; anything that succeeds pushes a catalog key.
 */
export function useSnackbar(): {
  queue: UNSTABLE_ToastQueue<{ message: string }>;
  notify: (message: string) => void;
} {
  const [queue] = useState(
    () => new UNSTABLE_ToastQueue<{ message: string }>({ maxVisibleToasts: 3 }),
  );
  return {
    queue,
    notify: (message: string) => queue.add({ message }),
  };
}

export function SnackbarRegion({
  queue,
}: {
  queue: UNSTABLE_ToastQueue<{ message: string }>;
}): ReactNode {
  const { t } = useTranslation();
  return (
    <Snackbar
      queue={queue}
      closeLabel={t('common.close')}
      renderContent={(content: { message: string }) => <span>{content.message}</span>}
    />
  );
}
