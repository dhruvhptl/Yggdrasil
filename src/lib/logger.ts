export const log = (...args: unknown[]) => {
  if (import.meta.env.DEV) console.log(...args);
};
export const warn = (...args: unknown[]) => {
  if (import.meta.env.DEV) console.warn(...args);
};
export const logError = (...args: unknown[]) => console.error(...args);
