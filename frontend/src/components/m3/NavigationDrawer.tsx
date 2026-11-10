import { Dialog as RacDialog, Modal } from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

/**
 * M3 navigation drawer (katalog 7.1) with the two variants the catalog asks for:
 * - `standard`: permanently visible beside the content (expanded window class).
 * - `modal`: overlaid with a scrim while open (compact/medium window class).
 *
 * Both variants use the inverse surface roles (UI v2): the drawer stays dark in the light theme.
 * Lebar 280 px, tanpa radius dan tanpa bayangan di kedua varian (Penpot `NavigationDrawer /
 * standard|modal`); varian modal berdiri di atas lapisan `scrim` opasitas 32 %.
 * The shell owns the list of links (`AppShell` renders them as children); this
 * component owns the container and, for the modal variant, the overlay, the
 * focus trap, the Escape handling and `aria-modal` semantics via React Aria's
 * `Modal`. Nothing here closes itself: the caller owns `isOpen`.
 */
export type NavigationDrawerVariant = 'standard' | 'modal';

export interface NavigationDrawerProps {
  variant?: NavigationDrawerVariant;
  /** Accessible name of the drawer (catalog: `nav.drawer.aria`). */
  'aria-label': string;
  /** Modal variant only: whether the drawer is showing. */
  isOpen?: boolean;
  /** Modal variant only: called on Escape, scrim click and focus-leave dismissal. */
  onOpenChange?: (isOpen: boolean) => void;
  /** Overrides the container class hooks on both variants. */
  className?: string;
  children?: ReactNode;
}

/** Lebar 280 px untuk kedua varian (Penpot `NavigationDrawer`), kolom flex. */
const BASE = 'flex w-[280px] shrink-0 flex-col bg-inverse-surface text-inverse-on-surface';

export function NavigationDrawer({
  variant = 'standard',
  isOpen = false,
  onOpenChange,
  className,
  children,
  'aria-label': ariaLabel,
}: NavigationDrawerProps) {
  if (variant === 'standard') {
    return (
      <aside aria-label={ariaLabel} className={cx(BASE, 'sticky top-0 h-dvh', className)}>
        {children}
      </aside>
    );
  }

  return (
    <Modal
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className="fixed inset-0 z-40 flex bg-scrim/32"
    >
      <RacDialog
        aria-label={ariaLabel}
        className={cx('m3-focus relative z-10 h-full outline-none', BASE, className)}
      >
        {children}
      </RacDialog>
    </Modal>
  );
}
