import { useCallback, useLayoutEffect, useRef, type RefObject } from 'react';
import { isDesktop } from './api';
import { resizeContentWindow } from './panel';

interface ContentWindowHeightOptions {
  shell: RefObject<HTMLElement | null>;
  content: RefObject<HTMLElement | null>;
  viewport?: RefObject<HTMLElement | null>;
  enabled?: boolean;
  revision?: number;
  onError: (error: unknown) => void;
}

export function useContentWindowHeight({ shell, content, viewport = shell, enabled = true, revision, onError }: ContentWindowHeightOptions) {
  const lastRequest = useRef<{ height: number; revision?: number; pending: Promise<void> } | null>(null);

  const syncHeight = useCallback((): Promise<void> => {
    if (!isDesktop || !enabled || !shell.current || !viewport.current || !content.current) return Promise.resolve();
    // Measure the unconstrained child, not scrollHeight (which cannot shrink
    // below the old viewport). Add the header, footer and shell padding.
    const height = Math.ceil(content.current.getBoundingClientRect().height
      + shell.current.getBoundingClientRect().height - viewport.current.getBoundingClientRect().height);
    if (!Number.isFinite(height) || height <= 0) return Promise.resolve();
    if (lastRequest.current?.height === height && lastRequest.current.revision === revision) return lastRequest.current.pending;
    const pending = resizeContentWindow(height, revision);
    const request = { height, revision, pending };
    lastRequest.current = request;
    void pending.catch(() => { if (lastRequest.current === request) lastRequest.current = null; });
    return pending;
  }, [shell, content, viewport, enabled, revision]);

  useLayoutEffect(() => {
    if (!isDesktop || !enabled) return;
    let frame = 0;
    let disposed = false;
    const sync = () => {
      void syncHeight().catch((error: unknown) => { if (!disposed) onError(error); });
    };
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(sync);
    };
    const observer = new ResizeObserver(schedule);
    for (const element of new Set([shell.current, viewport.current, content.current])) {
      if (element) observer.observe(element);
    }
    // A newly opened native window may be on a screen with a different work area.
    const resync = () => { lastRequest.current = null; schedule(); };
    window.addEventListener('focus', resync);
    sync();
    return () => {
      disposed = true;
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener('focus', resync);
    };
  }, [shell, content, viewport, enabled, syncHeight, onError]);

  return syncHeight;
}
