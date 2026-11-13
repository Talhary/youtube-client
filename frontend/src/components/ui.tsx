import type { ReactNode } from 'react';

type IconName =
  | 'home'
  | 'subs'
  | 'library'
  | 'settings'
  | 'search'
  | 'back'
  | 'close'
  | 'history'
  | 'playlist'
  | 'user'
  | 'chevron'
  | 'refresh';

const PATHS: Record<IconName, string> = {
  home: 'M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z',
  subs: 'M4 6h16M6 3h12M3 9h18v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1zm7 3.5v5l4.5-2.5z',
  library: 'M4 4h4v16H4zm6 0h4v16h-4zm6.5 1 3.8 1 -3.6 14-3.8-1z',
  settings:
    'M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7zm8.5 3.5-.1-1.3 2-1.5-2-3.4-2.3.9a8 8 0 0 0-2.2-1.3L15.500 3h-4l-.4 2.400a8 8 0 0 0-2.200 1.300l-2.300-.900-2 3.400 2 1.500a8 8 0 0 0 0 2.600l-2 1.500 2 3.400 2.300-.900a8 8 0 0 0 2.200 1.300l.4 2.400h4l.4-2.400a8 8 0 0 0 2.200-1.300l2.300.9 2-3.400-2-1.500z',
  search: 'M10.500 4a6.500 6.500 0 1 0 4.100 11.600L19.500 20.500 21 19l-4.900-4.900A6.500 6.500 0 0 0 10.500 4zm0 2a4.500 4.500 0 1 1 0 9 4.500 4.500 0 0 1 0-9z',
  back: 'M15.500 4 7.500 12l8 8 1.500-1.500L10.500 12 17 5.500z',
  close: 'M6.400 5 5 6.400 10.600 12 5 17.600 6.400 19 12 13.400 17.600 19 19 17.600 13.400 12 19 6.400 17.600 5 12 10.600z',
  history: 'M12 4a8 8 0 1 1-7.700 10h2.100A6 6 0 1 0 7 7.500L9.500 10H3V3.500l2.600 2.600A8 8 0 0 1 12 4zm-1 4h2v4.200l3 1.800-1 1.700-4-2.400z',
  playlist: 'M3 6h12v2H3zm0 5h12v2H3zm0 5h8v2H3zm14-3.500v7l5-3.500z',
  user: 'M12 4a4 4 0 1 1 0 8 4 4 0 0 1 0-8zm0 10c4.400 0 8 2 8 4.500V20H4v-1.500C4 16 7.600 14 12 14z',
  chevron: 'M8.500 4 7 5.500 13.500 12 7 18.500 8.500 20l8-8z',
  refresh: 'M12 4a8 8 0 0 1 6.900 4H16v2h6V4h-2v2.300A10 10 0 1 0 22 12h-2a8 8 0 1 1-8-8z',
};

export function Icon({ name, size = 22, className }: { name: IconName; size?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      className={className}
      fill={name === 'subs' ? 'none' : 'currentColor'}
      stroke={name === 'subs' ? 'currentColor' : 'none'}
      strokeWidth={name === 'subs' ? 1.8 : 0}
      strokeLinejoin="round"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}

export function Spinner({ className = '' }: { className?: string }) {
  return (
    <div className={`flex justify-center py-6 ${className}`} role="status" aria-label="Loading">
      <div className="spinner" />
    </div>
  );
}

export function ErrorBox({ text, onRetry }: { text: string; onRetry?: () => void }) {
  return (
    <div className="mx-4 my-6 rounded-2xl border border-line bg-panel p-4 text-center">
      <p className="text-sm text-mute">{text}</p>
      {onRetry && (
        <button
          onClick={onRetry}
          className="mt-3 rounded-full bg-accent px-5 py-2 text-sm font-semibold text-black active:opacity-80"
        >
          Try again
        </button>
      )}
    </div>
  );
}

export function Empty({ text }: { text: string }) {
  return <p className="px-6 py-10 text-center text-sm text-mute">{text}</p>;
}

/** Page header with a back button. */
export function TopBar({ title, onBack, right }: { title: string; onBack: () => void; right?: ReactNode }) {
  return (
    <header className="flex h-14 flex-none items-center gap-1 border-b border-line bg-bg px-1">
      <button onClick={onBack} aria-label="Back" className="flex h-12 w-12 items-center justify-center active:opacity-60">
        <Icon name="back" />
      </button>
      <h1 className="min-w-0 flex-1 truncate text-base font-semibold">{title}</h1>
      {right}
    </header>
  );
}
