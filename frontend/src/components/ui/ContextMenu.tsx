import { useEffect, useRef, useState, useCallback } from 'react';
import { createPortal } from 'react-dom';

export interface ContextMenuItem {
  label: string;
  icon?: string;
  danger?: boolean;
  disabled?: boolean;
  separator?: false;
  onClick: () => void;
}

export interface ContextMenuSeparator {
  separator: true;
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuSeparator;

interface MenuState {
  x: number;
  y: number;
  items: ContextMenuEntry[];
}

// Singleton state so only one menu is open at a time
let _setMenu: ((m: MenuState | null) => void) | null = null;

export function showContextMenu(x: number, y: number, items: ContextMenuEntry[]) {
  _setMenu?.({ x, y, items });
}

/** Mount once near the root — renders the active context menu */
export function ContextMenuPortal() {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    _setMenu = setMenu;
    return () => { _setMenu = null; };
  }, []);

  // Close on outside click, scroll, or Escape
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') close(); };
    // small delay so the triggering mousedown doesn't immediately close it
    const tid = setTimeout(() => {
      document.addEventListener('mousedown', close);
      document.addEventListener('scroll', close, true);
      document.addEventListener('keydown', onKey);
    }, 0);
    return () => {
      clearTimeout(tid);
      document.removeEventListener('mousedown', close);
      document.removeEventListener('scroll', close, true);
      document.removeEventListener('keydown', onKey);
    };
  }, [menu]);

  // Adjust position to stay within viewport
  const [pos, setPos] = useState({ x: 0, y: 0 });
  useEffect(() => {
    if (!menu || !ref.current) return;
    const { innerWidth: vw, innerHeight: vh } = window;
    const { offsetWidth: w, offsetHeight: h } = ref.current;
    setPos({
      x: menu.x + w > vw ? Math.max(0, vw - w - 8) : menu.x,
      y: menu.y + h > vh ? Math.max(0, vh - h - 8) : menu.y,
    });
  }, [menu]);

  if (!menu) return null;

  return createPortal(
    <div
      ref={ref}
      onMouseDown={e => e.stopPropagation()}
      style={{
        position: 'fixed',
        top: pos.y,
        left: pos.x,
        zIndex: 9999,
        background: 'var(--surface)',
        border: '1px solid var(--border)',
        borderRadius: 8,
        boxShadow: '0 8px 24px rgba(0,0,0,0.35)',
        minWidth: 180,
        padding: '4px 0',
        userSelect: 'none',
      }}
    >
      {menu.items.map((entry, i) => {
        if ('separator' in entry && entry.separator) {
          return (
            <div
              key={i}
              style={{ height: 1, background: 'var(--border)', margin: '4px 0' }}
            />
          );
        }
        const item = entry as ContextMenuItem;
        return (
          <button
            key={i}
            disabled={item.disabled}
            onClick={() => {
              setMenu(null);
              item.onClick();
            }}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              width: '100%',
              padding: '6px 14px',
              background: 'transparent',
              border: 'none',
              color: item.danger ? 'var(--error)' : item.disabled ? 'var(--text-muted)' : 'var(--text)',
              fontSize: 13,
              textAlign: 'left',
              cursor: item.disabled ? 'default' : 'pointer',
              opacity: item.disabled ? 0.5 : 1,
            }}
            onMouseEnter={e => {
              if (!item.disabled)
                (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.06)';
            }}
            onMouseLeave={e => {
              (e.currentTarget as HTMLElement).style.background = 'transparent';
            }}
          >
            {item.icon && (
              <span style={{ width: 16, textAlign: 'center', fontSize: 12, opacity: 0.8 }}>
                {item.icon}
              </span>
            )}
            {item.label}
          </button>
        );
      })}
    </div>,
    document.body
  );
}

/** Hook: returns an onContextMenu handler that shows a custom menu */
export function useContextMenu(items: () => ContextMenuEntry[]) {
  return useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    showContextMenu(e.clientX, e.clientY, items());
  }, [items]);
}
