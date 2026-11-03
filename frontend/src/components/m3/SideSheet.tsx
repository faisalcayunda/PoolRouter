import {
  DialogTrigger as RacDialogTrigger,
  Heading,
  Modal,
  Dialog as RacDialog,
  type ModalOverlayProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

export interface SideSheetProps extends ModalOverlayProps {
  title?: ReactNode;
  /** Standard sheets dock beside content without a scrim; modal sheets cover it (§7.1). */
  variant?: 'standard' | 'modal';
  children: ReactNode;
}

/**
 * M3 side sheet. Standard variant is a non-modal docked panel; modal variant
 * slides over content with a scrim and Esc dismissal.
 */
export function SideSheet({ title, variant = 'standard', children, ...props }: SideSheetProps) {
  if (variant === 'standard') {
    return (
      <aside
        aria-label={typeof title === 'string' ? title : undefined}
        className="typescale-body-medium flex w-80 shrink-0 flex-col overflow-auto border-l border-outline-variant bg-surface-container-low text-on-surface"
      >
        {title ? (
          <Heading className="typescale-title-large px-4 py-4 text-on-surface">{title}</Heading>
        ) : null}
        {children}
      </aside>
    );
  }
  return (
    <Modal
      {...props}
      isDismissable
      className="fixed inset-0 z-50 flex justify-end bg-on-surface/32"
    >
      <RacDialog
        className={cx(
          'm3-focus typescale-body-medium flex h-full w-80 flex-col overflow-auto rounded-l-extra-large bg-surface-container-low text-on-surface shadow-level3 outline-none',
        )}
      >
        {title ? (
          <Heading className="typescale-title-large px-4 py-4 text-on-surface">{title}</Heading>
        ) : null}
        {children}
      </RacDialog>
    </Modal>
  );
}

export function SideSheetTrigger(props: React.ComponentProps<typeof RacDialogTrigger>) {
  return <RacDialogTrigger {...props} />;
}
