import type { ReactNode } from 'react';

export function ConfigForm({ children }: { children: ReactNode }) {
  return <form className="flex flex-col gap-4">{children}</form>;
}
