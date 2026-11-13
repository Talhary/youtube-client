import { useEffect, useLayoutEffect, useRef, useState, type MutableRefObject } from 'react';
import { call, listen, send } from '../bridge';
import { ItemView } from '../components/ItemList';
import { ErrorBox, Icon, Spinner } from '../components/ui';
import { count, meta } from '../format';
import { useNav } from '../nav';
import type { Item, VideoCard, VideoPage } from '../types';

/**
 * The watch page. The video itself is drawn by the app in a separate native view placed
 * over the black box at the top; this page only reserves that space and reports where it is.
 */
interface Props {
  video: VideoCard;
  fullscreen: boolean;
  /** The video the native player currently shows. */
  playerId: MutableRefObject<string | null>;
  autoplay: boolean;
}

export function Watch({ video, fullscreen, playerId, autoplay }: Props) {
  const nav = useNav();
  const box = useRef<HTMLDivElement>(null);
  const [details, setDetails] = useState<VideoPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [round, setRound] = useState(0);

  // Start the player for this video.
  useEffect(() => {
    if (playerId.current !== video.id) {
      playerId.current = video.id;
      send({ cmd: 'player_open', id: video.id });
    }
  }, [video.id, playerId]);

  // Keep the native player over the box.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const report = () => {
      const r = el.getBoundingClientRect();
      send({ cmd: 'player_rect', x: r.left, y: r.top, w: r.width, h: r.height });
    };
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    window.addEventListener('resize', report);
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', report);
    };
  }, [video.id]);

  useEffect(() => {
    let live = true;
    setDetails(null);
    setError(null);
    setExpanded(false);
    call<VideoPage>({ cmd: 'video', id: video.id })
      .then((d) => live && setDetails(d))
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [video.id, round]);

  const v = details?.video ?? video;
  const related: Item[] = details?.related.items ?? [];

  // When the video ends, start the first one under "Up next".
  const nextUp = related.find((i): i is { Video: VideoCard } => 'Video' in i)?.Video;
  useEffect(() => {
    if (!autoplay || !nextUp) return;
    return listen((e) => {
      if (e.ev === 'player_ended' && (e as { id?: string }).id === video.id) nav.play(nextUp);
    });
  }, [autoplay, nextUp, video.id, nav]);

  return (
    <div className="flex h-full flex-col bg-bg">
      {/* The app draws the player here. */}
      <div ref={box} className="relative w-full flex-none bg-black" style={{ aspectRatio: '16 / 9', maxHeight: '60vh' }}>
        <div className="absolute inset-0 flex items-center justify-center">
          <div className="spinner" />
        </div>
      </div>
      {!fullscreen && (
        <div className="scroll min-h-0 flex-1">
          <div className="flex items-start gap-1 px-1 pt-1">
            <button onClick={nav.back} aria-label="Close video" className="flex h-11 w-11 flex-none items-center justify-center active:opacity-60">
              <Icon name="back" />
            </button>
            <div className="min-w-0 flex-1 py-2 pr-3">
              <h1 className="text-[16px] font-semibold leading-snug">{v.title}</h1>
              <p className="mt-1 text-xs text-mute">
                {meta([
                  v.views != null && `${count(v.views)} views`,
                  v.published,
                  details?.likes != null && `${count(details.likes)} likes`,
                ])}
              </p>
            </div>
          </div>
          {v.channel_name && (
            <button
              disabled={!v.channel_id}
              onClick={() => v.channel_id && nav.push({ kind: 'channel', id: v.channel_id, name: v.channel_name ?? undefined })}
              className="flex w-full items-center gap-3 border-y border-line px-4 py-3 text-left active:bg-panel"
            >
              <span className="flex h-9 w-9 flex-none items-center justify-center rounded-full bg-panel text-mute">
                <Icon name="user" size={18} />
              </span>
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{v.channel_name}</span>
              <Icon name="chevron" size={18} className="text-mute" />
            </button>
          )}
          {details?.description && (
            <button onClick={() => setExpanded((x) => !x)} className="block w-full px-4 py-3 text-left">
              <p className={`whitespace-pre-wrap break-words text-[13px] leading-relaxed text-mute ${expanded ? '' : 'clamp-2'}`}>
                {details.description}
              </p>
              <span className="mt-1 block text-xs font-medium text-accent">{expanded ? 'Show less' : 'Show more'}</span>
            </button>
          )}
          {error && <ErrorBox text={error} onRetry={() => setRound((r) => r + 1)} />}
          {!details && !error && <Spinner />}
          {related.length > 0 && (
            <section className="pb-6 pt-1">
              <h2 className="px-4 pb-1 pt-2 text-sm font-semibold">Up next</h2>
              {related.map((i, n) => (
                <ItemView key={n} item={i} compact />
              ))}
            </section>
          )}
        </div>
      )}
    </div>
  );
}
