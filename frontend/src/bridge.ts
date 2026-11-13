// Messages between this page and the app.
//
// Out: `SP.send(json)`. Commands the native shell handles itself (player, sign-in, exit)
// and commands for the core share one channel; the shell tells them apart by name.
// In: the shell calls `window.__spEvent(event)`.

import type { AppEvent } from './types';

declare global {
  interface Window {
    SP?: { send(json: string): void };
    __spEvent?: (e: AppEvent) => void;
  }
}

type Listener = (e: AppEvent) => void;

let nextReq = 1;
const pending = new Map<number, (e: AppEvent) => void>();
const listeners = new Set<Listener>();

export const hasBridge = () => typeof window.SP?.send === 'function';

export function send(cmd: Record<string, unknown>): void {
  try {
    window.SP?.send(JSON.stringify(cmd));
  } catch (e) {
    console.error('bridge send failed', e);
  }
}

/** Send a core request and wait for its answer. Rejects with the core's error text. */
export function call<T>(cmd: Record<string, unknown>): Promise<T> {
  const req = nextReq++;
  return new Promise<T>((resolve, reject) => {
    if (!hasBridge()) {
      reject(new Error('Not running inside the app'));
      return;
    }
    const timer = window.setTimeout(() => {
      pending.delete(req);
      reject(new Error('No answer from the app. Check your connection and try again.'));
    }, 60_000);
    pending.set(req, (e) => {
      window.clearTimeout(timer);
      const any = e as unknown as { err?: string; ok?: T };
      if (typeof any.err === 'string') reject(new Error(any.err));
      else resolve((any.ok ?? (e as unknown)) as T);
    });
    send({ ...cmd, req });
  });
}

export function listen(l: Listener): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

window.__spEvent = (e: AppEvent) => {
  const req = (e as { req?: number }).req;
  if (typeof req === 'number') {
    const done = pending.get(req);
    if (done) {
      pending.delete(req);
      done(e);
    }
  }
  listeners.forEach((l) => {
    try {
      l(e);
    } catch (err) {
      console.error(err);
    }
  });
};
