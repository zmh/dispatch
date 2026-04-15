import { useEffect, useState } from "react";

export function useLoadingTimer(active: boolean, slowAfterMs: number = 20_000) {
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState<number>(() => Date.now());

  useEffect(() => {
    if (active) {
      const now = Date.now();
      setStartedAt(now);
      setNowMs(now);
    } else {
      setStartedAt(null);
      setNowMs(Date.now());
    }
  }, [active]);

  useEffect(() => {
    if (!active || startedAt === null) return;
    const timer = setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => clearInterval(timer);
  }, [active, startedAt]);

  const elapsedMs = active && startedAt !== null ? Math.max(0, nowMs - startedAt) : 0;
  const elapsedSeconds = Math.floor(elapsedMs / 1000);
  const isSlow = active && elapsedMs >= slowAfterMs;

  return {
    elapsedMs,
    elapsedSeconds,
    isSlow,
  };
}
