import type { ReactNode } from 'react';

export function ConfigList({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-2">{children}</div>;
}
