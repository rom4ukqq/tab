import React, { useEffect, useState } from 'react';
import { artworkStyle, type Background } from './BackgroundPicker';

// Deterministic decorative text: no timers, network or per-frame React renders.
const glyphs = '01.:+*#%X@░';
const columns = Array.from({ length: 32 }, (_, column) => Array.from({ length: 42 }, (_, row) =>
  glyphs[(column * 17 + row * 13 + row * row) % glyphs.length]).join('\n'));

export function MatrixBackdrop({ animate, theme }: { animate: boolean; theme: Background }) {
  const [visible, setVisible] = useState(!document.hidden);
  useEffect(() => {
    const update = () => setVisible(!document.hidden);
    document.addEventListener('visibilitychange', update);
    return () => document.removeEventListener('visibilitychange', update);
  }, []);
  if (theme === 'none') return null;
  return <div className={`matrix-backdrop${animate && visible ? '' : ' matrix-paused'}`} aria-hidden="true">
    <div className="matrix-grid"/>
    {theme === 'matrix' ? <div className="matrix-columns">{columns.map((text, index) => <span key={index} style={{
      left: `${index * 3.2}%`, top: `${-60 + (index * 19 % 75)}%`,
      animationDuration: `${28 + index % 9 * 4}s`, animationDelay: `${-index * 3}s`,
      opacity: .12 + (index % 4) * .055,
    }}>{text}</span>)}</div> : <div className="ascii-scene" style={artworkStyle(theme)}/>}
    <div className="matrix-haze"/>
  </div>;
}
