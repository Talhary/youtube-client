import { useCallback, useEffect, useRef, useState } from 'react';
import { call, send } from './bridge';
import type { Item, Page } from './types';

export interface PageState {
  items: Item[];
  loading: boolean;
  loadingMore: boolean;
  error: string | null;
  hasMore: boolean;
  loadMore(): void;
  reload(): void;
}

const keyOf = (i: Item): string =>
  'Video' in i ? `v${i.Video.id}` : 'Channel' in i ? `c${i.Channel.id}` : `p${i.Playlist.id}`;

/**
 * A list that loads its first page with `load` and further pages through the core's
 * continuation handles. `key` identifies the list; changing it starts over.
 */
export function usePage(key: string, load: () => Promise<Page>): PageState {
  const [items, setItems] = useState<Item[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [next, setNext] = useState<number | null>(null);
  const [round, setRound] = useState(0);
  const loadRef = useRef(load);
  loadRef.current = load;
  // Guards against answers that arrive after the list changed.
  const gen = useRef(0);
  const nextRef = useRef<number | null>(null);
  nextRef.current = next;

  useEffect(() => {
    const mine = ++gen.current;
    setItems([]);
    setNext(null);
    setError(null);
    setLoading(true);
    loadRef
      .current()
      .then((p) => {
        if (gen.current !== mine) return;
        setItems(p.items);
        setNext(p.next);
      })
      .catch((e: Error) => {
        if (gen.current === mine) setError(e.message);
      })
      .finally(() => {
        if (gen.current === mine) setLoading(false);
      });
    return () => {
      // Tell the core it can forget the unused continuation.
      if (nextRef.current != null) send({ cmd: 'drop_cursor', cursor: nextRef.current });
    };
  }, [key, round]);

  const loadMore = useCallback(() => {
    if (next == null || loadingMore) return;
    const mine = gen.current;
    setLoadingMore(true);
    setNext(null);
    call<Page>({ cmd: 'more', cursor: next })
      .then((p) => {
        if (gen.current !== mine) return;
        setItems((old) => {
          const seen = new Set(old.map(keyOf));
          return old.concat(p.items.filter((i) => !seen.has(keyOf(i))));
        });
        setNext(p.next);
      })
      .catch(() => {
        // The handle is single-use; a failed page simply ends the list.
      })
      .finally(() => {
        if (gen.current === mine) setLoadingMore(false);
      });
  }, [next, loadingMore]);

  const reload = useCallback(() => setRound((r) => r + 1), []);

  return { items, loading, loadingMore, error, hasMore: next != null, loadMore, reload };
}
