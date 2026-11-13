import { useEffect, useRef } from 'react';
import { count, duration, img, meta } from '../format';
import { useNav } from '../nav';
import type { ChannelCard, Item, PlaylistCard, VideoCard } from '../types';
import type { PageState } from '../usePage';
import { Empty, ErrorBox, Icon, Spinner } from './ui';

function Thumb({ src, badge, live }: { src?: string; badge?: string; live?: boolean }) {
  return (
    <div className="relative aspect-video w-full overflow-hidden bg-panel">
      {src && <img src={src} alt="" loading="lazy" decoding="async" className="h-full w-full object-cover" />}
      {live ? (
        <span className="absolute bottom-2 right-2 rounded bg-red-600 px-1.5 py-0.5 text-[11px] font-semibold">LIVE</span>
      ) : (
        badge && (
          <span className="absolute bottom-2 right-2 rounded bg-black/80 px-1.5 py-0.5 text-[11px] font-medium">{badge}</span>
        )
      )}
    </div>
  );
}

export function VideoRow({ v }: { v: VideoCard }) {
  const nav = useNav();
  return (
    <article className="pb-4">
      <button onClick={() => nav.play(v)} className="block w-full text-left active:opacity-80" aria-label={v.title}>
        <Thumb src={img(v.thumbnail)} badge={duration(v.duration_s)} live={v.is_live} />
        <div className="px-3 pt-2.5">
          <h3 className="clamp-2 text-[15px] font-medium leading-snug">{v.title}</h3>
        </div>
      </button>
      <button
        className="block w-full px-3 pt-1 text-left text-xs text-mute active:opacity-70"
        disabled={!v.channel_id}
        onClick={() => v.channel_id && nav.push({ kind: 'channel', id: v.channel_id, name: v.channel_name ?? undefined })}
      >
        {meta([v.channel_name, v.views != null && `${count(v.views)} views`, v.published])}
      </button>
    </article>
  );
}

/** Small side-by-side row, used under the player. */
export function VideoCompact({ v }: { v: VideoCard }) {
  const nav = useNav();
  return (
    <button onClick={() => nav.play(v)} className="flex w-full gap-3 px-3 py-2 text-left active:opacity-80">
      <div className="relative w-40 flex-none overflow-hidden rounded-lg bg-panel" style={{ aspectRatio: '16 / 9' }}>
        {v.thumbnail && <img src={img(v.thumbnail)} alt="" loading="lazy" className="h-full w-full object-cover" />}
        {v.duration_s != null && !v.is_live && (
          <span className="absolute bottom-1 right-1 rounded bg-black/80 px-1 text-[10px]">{duration(v.duration_s)}</span>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <h3 className="clamp-2 text-sm font-medium leading-snug">{v.title}</h3>
        <p className="mt-1 truncate text-xs text-mute">{v.channel_name}</p>
        <p className="truncate text-xs text-mute">{meta([v.views != null && `${count(v.views)} views`, v.published])}</p>
      </div>
    </button>
  );
}

export function ChannelRow({ c }: { c: ChannelCard }) {
  const nav = useNav();
  return (
    <button
      onClick={() => nav.push({ kind: 'channel', id: c.id, name: c.name })}
      className="flex w-full items-center gap-3 px-4 py-2.5 text-left active:bg-panel"
    >
      <div className="h-12 w-12 flex-none overflow-hidden rounded-full bg-panel">
        {c.avatar && <img src={img(c.avatar)} alt="" loading="lazy" className="h-full w-full object-cover" />}
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-[15px] font-medium">{c.name}</p>
        {c.subscribers != null && <p className="text-xs text-mute">{count(c.subscribers)} subscribers</p>}
      </div>
      <Icon name="chevron" size={18} className="text-mute" />
    </button>
  );
}

export function PlaylistRow({ p }: { p: PlaylistCard }) {
  const nav = useNav();
  return (
    <button
      onClick={() => nav.push({ kind: 'playlist', id: p.id, name: p.name })}
      className="flex w-full gap-3 px-3 py-2 text-left active:opacity-80"
    >
      <div className="relative w-40 flex-none overflow-hidden rounded-lg bg-panel" style={{ aspectRatio: '16 / 9' }}>
        {p.thumbnail && <img src={img(p.thumbnail)} alt="" loading="lazy" className="h-full w-full object-cover" />}
        <span className="absolute inset-y-0 right-0 flex w-14 flex-col items-center justify-center bg-black/75 text-[11px]">
          <Icon name="playlist" size={18} />
          {p.video_count != null && <span className="mt-0.5">{p.video_count}</span>}
        </span>
      </div>
      <div className="min-w-0 flex-1">
        <h3 className="clamp-2 text-sm font-medium leading-snug">{p.name}</h3>
        <p className="mt-1 truncate text-xs text-mute">{meta([p.channel_name, 'Playlist'])}</p>
      </div>
    </button>
  );
}

export function ItemView({ item, compact }: { item: Item; compact?: boolean }) {
  if ('Video' in item) return compact ? <VideoCompact v={item.Video} /> : <VideoRow v={item.Video} />;
  if ('Channel' in item) return <ChannelRow c={item.Channel} />;
  return <PlaylistRow p={item.Playlist} />;
}

const keyOf = (i: Item): string =>
  'Video' in i ? `v${i.Video.id}` : 'Channel' in i ? `c${i.Channel.id}` : `p${i.Playlist.id}`;

function Skeletons() {
  return (
    <div aria-hidden="true">
      {[0, 1, 2].map((i) => (
        <div key={i} className="pb-4">
          <div className="skeleton aspect-video w-full" />
          <div className="skeleton mx-3 mt-3 h-4 w-4/5 rounded" />
          <div className="skeleton mx-3 mt-2 h-3 w-2/5 rounded" />
        </div>
      ))}
    </div>
  );
}

/** A list with loading, error and empty states that fetches more when its end comes near. */
export function ItemList({ page, compact, empty }: { page: PageState; compact?: boolean; empty: string }) {
  const sentinel = useRef<HTMLDivElement>(null);
  const loadMore = page.loadMore;
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !page.hasMore) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) loadMore();
      },
      { rootMargin: '900px' },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [page.hasMore, loadMore, page.items.length]);

  if (page.loading) return compact ? <Spinner /> : <Skeletons />;
  if (page.error) return <ErrorBox text={page.error} onRetry={page.reload} />;
  if (page.items.length === 0) return <Empty text={empty} />;
  return (
    <div>
      {page.items.map((i) => (
        <ItemView key={keyOf(i)} item={i} compact={compact} />
      ))}
      <div ref={sentinel} />
      {page.loadingMore && <Spinner />}
    </div>
  );
}
