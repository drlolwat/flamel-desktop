/**
 * Minimal SVG icon set for flamel.
 * All icons are 16×16 viewBox, stroke-based unless noted.
 * Usage: <Icon.Send size={14} />
 */
import type { JSX } from 'react';

type IconProps = { size?: number; className?: string; style?: React.CSSProperties };

function mk(path: JSX.Element, vb = '0 0 16 16') {
  return function Icon({ size = 16, className, style }: IconProps) {
    return (
      <svg
        width={size}
        height={size}
        viewBox={vb}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        className={className}
        style={style}
        aria-hidden
      >
        {path}
      </svg>
    );
  };
}

function mkFill(path: JSX.Element, vb = '0 0 16 16') {
  return function Icon({ size = 16, className, style }: IconProps) {
    return (
      <svg
        width={size}
        height={size}
        viewBox={vb}
        fill="currentColor"
        stroke="none"
        className={className}
        style={style}
        aria-hidden
      >
        {path}
      </svg>
    );
  };
}

export const Icon = {
  /** Flamel alchemist mark — fire triangle + terminal cursor */
  Logo: function Logo({ size = 16, className, style, ...rest }: IconProps & Record<string, any>) {
    const w = size * 0.75; // 60:80 aspect ratio
    return (
      <svg
        width={w}
        height={size}
        viewBox="0 0 60 80"
        fill="none"
        className={className}
        style={style}
        aria-hidden
        {...rest}
      >
        <polygon points="30,6 58,74 2,74" stroke="currentColor" strokeWidth="3.5" strokeLinejoin="round" />
        <rect x="27" y="0" width="6" height="80" rx="1.5" fill="currentColor" />
      </svg>
    );
  },

  /** Send / submit */
  Send: mk(<><path d="M13.5 8H2.5" /><path d="M9 3.5L13.5 8l-4.5 4.5" /></>),

  /** Stop square */
  Stop: mkFill(<rect x="4" y="4" width="8" height="8" rx="1" />),

  /** Attach / paperclip */
  Attach: mk(
    <path d="M12 7.5l-5.5 5.5a3.18 3.18 0 01-4.5-4.5l6-6a2.12 2.12 0 013 3L5.5 11a1.06 1.06 0 01-1.5-1.5L9.5 4" />,
  ),

  /** Plus */
  Plus: mk(<><path d="M8 3v10" /><path d="M3 8h10" /></>),

  /** Close / X */
  Close: mk(<><path d="M4 4l8 8" /><path d="M12 4l-8 8" /></>),

  /** Chevron down */
  ChevronDown: mk(<path d="M4 6l4 4 4-4" />),

  /** Chevron right */
  ChevronRight: mk(<path d="M6 4l4 4-4 4" />),

  /** Branch / git */
  Branch: mk(
    <path d="M5 3v7a2 2 0 002 2h4M5 3a1.5 1.5 0 100 3 1.5 1.5 0 000-3zm6 8a1.5 1.5 0 100 3 1.5 1.5 0 000-3zm0-8a1.5 1.5 0 100 3 1.5 1.5 0 000-3zm0 0v5" />,
  ),

  /** Check / done */
  Check: mk(<path d="M3 8l3.5 3.5L13 5" />),

  /** Circle (todo) */
  Circle: mk(<circle cx="8" cy="8" r="5.5" />),

  /** Spinner-dot (blocked) */
  Blocked: mk(
    <><circle cx="8" cy="8" r="5.5" /><path d="M6 6l4 4M10 6l-4 4" /></>,
  ),

  /** Layout / right panel toggle */
  PanelRight: mk(
    <><rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M10 3v10" /></>,
  ),

  /** Sidebar / left panel */
  PanelLeft: mk(
    <><rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M6 3v10" /></>,
  ),

  /** Plan / list */
  Plan: mk(
    <><path d="M4 5h8M4 8h6M4 11h4" /></>,
  ),

  /** Drive / folder */
  Drive: mk(
    <path d="M2 5.5A1.5 1.5 0 013.5 4H7l1.5 2H12.5A1.5 1.5 0 0114 7.5v4A1.5 1.5 0 0112.5 13h-9A1.5 1.5 0 012 11.5v-6z" />,
  ),

  /** Prompts / lightning */
  Prompts: mk(
    <path d="M10 2L5 9h4.5L6 14l7-8H8.5L10 2z" />,
  ),

  /** Ponytail — tied loop and flowing tail */
  Ponytail: mk(
    <><circle cx="6" cy="6" r="2.5" /><path d="M8 6c3 0 4.5 1.8 4.5 4 0 1.7-.8 3-2 4-.1-2.4-1-4.2-3-5.2" /></>,
  ),

  /** Settings / gear */
  Settings: mk(
    <><circle cx="8" cy="8" r="2.5" /><path d="M8 2v1.5m0 9V14M2 8h1.5m9 0H14M3.93 3.93l1.06 1.06m6.02 6.02 1.06 1.06M3.93 12.07l1.06-1.06m6.02-6.02 1.06-1.06" /></>,
  ),

  /** User / avatar */
  User: mk(
    <><circle cx="8" cy="6" r="2.5" /><path d="M3 13.5c0-2.485 2.239-4.5 5-4.5s5 2.015 5 4.5" /></>,
  ),

  /** Copy */
  Copy: mk(
    <><rect x="5.5" y="5.5" width="7" height="8" rx="1" /><path d="M3 10.5V4a1 1 0 011-1h6" /></>,
  ),

  /** File */
  File: mk(
    <><path d="M4 2h6l3 3v9a1 1 0 01-1 1H4a1 1 0 01-1-1V3a1 1 0 011-1z" /><path d="M10 2v3h3" /></>,
  ),

  /** Folder */
  Folder: mk(
    <path d="M2 5.5A1.5 1.5 0 013.5 4H7l1.5 2H12.5A1.5 1.5 0 0114 7.5v4A1.5 1.5 0 0112.5 13h-9A1.5 1.5 0 012 11.5v-6z" />,
  ),

  /** FolderOpen */
  FolderOpen: mk(
    <path d="M2 5.5A1.5 1.5 0 013.5 4H7l1.5 2H12.5A1.5 1.5 0 0114 7.5v.5H2v-2.5zM2 8h12l-1.5 5H3.5L2 8z" />,
  ),

  /** Dot (streaming indicator, dirty file) */
  Dot: mkFill(<circle cx="8" cy="8" r="4" />),

  /** Ellipsis */
  Ellipsis: mkFill(
    <><circle cx="4" cy="8" r="1.5" /><circle cx="8" cy="8" r="1.5" /><circle cx="12" cy="8" r="1.5" /></>,
  ),

  /** Chat bubble */
  Chat: mk(
    <><path d="M2 4a1 1 0 011-1h10a1 1 0 011 1v7a1 1 0 01-1 1H5.5L2 14V4z" /></>,
  ),

  /** Image */
  Image: mk(
    <><rect x="2" y="3" width="12" height="10" rx="1.5" /><circle cx="5.5" cy="7" r="1.5" /><path d="M14 10.5l-3.5-3.5L7 11l-2-2-3 3" /></>,
  ),

  /** External link (open in new tab) */
  ExternalLink: mk(
    <><path d="M10 2h4v4" /><path d="M14 2L7 9" /><path d="M12 9v3.5a1.5 1.5 0 01-1.5 1.5h-7A1.5 1.5 0 012 12.5v-7A1.5 1.5 0 013.5 4H7" /></>,
  ),

  /** History / clock */
  History: mk(
    <><circle cx="8" cy="8" r="6" /><path d="M8 5v3.5l2.5 1.5" /></>,
  ),

  /** Server / runner */
  Server: mk(
    <><rect x="3" y="2" width="10" height="4" rx="1" /><rect x="3" y="10" width="10" height="4" rx="1" /><path d="M8 6v4" /><circle cx="5.5" cy="4" r="0.75" fill="currentColor" stroke="none" /><circle cx="5.5" cy="12" r="0.75" fill="currentColor" stroke="none" /></>,
  ),

  /** Refresh / reload */
  Refresh: mk(
    <><path d="M3 8a5 5 0 019-3h2" /><path d="M14 2v3h-3" /><path d="M13 8a5 5 0 01-9 3H2" /><path d="M2 14v-3h3" /></>,
  ),

  /** Pull from remote */
  Download: mk(<><path d="M8 2v9" /><path d="M4.5 7.5L8 11l3.5-3.5" /><path d="M3 14h10" /></>),

  /** Push to remote */
  Upload: mk(<><path d="M8 14V5" /><path d="M4.5 8.5L8 5l3.5 3.5" /><path d="M3 2h10" /></>),

  /** Side-by-side change */
  Diff: mk(<><path d="M5 2v12" /><path d="M11 2v12" /><path d="M2 5l3-3 3 3" /><path d="M8 11l3 3 3-3" /></>),

  /** Trash / delete */
  Trash: mk(
    <><path d="M3 4h10" /><path d="M6 4V3a1 1 0 011-1h2a1 1 0 011 1v1" /><path d="M4 4l.7 9.1a1 1 0 001 .9h4.6a1 1 0 001-.9L12 4" /></>,
  ),

  /** Key / token */
  Key: mk(
    <><circle cx="10.5" cy="5.5" r="3" /><path d="M8 8L3 13" /><path d="M3 13l2-1 1 2" /><path d="M5.5 10.5l1.5-.5" /></>,
  ),

  /** Deploy / rocket */
  Deploy: mk(
    <><path d="M9 3c3 0 5 3 4 7l-3-1-2 3-2-3-3 1c-1-4 3-7 6-7z" /><circle cx="9.5" cy="7" r="1" /></>,
  ),

  /** Shield / admin */
  Shield: mk(
    <path d="M8 2L3 4.5v3c0 3.5 2.1 6.8 5 8 2.9-1.2 5-4.5 5-8v-3L8 2z" />,
  ),

  /** Clock / schedule */
  Clock: mk(
    <><circle cx="8" cy="8" r="5.5" /><path d="M8 5.5V8l2.5 1.5" /></>,
  ),
};
