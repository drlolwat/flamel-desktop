import { useEffect, useLayoutEffect, useRef, useCallback } from 'react';

const NEAR_BOTTOM_THRESHOLD = 120; // px from bottom to count as "away from bottom"
const BOTTOM_EPSILON = 4;

export function useAutoScroll(deps: unknown[], isStreaming = false) {
  const ref = useRef<HTMLDivElement>(null);
  const anchorRef = useRef<HTMLDivElement>(null);
  // Starts true so an empty chat auto-scrolls on first load
  const isAtBottomRef = useRef(true);
  const userDetachedRef = useRef(false);
  const lastScrollTopRef = useRef(0);
  const rafRef = useRef<number>(0);

  const scrollToAnchor = useCallback(() => {
    anchorRef.current?.scrollIntoView({ block: 'end' });
  }, []);

  // A newly opened or restored tab starts attached to its latest durable event.
  // Run once before passive scroll tracking can interpret scrollTop=0 as intent.
  useLayoutEffect(() => {
    userDetachedRef.current = false;
    isAtBottomRef.current = true;
    scrollToAnchor();
    const frame = requestAnimationFrame(scrollToAnchor);
    return () => cancelAnimationFrame(frame);
  }, [scrollToAnchor]);

  // Track scroll position — update isAtBottomRef whenever the user scrolls
  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const releaseAutoScroll = () => {
      userDetachedRef.current = true;
      isAtBottomRef.current = false;
    };

    const handleScroll = () => {
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      const scrollingUp = el.scrollTop < lastScrollTopRef.current - 1;
      lastScrollTopRef.current = el.scrollTop;

      if (dist <= BOTTOM_EPSILON) {
        userDetachedRef.current = false;
        isAtBottomRef.current = true;
        return;
      }

      if (scrollingUp || userDetachedRef.current) {
        releaseAutoScroll();
        return;
      }

      if (dist >= NEAR_BOTTOM_THRESHOLD) isAtBottomRef.current = false;
    };

    // Wheel fires before scrollTop updates; release immediately on upward intent.
    const handleWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) releaseAutoScroll();
    };

    // Touch: track start position so we can detect upward swipes
    let touchStartY = 0;
    const handleTouchStart = (e: TouchEvent) => { touchStartY = e.touches[0].clientY; };
    const handleTouchMove = (e: TouchEvent) => {
      if (e.touches[0].clientY > touchStartY) releaseAutoScroll();
    };

    lastScrollTopRef.current = el.scrollTop;
    handleScroll();

    el.addEventListener('scroll', handleScroll, { passive: true });
    el.addEventListener('wheel', handleWheel, { passive: true });
    el.addEventListener('touchstart', handleTouchStart, { passive: true });
    el.addEventListener('touchmove', handleTouchMove, { passive: true });
    return () => {
      el.removeEventListener('scroll', handleScroll);
      el.removeEventListener('wheel', handleWheel);
      el.removeEventListener('touchstart', handleTouchStart);
      el.removeEventListener('touchmove', handleTouchMove);
    };
  }, []);

  // Follow late layout changes (tool expansion, images, fonts) while attached to
  // the bottom. ResizeObserver is event-driven and does not fight manual scroll.
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;

    const observer = new ResizeObserver(() => {
      if (!isAtBottomRef.current || userDetachedRef.current) return;
      cancelAnimationFrame(rafRef.current);
      rafRef.current = requestAnimationFrame(scrollToAnchor);
    });
    observer.observe(el);
    for (const child of Array.from(el.children)) observer.observe(child);

    return () => {
      observer.disconnect();
      cancelAnimationFrame(rafRef.current);
    };
  }, [scrollToAnchor]);

  // During streaming: rAF loop — only scrolls if user is already near bottom
  useEffect(() => {
    if (!isStreaming) return;

    function tick() {
      if (isAtBottomRef.current) scrollToAnchor();
      rafRef.current = requestAnimationFrame(tick);
    }
    rafRef.current = requestAnimationFrame(tick);

    return () => cancelAnimationFrame(rafRef.current);
  }, [isStreaming, scrollToAnchor]);

  // When streaming ends, do a delayed scroll to catch typed reveal's final snap
  useEffect(() => {
    if (isStreaming) return;
    const t = setTimeout(() => {
      if (isAtBottomRef.current) scrollToAnchor();
    }, 80);
    return () => clearTimeout(t);
  }, [isStreaming, scrollToAnchor]);

  // When not streaming: scroll on dep changes (new blocks, final content)
  useEffect(() => {
    if (isAtBottomRef.current) scrollToAnchor();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  const scrollToBottom = useCallback(() => {
    userDetachedRef.current = false;
    isAtBottomRef.current = true;
    scrollToAnchor();
  }, [scrollToAnchor]);

  return { ref, anchorRef, scrollToBottom };
}
