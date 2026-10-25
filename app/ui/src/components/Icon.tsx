// The few icons the shell uses, drawn as strokes in the current colour.

const paths = {
  home: 'M4 11l8-7 8 7v9h-5v-6H9v6H4z',
  settings: 'M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-3.5-3.5',
  close: 'M6 6l12 12M18 6L6 18',
  plus: 'M12 5v14M5 12h14',
  pencil: 'M4 20h4L19 9l-4-4L4 16v4zM13 7l4 4',
  file: 'M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5',
  send: 'M12 19V5M5 12l7-7 7 7',
  stop: 'M7 7h10v10H7z',
  chevron: 'M6 9l6 6 6-6',
  check: 'M5 12l5 5 9-10',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2',
  play: 'M8 5l11 7-11 7z',
  back: 'M15 6l-6 6 6 6',
} as const;

export type IconName = keyof typeof paths;

export function Icon(props: { name: IconName; size?: number }) {
  const size = () => props.size ?? 15;
  return (
    <svg
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.9"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={paths[props.name]} />
    </svg>
  );
}
