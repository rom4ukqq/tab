import React, { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';

export function WindowChrome({ onError }: { onError: (error: string) => void }) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const refresh = () => win.isMaximized().then(value => { if (!disposed) setMaximized(value); }).catch(() => {});
    void refresh();
    win.onResized(refresh).then(fn => { if (disposed) fn(); else unlisten = fn; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  const action = (fn: () => Promise<void>) => { void fn().catch(e => onError(String(e))); };
  return <div className="window-chrome">
    <div className="window-drag" onMouseDown={e => {
      if (e.button !== 0) return;
      if (e.detail === 2) action(() => getCurrentWindow().toggleMaximize());
      else action(() => getCurrentWindow().startDragging());
    }}>
      <span className="window-symbol" aria-hidden="true">⠿</span>
      <span>Agent Hub</span><span className="window-caption">LOCAL AGENT BRIDGE</span>
    </div>
    <div className="window-controls">
      <button aria-label="Свернуть" title="Свернуть" onClick={() => action(() => getCurrentWindow().minimize())}><svg viewBox="0 0 16 16"><path d="M3 8h10"/></svg></button>
      <button aria-label={maximized ? 'Восстановить окно' : 'Развернуть'} title={maximized ? 'Восстановить окно' : 'Развернуть'} onClick={() => action(() => getCurrentWindow().toggleMaximize())}><svg viewBox="0 0 16 16">{maximized ? <><path d="M6 3h7v7M3 6h7v7H3Z"/></> : <path d="M3.5 3.5h9v9h-9Z"/>}</svg></button>
      <button className="window-close" aria-label="Закрыть в трей" title="Закрыть в трей" onClick={() => action(() => getCurrentWindow().close())}><svg viewBox="0 0 16 16"><path d="m4 4 8 8m0-8-8 8"/></svg></button>
    </div>
  </div>;
}

// Deterministic decorative text: no timers, network or per-frame React renders.
const glyphs = '01.:+*#%X@░';
const columns = Array.from({ length: 32 }, (_, column) => Array.from({ length: 42 }, (_, row) =>
  glyphs[(column * 17 + row * 13 + row * row) % glyphs.length]).join('\n'));

export function MatrixBackdrop({ animate }: { animate: boolean }) {
  const [visible, setVisible] = useState(!document.hidden);
  useEffect(() => {
    const update = () => setVisible(!document.hidden);
    document.addEventListener('visibilitychange', update);
    return () => document.removeEventListener('visibilitychange', update);
  }, []);
  return <div className={`matrix-backdrop${animate && visible ? '' : ' matrix-paused'}`} aria-hidden="true">
    <div className="matrix-grid"/>
    <div className="matrix-columns">{columns.map((text, index) => <span key={index} style={{
      left: `${index * 3.2}%`, top: `${-60 + (index * 19 % 75)}%`,
      animationDuration: `${28 + index % 9 * 4}s`, animationDelay: `${-index * 3}s`,
      opacity: .12 + (index % 4) * .055,
    }}>{text}</span>)}</div>
    <div className="matrix-haze"/>
  </div>;
}
