import { useCallback, useEffect, useState } from 'react';
import { ApiError } from '../../api/client';

/** State machine the handoff spec asks of every screen (section 7: loading, empty, error, success). */
export type QueryState<T> =
  | { status: 'loading'; data: null; error: null }
  | { status: 'success'; data: T; error: null }
  | { status: 'error'; data: null; error: ApiError | Error };

/** `QueryState` plus a stable retry handle; kept as an intersection so narrowing on `status` works. */
export type Query<T> = QueryState<T> & {
  /** Stable identity so screens can bind "Coba lagi" without re-running on every render. */
  reload: () => void;
};

/**
 * Runs `load` whenever `key` changes and keeps the four states of section 7 apart. An aborted
 * request (a newer key arrived, or the component unmounted) never writes state: that is what keeps
 * a late response from showing a previous NIK or payload after the user moved on (section 9).
 *
 * `load` and the returned state are updated from the effect only, never during render, so the
 * component never reads a ref mid-render and the initial state is `loading` by construction.
 */
export function useQuery<T>(
  load: (signal: AbortSignal) => Promise<T>,
  key: string,
  options: { enabled?: boolean } = {},
): Query<T> {
  const enabled = options.enabled ?? true;
  const [state, setState] = useState<QueryState<T>>({
    status: 'loading',
    data: null,
    error: null,
  });
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    // Deferred so a key change paints the loading state without a synchronous cascade in the effect.
    queueMicrotask(() => {
      if (!controller.signal.aborted) {
        setState({ status: 'loading', data: null, error: null });
      }
    });
    // `Promise.resolve` keeps a synchronous stub or a non-promise return from crashing the effect.
    Promise.resolve(load(controller.signal)).then(
      (data) => {
        if (!controller.signal.aborted) setState({ status: 'success', data, error: null });
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        setState({
          status: 'error',
          data: null,
          error: error instanceof Error ? error : new Error(String(error)),
        });
      },
    );
    return () => controller.abort();
    // `load` is intentionally not a dependency: the caller closes over the request it means, and
    // `key` is the value that identifies that request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled, nonce]);

  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return { ...state, reload };
}

/** Map an unknown failure to the problem code the copy layer keys off. */
export function problemCode(error: unknown): string {
  return error instanceof ApiError ? error.kode : 'jaringan';
}
