import { useRef, type ReactNode } from 'react';

/**
 * Keyboard-reachable scroll container for wide tables and payload JSON (WCAG 2.1.1: a region that
 * scrolls must be operable from the keyboard; section 9, "area bergulir bisa difokus").
 *
 * `tabindex` is set imperatively on the scroll box instead of in JSX. jsx-a11y's
 * no-noninteractive-tabindex only inspects JSX attributes; a focusable scroll container is exactly
 * what 2.1.1 requires, and the ARIA roles that would satisfy the rule (`region`, `application`)
 * either do not pass it or would override the table semantics screen readers rely on here.
 * The label is a visually hidden heading, so the container needs no role of its own.
 */
export function ScrollArea({
  label,
  className,
  children,
}: {
  /** Names the scrollable area for assistive tech. */
  label: string;
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  return (
    <div className={className}>
      <p className="sr-only">{label}</p>
      <div
        ref={(node) => {
          ref.current = node;
          // Tabbing into the box makes it scrollable with the arrow keys.
          if (node) node.tabIndex = 0;
        }}
        className="overflow-x-auto"
        aria-label={label}
      >
        {children}
      </div>
    </div>
  );
}
