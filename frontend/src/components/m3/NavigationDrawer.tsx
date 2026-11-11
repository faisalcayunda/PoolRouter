import { Dialog as RacDialog, Modal } from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

/**
 * Sidebar navigasi (acuan ARC-261) dengan dua varian:
 * - `standard`: selalu tampil di samping konten (kelas jendela large, >= 1200 px); bisa diciutkan
 *   menjadi rel ikon 72 px lewat `collapsed`.
 * - `modal`: drawer di atas scrim saat terbuka (di bawah large).
 *
 * Putih dengan border tipis di sisi kanan, lebar 240 px, tanpa bayangan. The shell owns the list of
 * links (`AppShell` renders them as children); this component owns the container and, for the modal
 * variant, the overlay, the focus trap, the Escape handling and `aria-modal` semantics via React
 * Aria's `Modal`. Nothing here closes itself: the caller owns `isOpen`.
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
  /** Varian standard saja: rel ikon 72 px alih-alih 240 px. */
  collapsed?: boolean;
  /** Overrides the container class hooks on both variants. */
  className?: string;
  children?: ReactNode;
}

/** Kolom flex putih; lebar ditentukan per varian. */
const BASE =
  'flex shrink-0 flex-col border-r border-outline-variant bg-surface-container-lowest text-on-surface';

export function NavigationDrawer({
  variant = 'standard',
  isOpen = false,
  collapsed = false,
  onOpenChange,
  className,
  children,
  'aria-label': ariaLabel,
}: NavigationDrawerProps) {
  if (variant === 'standard') {
    return (
      <aside
        aria-label={ariaLabel}
        className={cx(BASE, 'sticky top-0 h-dvh', collapsed ? 'w-[72px]' : 'w-[272px]', className)}
      >
        {children}
      </aside>
    );
  }

  return (
    <Modal
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className="fixed inset-0 z-40 flex bg-scrim/40"
    >
      <RacDialog
        aria-label={ariaLabel}
        className={cx('m3-focus relative z-10 h-full w-[272px] outline-none', BASE, className)}
      >
        {children}
      </RacDialog>
    </Modal>
  );
}
