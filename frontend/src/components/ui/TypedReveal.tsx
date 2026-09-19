import { useEffect, useState, type ComponentProps } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useTypedReveal } from "../../hooks/useTypedReveal";

type MarkdownComponents = ComponentProps<typeof ReactMarkdown>["components"];

interface TypedRevealBaseProps {
  text: string;
  active?: boolean;
  delay?: number;
  charsPerSecond?: number;
  minStep?: number;
  maxStep?: number;
  snapBacklog?: number;
  reducedMotion?: boolean;
  stabilizeMarkdown?: boolean;
}

export interface TypedRevealTextProps extends TypedRevealBaseProps {}

export interface TypedRevealMarkdownProps extends TypedRevealBaseProps {
  components?: MarkdownComponents;
}

function useRevealStart(active: boolean, delay: number, text: string): boolean {
  const [started, setStarted] = useState(!active || delay <= 0);

  useEffect(() => {
    if (!active) {
      setStarted(true);
      return;
    }
    if (delay <= 0 || typeof window === "undefined") {
      setStarted(true);
      return;
    }
    setStarted(false);
    const timer = window.setTimeout(() => setStarted(true), delay);
    return () => window.clearTimeout(timer);
  }, [active, delay, text]);

  return started;
}

function RevealedText({
  text,
  charsPerSecond,
  minStep,
  maxStep,
  snapBacklog,
  reducedMotion,
  stabilizeMarkdown,
}: TypedRevealTextProps) {
  const revealed = useTypedReveal(text, {
    isStreaming: true,
    charsPerSecond,
    minStep,
    maxStep,
    snapBacklog,
    reducedMotion,
    stabilizeMarkdown,
  });
  return <>{revealed}</>;
}

function RevealedMarkdown({
  text,
  charsPerSecond,
  minStep,
  maxStep,
  snapBacklog,
  reducedMotion,
  stabilizeMarkdown,
  components,
}: TypedRevealMarkdownProps) {
  const revealed = useTypedReveal(text, {
    isStreaming: true,
    charsPerSecond,
    minStep,
    maxStep,
    snapBacklog,
    reducedMotion,
    stabilizeMarkdown,
  });

  return (
    <ReactMarkdown skipHtml remarkPlugins={[remarkGfm]} components={components}>
      {revealed}
    </ReactMarkdown>
  );
}

export function TypedRevealText({
  text,
  active = false,
  delay = 0,
  charsPerSecond = 720,
  minStep,
  maxStep = 48,
  snapBacklog = 1200,
  reducedMotion,
  stabilizeMarkdown = false,
}: TypedRevealTextProps) {
  const started = useRevealStart(active, delay, text);
  if (!active) return <>{text}</>;
  if (!started) return null;
  return (
    <RevealedText
      text={text}
      charsPerSecond={charsPerSecond}
      minStep={minStep}
      maxStep={maxStep}
      snapBacklog={snapBacklog}
      reducedMotion={reducedMotion}
      stabilizeMarkdown={stabilizeMarkdown}
    />
  );
}

export function TypedRevealMarkdown({
  text,
  active = false,
  delay = 0,
  charsPerSecond = 760,
  minStep,
  maxStep = 56,
  snapBacklog = 1600,
  reducedMotion,
  stabilizeMarkdown = true,
  components,
}: TypedRevealMarkdownProps) {
  const started = useRevealStart(active, delay, text);
  if (!active) {
    return (
      <ReactMarkdown skipHtml remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    );
  }
  if (!started) return null;
  return (
    <RevealedMarkdown
      text={text}
      charsPerSecond={charsPerSecond}
      minStep={minStep}
      maxStep={maxStep}
      snapBacklog={snapBacklog}
      reducedMotion={reducedMotion}
      stabilizeMarkdown={stabilizeMarkdown}
      components={components}
    />
  );
}
