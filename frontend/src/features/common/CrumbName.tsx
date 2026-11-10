import { createContext, useContext, useEffect, type ReactNode } from 'react';

/** Nama yang dikenal sebuah screen untuk record yang sedang dibuka; `id` adalah id dari URL. */
export interface CrumbRecord {
  id: string;
  name: string;
}

const CrumbNameContext = createContext<(record: CrumbRecord | null) => void>(() => undefined);

/** Dipasang shell: screen melaporkan nama record lewat `useCrumbName`, shell menaruhnya di breadcrumb. */
export function CrumbNameProvider({
  onChange,
  children,
}: {
  onChange: (record: CrumbRecord | null) => void;
  children: ReactNode;
}): ReactNode {
  return <CrumbNameContext.Provider value={onChange}>{children}</CrumbNameContext.Provider>;
}

/**
 * Melaporkan nama record ke breadcrumb shell. Shell hanya memakainya bila `id` sama dengan id
 * di URL, jadi nama record lama tidak bocor ke record berikutnya; tanpa nama, crumb tetap id.
 */
export function useCrumbName(id: string | undefined, name: string | undefined): void {
  const report = useContext(CrumbNameContext);
  useEffect(() => {
    if (!id || !name) return;
    report({ id, name });
    return () => report(null);
  }, [report, id, name]);
}
