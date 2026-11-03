import { useEffect, useId, useRef } from 'react';

/**
 * Sets `document.title` per screen through the catalog (section 13.2). Never pass a NIK or any
 * other personal data here: the title lands in browser history and window lists (section 9).
 */
export function useDocumentTitle(title: string): void {
  useEffect(() => {
    document.title = String(title);
  }, [title]);
}

/**
 * Focus the `h1` after a route change so the screen reader announces the new page and Tab starts
 * from the top. The heading is not in the tab order, hence `tabIndex={-1}` + programmatic focus
 * (section 8, "fokus ke h1 setelah navigasi").
 */
export function useFocusOnMount<T extends HTMLElement>(): React.RefObject<T | null> {
  const ref = useRef<T>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return ref;
}

/** Stable heading ids for `aria-labelledby` / `aria-describedby` pairs. */
export function useHeadingIds() {
  const base = useId();
  return {
    titleId: `${base}-title`,
    bodyId: `${base}-body`,
  };
}

/**
 * Scrollable region needs a keyboard entry point (section 9: "area bergulir bisa difokus").
 * `tabIndex=0` only when the content can actually overflow — the hook returns the props as-is;
 * components decide via CSS `overflow` and always set `tabIndex=0`, which WCAG 2.1.1 accepts.
 */
export const SCROLL_REGION_PROPS = {
  tabIndex: 0,
  role: 'group',
} as const;
