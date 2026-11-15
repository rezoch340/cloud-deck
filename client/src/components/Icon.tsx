const paths: Record<string, string> = {
  deck: 'm3 7 9-5 9 5-9 5-9-5Zm0 5 9 5 9-5M3 17l9 5 9-5',
  home: 'M3 4h18v13H3zM8 21h8m-4-4v4M7 8h10',
  apps: 'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 17.5h7m-3.5-3.5v7',
  folder: 'M3 6V4h6l3 3h9v14H3zM3 11h18',
  activity: 'M4 3h16v18H4zM8 8h8m-8 4h8m-8 4h5',
  settings: 'M4 7h16M4 17h16M7 4v6m10 4v6',
  cloud: 'M7 18a5 5 0 0 1-1-9.9A6.5 6.5 0 0 1 18.4 9.5 4.3 4.3 0 0 1 18 18Z',
  node: 'M2.5 7h6v10h-6zM15.5 2.5h6v7h-6zM15.5 14.5h6v7h-6zM8.5 12H12V6h3.5M12 12v6h3.5',
  code: 'm8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18',
  service: 'M3 3h18v7H3zM3 14h18v7H3zM7 6.5h.01M7 17.5h.01M12 6.5h5M12 17.5h5',
  check: 'm5 12 4 4L19 6',
  close: 'm6 6 12 12M6 18 18 6',
  plus: 'M12 5v14M5 12h14',
  arrow: 'm7 17 10-10M7 7h10v10',
  chevron: 'm9 5 7 7-7 7',
  back: 'm10 5-7 7 7 7M3 12h18',
  shield: 'm12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6Zm-4 9 3 3 5-6',
  alert: 'm12 3 10 18H2ZM12 9v5m0 3h.01',
  info: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18ZM12 11v6m0-10h.01',
  refresh: 'M20 7v5h-5M4 17v-5h5M20 12A8 8 0 0 0 6 7M4 12a8 8 0 0 0 14 5',
  user: 'M12 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM4 21v-2a8 8 0 0 1 16 0v2',
  link: 'm10 14 4-4M9 7l2-2a5 5 0 0 1 7 7l-2 2M15 17l-2 2a5 5 0 0 1-7-7l2-2',
  browser: 'M2 3h20v17H2zM2 8h20M5 5.5h.01m3 0h.01M9 13l3 3 4-4',
  play: 'm8 4 12 8-12 8Z',
  file: 'M14 3H5v18h14V8ZM14 3v5h5M8 13h8m-8 4h6',
  chip: 'M5 5h14v14H5zM9 9h6v6H9zM9 2v3m6-3v3M9 19v3m6-3v3M2 9h3m-3 6h3m14-6h3m-3 6h3',
};
export function Icon({ name, className = '' }: { name: string; className?: string }) {
  return (
    <svg className={`cd-icon ${className}`} viewBox="0 0 24 24" aria-hidden="true">
      <path d={paths[name] || paths.apps} />
    </svg>
  );
}
export function GoogleMark() {
  return (
    <svg className="cd-google" viewBox="0 0 24 24" aria-hidden="true">
      <path
        fill="#4285f4"
        d="M21.6 12.23c0-.71-.06-1.39-.18-2.05H12v3.88h5.38a4.61 4.61 0 0 1-1.99 3.03v2.52h3.23c1.89-1.74 2.98-4.3 2.98-7.38Z"
      />
      <path
        fill="#34a853"
        d="M12 22c2.7 0 4.96-.9 6.62-2.39l-3.23-2.52c-.9.6-2.05.96-3.39.96-2.61 0-4.82-1.76-5.61-4.12H3.05v2.6A10 10 0 0 0 12 22Z"
      />
      <path
        fill="#fbbc05"
        d="M6.39 13.93A6 6 0 0 1 6.08 12c0-.67.11-1.32.31-1.93v-2.6H3.05A10 10 0 0 0 2 12c0 1.61.39 3.14 1.05 4.53l3.34-2.6Z"
      />
      <path
        fill="#ea4335"
        d="M12 5.95c1.47 0 2.79.5 3.82 1.49l2.87-2.87A9.61 9.61 0 0 0 12 2a10 10 0 0 0-8.95 5.47l3.34 2.6C7.18 7.71 9.39 5.95 12 5.95Z"
      />
    </svg>
  );
}
