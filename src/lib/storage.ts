// localStorage can throw (blocked storage, quota) and returns untrusted text.
// Every read goes through a validator and falls back to the default.

const PREFIX = 'pcremote.';

export function readStored<T>(key: string, fallback: T, validate: (value: unknown) => value is T): T {
  try {
    const raw = window.localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    const value: unknown = JSON.parse(raw);
    return validate(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

export function writeStored(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // Preferences are a convenience; the app keeps working without them.
  }
}

export const isString = (value: unknown): value is string => typeof value === 'string';
export const isBoolean = (value: unknown): value is boolean => typeof value === 'boolean';
export const oneOf =
  <T extends string | number>(options: readonly T[]) =>
  (value: unknown): value is T =>
    options.includes(value as T);
