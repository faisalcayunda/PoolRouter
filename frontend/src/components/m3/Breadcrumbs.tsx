import { Link } from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

export interface BreadcrumbItem {
  /** Label from the catalog; the usage site resolves the key. */
  label: ReactNode;
  /** Omit on the last item: the current page is not a link. */
  href?: string;
}

/**
 * Breadcrumb trail (extensions 7.1) for screens reached from a list, e.g.
 * Konfigurasi / Provider / detail. The last item is the current page (`aria-current`).
 * UI v2 (Penpot `Breadcrumbs`): body-small, pemisah teks "/", tautan `primary`, induk tanpa
 * tautan `on-surface-variant`, halaman saat ini `on-surface`.
 */
export function Breadcrumbs({
  items,
  className,
  'aria-label': ariaLabel,
}: {
  items: BreadcrumbItem[];
  className?: string;
  /** Accessible name of the trail, from the catalog. */
  'aria-label': string;
}) {
  return (
    <nav aria-label={ariaLabel} className={className}>
      <ol className="typescale-body-small flex flex-wrap items-center gap-1 text-on-surface-variant">
        {items.map((item, index) => {
          const last = index === items.length - 1;
          return (
            <li key={index} className={cx('flex items-center gap-1')}>
              {index > 0 ? <span aria-hidden="true">/</span> : null}
              {item.href && !last ? (
                <Link
                  href={item.href}
                  className="m3-focus rounded-extra-small text-primary underline-offset-2 hover:underline"
                >
                  {item.label}
                </Link>
              ) : (
                <span
                  aria-current={last ? 'page' : undefined}
                  className={last ? 'text-on-surface' : undefined}
                >
                  {item.label}
                </span>
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
