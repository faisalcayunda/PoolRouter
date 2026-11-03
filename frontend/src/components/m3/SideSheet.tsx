import {
  DialogTrigger as RacDialogTrigger,
  Heading,
  Modal,
  Dialog as RacDialog,
  type ModalOverlayProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { IconButton } from './IconButton';
import { Icon, icons } from '../../icons';

export interface SideSheetProps extends ModalOverlayProps {
  title: ReactNode;
  /** Standard sheets dock beside content without a scrim; modal sheets cover it (§7.1). */
  variant?: 'standard' | 'modal';
  /** Accessible name of the modal close button; catalog text, never empty (§7.5). */
  closeLabel?: string;
  children: ReactNode;
}

/**
 * M3 side sheet. The standard variant is a non-modal docked panel; the modal
 * variant slides over content with a scrim, a focus trap and Esc dismissal.
 * Open state is the caller's (`SideSheetTrigger`, or `isOpen` + `onOpenChange`).
 */
export function SideSheet({
  title,
  variant = 'standard',
  closeLabel,
  children,
  ...props
}: SideSheetProps) {
  if (variant === 'standard') {
    return (
      <aside
        aria-label={typeof title === 'string' ? title : undefined}
        className="typescale-body-medium flex w-80 shrink-0 flex-col overflow-auto border-l border-outline-variant bg-surface-container-low text-on-surface"
      >
        <Heading className="typescale-title-large px-4 py-4 text-on-surface">{title}</Heading>
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
      <RacDialog className="m3-focus typescale-body-medium flex h-full w-80 flex-col overflow-auto rounded-l-extra-large bg-surface-container-low text-on-surface shadow-level3 outline-none">
        {({ close }) => (
          <>
            <div className="flex items-start justify-between gap-2">
              {/* slot="title" is what gives the modal dialog its accessible name. */}
              <Heading slot="title" className="typescale-title-large px-4 py-4 text-on-surface">
                {title}
              </Heading>
              {closeLabel ? (
                <IconButton aria-label={closeLabel} onPress={close} className="mt-3 mr-2">
                  <Icon icon={icons.close} />
                </IconButton>
              ) : null}
            </div>
            {children}
          </>
        )}
      </RacDialog>
    </Modal>
  );
}

export function SideSheetTrigger(props: React.ComponentProps<typeof RacDialogTrigger>) {
  return <RacDialogTrigger {...props} />;
}
