// A schedule and its times in words, as the task pages say them.

import type { Schedule } from '../../protocol/Schedule';

const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const plural = ['Mondays', 'Tuesdays', 'Wednesdays', 'Thursdays', 'Fridays', 'Saturdays', 'Sundays'];

export { days };

/** `Weekdays at 08:00`, `Every 2 hours`, `Only when you run it`. */
export function scheduleWords(schedule: Schedule) {
  if (schedule.kind === 'manual') return 'Only when you run it';
  if (schedule.kind === 'every') return schedule.hours === 1 ? 'Every hour' : `Every ${schedule.hours} hours`;
  const picked = [...schedule.days].sort((a, b) => a - b);
  const key = picked.join('');
  const which =
    key === '0123456'
      ? 'Every day'
      : key === '01234'
        ? 'Weekdays'
        : key === '56'
          ? 'Weekends'
          : picked.length === 0
            ? 'No days picked'
            : picked.length === 1
              ? plural[picked[0]]
              : `${picked
                  .slice(0, -1)
                  .map((d) => days[d])
                  .join(', ')} and ${days[picked[picked.length - 1]]}`;
  return `${which} at ${schedule.time}`;
}

const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
const time = (date: Date) => date.toTimeString().slice(0, 5);

/** `Today`, `Yesterday`, or `Fri 23 Oct`. */
export function dayWords(ms: number) {
  const date = new Date(ms);
  const today = new Date();
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (sameDay(date, today)) return 'Today';
  if (sameDay(date, yesterday)) return 'Yesterday';
  return date.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
}

/** `08:00`. */
export const timeWords = (ms: number) => time(new Date(ms));

/** `today 08:00`, `tomorrow 07:30`, `Fri 08:00`. */
export function whenWords(ms: number) {
  const date = new Date(ms);
  const today = new Date();
  const tomorrow = new Date(today.getTime() + 86_400_000);
  if (sameDay(date, today)) return `today ${time(date)}`;
  if (sameDay(date, tomorrow)) return `tomorrow ${time(date)}`;
  return `${date.toLocaleDateString(undefined, { weekday: 'short' })} ${time(date)}`;
}
