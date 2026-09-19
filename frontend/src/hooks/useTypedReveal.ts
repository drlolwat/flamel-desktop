import { useEffect, useMemo, useRef, useState } from "react";
import {
  clampRevealLength,
  nextTypedRevealLength,
  normalizeTypedRevealOptions,
  stabilizePartialMarkdown,
  type TypedRevealOptions,
} from "../lib/typedReveal";

interface UseTypedRevealOptions extends TypedRevealOptions {
  isStreaming?: boolean;
  reducedMotion?: boolean;
  stabilizeMarkdown?: boolean;
}

function detectReducedMotion(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function usePrefersReducedMotion(): boolean {
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(detectReducedMotion);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setPrefersReducedMotion(query.matches);
    update();
    query.addEventListener?.("change", update);
    return () => query.removeEventListener?.("change", update);
  }, []);

  return prefersReducedMotion;
}

export function useTypedReveal(text: string, options: UseTypedRevealOptions = {}): string {
  const normalized = useMemo(() => normalizeTypedRevealOptions(options), [
    options.charsPerSecond,
    options.minStep,
    options.maxStep,
    options.snapBacklog,
  ]);
  const prefersReducedMotion = usePrefersReducedMotion();
  const shouldReduceMotion = options.reducedMotion ?? prefersReducedMotion;
  const isStreaming = !!options.isStreaming && !shouldReduceMotion;
  const displayLenRef = useRef(isStreaming ? 0 : text.length);
  const targetLenRef = useRef(text.length);
  const lastFrameRef = useRef<number | null>(null);
  const rafRef = useRef<number | null>(null);
  const [displayLen, setDisplayLen] = useState(() => (isStreaming ? 0 : text.length));

  useEffect(() => {
    targetLenRef.current = text.length;

    if (!isStreaming) {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      lastFrameRef.current = null;
      displayLenRef.current = text.length;
      setDisplayLen(text.length);
      return;
    }

    const clamped = clampRevealLength(displayLenRef.current, text.length);
    if (clamped !== displayLenRef.current) {
      displayLenRef.current = clamped;
      setDisplayLen(clamped);
    }

    if (text.length - displayLenRef.current >= normalized.snapBacklog) {
      displayLenRef.current = text.length;
      setDisplayLen(text.length);
      return;
    }

    if (rafRef.current !== null || displayLenRef.current >= text.length) return;

    const tick = (now: number) => {
      const lastFrame = lastFrameRef.current ?? now;
      lastFrameRef.current = now;
      const nextLength = nextTypedRevealLength({
        ...normalized,
        currentLength: displayLenRef.current,
        targetLength: targetLenRef.current,
        deltaMs: now - lastFrame,
      });
      displayLenRef.current = nextLength;
      setDisplayLen(nextLength);
      if (nextLength >= targetLenRef.current) {
        rafRef.current = null;
        lastFrameRef.current = null;
        return;
      }
      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
  }, [isStreaming, normalized, text]);

  useEffect(() => () => {
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
  }, []);

  const shown = displayLen >= text.length ? text : text.slice(0, displayLen);
  if (!options.stabilizeMarkdown || shown.length >= text.length) return shown;
  return stabilizePartialMarkdown(shown);
}
