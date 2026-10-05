export const MIN_LEFT = 180;
export const MIN_RIGHT = 240;

export function clampWidth(width: number, total: number): number {
  return Math.min(Math.max(width, MIN_LEFT), Math.max(MIN_LEFT, total - MIN_RIGHT));
}
