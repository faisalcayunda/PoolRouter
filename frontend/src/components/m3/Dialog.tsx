import {
  DialogTrigger as RacDialogTrigger,
  Heading,
  Modal,
  Dialog as RacDialog,
  type ModalOverlayProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { IconButton } from './IconButton';
import { icons } from '../../icons';

export interface DialogProps extends ModalOverlayProps {
  /** Dialog title; the caller supplies catalog text (Addendum A 4). */
  title: ReactNode;
  /** Optional visually hidden description for aria-describedby. */
  description?: ReactNode;
  variant?: 'basic' | 'full-screen';
  onCloseLabel?: string;
  onDismiss?: () => void;
  footer?: ReactNode;
  children: ReactNode;
}

/**
 * M3 dialog (basic and full-screen) with scrim, focus trap and Esc dismissal.
 * Full-screen covers the viewport below the expanded window size class (§7.1).
 */
export function Dialog({
  title,
  description,
  variant = 'basic',
  onCloseLabel,
  onDismiss,
  footer,
  children,
  ...props
}: DialogProps) {
  return (
    <Modal
      {...props}
      isDismissable
      className={cx(
        'fixed inset-0 z-50 flex bg-on-surface/32 data-[entering]:animate-none data-[exiting]:animate-none',
        variant === 'basic' ? 'items-center justify-center p-4' : 'items-stretch justify-stretch',
      )}
    >
      <RacDialog
        className={cx(
          'm3-focus flex flex-col bg-surface-container-high text-on-surface shadow-level3 outline-none',
          variant === 'basic'
            ? 'w-full max-w-[560px] rounded-extra-large p-6'
            : 'h-full w-full rounded-none',
        )}
      >
        {({ close }: { close: () => void }) => (
          <>
            <div className="flex items-start justify-between gap-2">
              <Heading slot="title" className="typescale-headline-small text-on-surface">
                {title}
              </Heading>
              {onDismiss ? (
                <IconButton
                  aria-label={onCloseLabel ?? ''}
                  icon-position="end"
                  onPress={() => {
                    onDismiss();
                    close();
                  }}
                >
                  <IconClose />
                </IconButton>
              ) : null}
            </div>
            {description ? <p className="typescale-body-medium text-on-surface-variant">{description}</p> : null}
            <div className="typescale-body-medium min-h-0 flex-1 overflow-auto text-on-surface-variant">
              {children}
            </div>
            {footer ? <div className="mt-4 flex justify-end gap-2">{footer}</div> : null}
          </>
        )}
      </RacDialog>
    </Modal>
  );
}

function IconClose() {
  return <Icon icon={icons.close} />;
}

import { Icon } from '../../icons';

/** Convenience wrapper so dialogs can be opened by a trigger button. */
export function DialogTrigger(props: React.ComponentProps<typeof RacDialogTrigger>) {
  return <RacDialogTrigger {...props} />;
}
