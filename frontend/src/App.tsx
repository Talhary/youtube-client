import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { hasBridge, listen, send } from './bridge';
import { Icon } from './components/ui';
import { NavContext, type Nav, type Route } from './nav';
import { LibraryTab, SettingsTab } from './pages/Library';
import { AccountList, ChannelView, HomeTab, PlaylistView, SearchPage, SubsTab } from './pages/Lists';
import { Watch } from './pages/Watch';
import type { AuthState, Settings, Status, VideoCard } from './types';

type Tab = 'home' | 'subs' | 'library' | 'settings';

const TABS: Array<{ id: Tab; label: string; icon: 'home' | 'subs' | 'library' | 'settings' }> = [
  { id: 'home', label: 'Home', icon: 'home' },
  { id: 'subs', label: 'Subscriptions', icon: 'subs' },
  { id: 'library', label: 'Library', icon: 'library' },
  { id: 'settings', label: 'Settings', icon: 'settings' },
];

function authFrom(a: unknown): { state: AuthState; error?: string } | null {
  if (a === 'SignedOut' || a === 'Denied') return { state: 'out' };
  if (a && typeof a === 'object') {
    if ('SignedIn' in a) return { state: 'in' };
    if ('Failed' in a) return { state: 'out', error: String((a as { Failed: unknown }).Failed) };
  }
  return null;
}

function RouteView({ route }: { route: Route }) {
  switch (route.kind) {
    case 'search':
      return <SearchPage />;
    case 'channel':
      return <ChannelView id={route.id} name={route.name} />;
    case 'playlist':
      return <PlaylistView id={route.id} name={route.name} />;
    case 'history':
    case 'playlists':
    case 'channels':
      return <AccountList kind={route.kind} />;
  }
}

export default function App() {
  const [tab, setTab] = useState<Tab>('home');
  const [stack, setStack] = useState<Route[]>([]);
  const [watching, setWatching] = useState<VideoCard | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [auth, setAuth] = useState<AuthState>('unknown');
  const [signingIn, setSigningIn] = useState(false);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [adRules, setAdRules] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  /** Bumped by the refresh button to load the feeds again. */
  const [refresh, setRefresh] = useState(0);

  // What the native player is showing, so a change that came from the player itself
  // (a suggestion tapped inside it) is not sent back as a new "open".
  const playerId = useRef<string | null>(null);
  const live = useRef({ tab, stack, watching });
  live.current = { tab, stack, watching };

  const closeVideo = useCallback(() => {
    playerId.current = null;
    setWatching(null);
    setFullscreen(false);
    send({ cmd: 'player_close' });
  }, []);

  const back = useCallback(() => {
    const now = live.current;
    if (now.watching) closeVideo();
    else if (now.stack.length) setStack((s) => s.slice(0, -1));
    else if (now.tab !== 'home') setTab('home');
    else send({ cmd: 'exit' });
  }, [closeVideo]);

  const nav = useMemo<Nav>(
    () => ({
      push(route) {
        if (live.current.watching) closeVideo();
        setStack((s) => s.concat(route));
      },
      back,
      play(video) {
        setWatching(video);
      },
      signedIn: auth === 'in',
    }),
    [auth, back, closeVideo],
  );

  useEffect(() => {
    const stop = listen((e) => {
      switch (e.ev) {
        case 'ready':
          setStatus((e as { status: Status }).status);
          break;
        case 'settings':
          setSettings((e as { settings: Settings }).settings);
          break;
        case 'auth': {
          const a = authFrom((e as { auth: unknown }).auth);
          if (a) {
            setAuth(a.state);
            if (a.error) setToast(a.error);
          }
          break;
        }
        case 'signin': {
          const s = e as { state: string; text?: string };
          setSigningIn(s.state === 'open');
          if (s.state === 'failed' && s.text) setToast(s.text);
          break;
        }
        case 'adrules':
          setAdRules(String((e as { text?: unknown }).text ?? ''));
          break;
        case 'toast':
          setToast(String((e as { text?: unknown }).text ?? ''));
          break;
        case 'back':
          back();
          break;
        case 'player_fullscreen':
          setFullscreen(Boolean((e as { on?: unknown }).on));
          break;
        case 'player_video': {
          const id = String((e as { id?: unknown }).id ?? '');
          const cur = live.current.watching;
          if (id && cur && cur.id !== id) {
            playerId.current = id;
            setWatching({
              id, title: '', channel_id: null, channel_name: null, duration_s: null,
              thumbnail: null, views: null, published: null, is_live: false,
            });
          }
          break;
        }
      }
    });
    send({ cmd: 'ui_ready' });
    return stop;
  }, [back]);

  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(t);
  }, [toast]);

  const top = stack[stack.length - 1];

  return (
    <NavContext.Provider value={nav}>
      <div className="relative flex h-full flex-col bg-bg text-[15px]">
        {/* Tabs stay mounted under pushed pages so their lists keep their place. */}
        <div className={`flex min-h-0 flex-1 flex-col ${top || watching ? 'hidden' : ''}`}>
          <header className="flex h-14 flex-none items-center border-b border-line px-4">
            <span className="text-lg font-bold tracking-tight">
              Stream<span className="text-accent">Pipe</span>
            </span>
            <span className="flex-1" />
            {(tab === 'home' || tab === 'subs') && (
              <button onClick={() => setRefresh((r) => r + 1)} aria-label="Refresh" className="flex h-11 w-11 items-center justify-center active:opacity-60">
                <Icon name="refresh" />
              </button>
            )}
            <button onClick={() => nav.push({ kind: 'search' })} aria-label="Search" className="flex h-11 w-11 items-center justify-center active:opacity-60">
              <Icon name="search" />
            </button>
          </header>
          <main className="min-h-0 flex-1">
            <div className={tab === 'home' ? 'h-full' : 'hidden'}><HomeTab refresh={refresh} /></div>
            {tab === 'subs' && <SubsTab refresh={refresh} />}
            {tab === 'library' && <LibraryTab authKnown={auth !== 'unknown'} signingIn={signingIn} />}
            {tab === 'settings' && <SettingsTab settings={settings} status={status} adRules={adRules} />}
          </main>
          <nav className="flex flex-none border-t border-line bg-bg" aria-label="Sections">
            {TABS.map((t) => (
              <button
                key={t.id}
                onClick={() => setTab(t.id)}
                aria-current={tab === t.id ? 'page' : undefined}
                className={`flex flex-1 flex-col items-center gap-0.5 py-2 text-[10px] ${tab === t.id ? 'text-accent' : 'text-mute'}`}
              >
                <Icon name={t.icon} size={22} />
                {t.label}
              </button>
            ))}
          </nav>
        </div>

        {top && !watching && (
          <div className="absolute inset-0 bg-bg">
            <RouteView key={`${stack.length}:${JSON.stringify(top)}`} route={top} />
          </div>
        )}

        {watching && (
          <div className="absolute inset-0">
            <Watch video={watching} fullscreen={fullscreen} playerId={playerId} autoplay={settings?.autoplay ?? true} />
          </div>
        )}

        {!hasBridge() && (
          <p className="absolute inset-x-0 top-14 bg-red-900/80 px-4 py-2 text-center text-xs">
            Preview only: this page is not running inside the app.
          </p>
        )}

        {toast && (
          <div role="status" className="pointer-events-none absolute inset-x-4 bottom-20 flex justify-center">
            <span className="rounded-full bg-white px-4 py-2 text-center text-sm font-medium text-black shadow-lg">{toast}</span>
          </div>
        )}
      </div>
    </NavContext.Provider>
  );
}
